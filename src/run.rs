use std::{
    borrow::{Borrow, BorrowMut},
    cell::RefCell,
    collections::HashMap,
    error::Error,
    io::Cursor,
    ops::Deref,
    path::Path,
    rc::{Rc, Weak},
    vec,
};

use byteorder::{BigEndian, ReadBytesExt};

use crate::{
    deserialize::deserialize_class_file,
    parse::{
        parse, parse_field_descriptor, parse_method_descriptor, Attribute, Class as ParsedClass,
        ClassInfo, Constant, Field, FieldType, Method, MethodDescriptor,
    },
};

#[derive(Debug)]
struct ThreadMemory {
    jvm_stack: Vec<Frame>,
}

#[derive(Debug)]
struct Frame {
    local_variables: Vec<u32>,
    operand_stack: Vec<u32>,
    constant_pool: Weak<RuntimeConstantPool>,
    code_bytes: Option<Vec<u8>>,
    instruction_counter: usize,
    class_name: String,
    method: Method,
}

impl Frame {
    fn new(
        global_memory: &mut GlobalMemory,
        class_name: String,
        method_name: String,
        type_descriptor: MethodDescriptor,
    ) -> Result<Frame, Box<dyn Error>> {
        let mut class_name = class_name;
        let mut current_class = None;
        let mut current_method = None;
        // attempt to resolve methods - we should probably somehow precompute this?
        while current_method.is_none() {
            current_class = Some(
                global_memory
                    .method_area
                    .classes
                    .get(&class_name)
                    .ok_or(format!("Class not found {} :(", class_name))?,
            );
            let parsed_class = current_class
                .unwrap()
                .parsed_class
                .as_ref()
                .ok_or("no parsed_class")?;
            current_method = parsed_class
                .methods
                .iter()
                .filter(|method| method.name == method_name && method.descriptor == type_descriptor)
                .next();

            if current_method.is_none() {
                class_name = parsed_class
                    .as_ref()
                    .super_class
                    .as_ref()
                    .unwrap()
                    .name
                    .to_owned();
            }
        }

        let current_method = current_method.unwrap();
        let mut code_bytes = None;
        if !current_method.access.native {
            let code = current_method
                .attributes
                .iter()
                .filter(|attr| matches!(attr, Attribute::Code { .. }))
                .next()
                .ok_or("no code :(")?;
            println!("current_method: {current_method:?}");

            code_bytes = Some(code.as_code().ok_or("no code :(")?.to_owned());
        }

        let current_frame = Frame {
            constant_pool: Rc::downgrade(
                &current_class
                    .unwrap()
                    .constant_pool
                    .to_owned()
                    .ok_or("couldnt find stuff")?,
            ),
            local_variables: vec![0; 20],
            operand_stack: vec![],
            code_bytes,
            instruction_counter: 0,
            class_name: class_name.to_owned(),
            method: current_method.to_owned(),
        };
        println!(
            "new frame for method {}.{}({:?}): {:?}",
            class_name, method_name, type_descriptor, current_frame
        );
        return Ok(current_frame);
    }
}

#[derive(Debug)]
struct GlobalMemory {
    heap: Heap,
    method_area: MethodArea,
}

impl GlobalMemory {
    fn load_class(&mut self, name: String) -> Result<(), Box<dyn Error>> {
        if self.method_area.classes.contains_key(&name) {
            return Ok(());
        }
        // Look for class in (hardcoded) classpath
        let class_path = vec![
            ".",
            "../../openjdk/jdk11u/build/linux-x86_64-normal-server-release/jdk/modules/java.base",
        ];

        let mut path = None;
        for directory in class_path.iter() {
            let potential_path = Path::new(directory).join(name.to_owned() + ".class");
            if potential_path.exists() {
                path = Some(potential_path)
            }
        }
        let spath = path
            .ok_or("file not found")?
            .to_str()
            .ok_or("not a path")?
            .to_string();
        println!("spath: {spath}");

        let deserialized = deserialize_class_file(spath)?;

        let class = parse(deserialized)?;

        if let Some(ref class) = class.super_class {
            println!("found super class {class:?}, loading it!");
            self.load_class(class.name.to_owned())?;
        }

        // TODO: load interfaces
        let name = class.this_class.name.to_owned();
        let rc_class = Rc::new(class);
        self.method_area.add_class(
            name.to_owned(),
            Klass {
                name,
                parsed_class: Some(rc_class.clone()),
                fields: None,
                static_fields: None,
                constant_pool: None,
                static_field_values: Some(vec![]),
                java_clone: None,
                initialized: false,
            },
        );
        return Ok(());
    }

    fn link_class(&mut self, class_name: String) -> Result<(), Box<dyn Error>> {
        println!("linking class {class_name}");
        let klass = self
            .method_area
            .classes
            .get(&class_name)
            .ok_or("class not found")?;

        if klass.is_linked() {
            println!("Class {class_name} already linked, skip linking :^)");
            return Ok(());
        }

        let class = klass
            .parsed_class
            .as_ref()
            .ok_or("class exists, but we didnt parse it yet")?
            .to_owned();

        if class.super_class.is_some() {
            self.link_class(class.super_class.as_ref().unwrap().name.to_owned())?;
        }

        let mut pool = vec![];
        for item in class.constant_pool.iter() {
            pool.push(item.to_owned())
        }

        let klass = self
            .method_area
            .classes
            .get(&class_name)
            .ok_or("class not found")?;

        // preperation
        let mut field_values: Vec<u32> = vec![];
        for field in klass
            .static_fields
            .as_ref()
            .ok_or("no static fields")?
            .iter()
        {
            match field.field_type {
                crate::parse::FieldType::Integer => {
                    field_values.push(0);
                }
                crate::parse::FieldType::ClassInstance(_) => {
                    field_values.push(0);
                }
                crate::parse::FieldType::LongInteger => {
                    field_values.push(0);
                    field_values.push(0);
                }
                crate::parse::FieldType::Array(_) => {
                    field_values.push(0);
                }
                crate::parse::FieldType::Boolean => {
                    field_values.push(0);
                }
                crate::parse::FieldType::Byte => {
                    field_values.push(0);
                }
                crate::parse::FieldType::Char => {
                    field_values.push(0);
                }
                crate::parse::FieldType::Short => {
                    field_values.push(0);
                }
                crate::parse::FieldType::Float => {
                    field_values.push((0.0 as f32).to_bits());
                }
                crate::parse::FieldType::Double => {
                    let bits = (0.0 as f64).to_bits();
                    let mut csr = Cursor::new(bits.to_be_bytes());
                    field_values.push(csr.read_u32::<BigEndian>()?);
                    field_values.push(csr.read_u32::<BigEndian>()?);
                }
            }
        }

        // field layout of java/lang/Class
        let klass = self
            .method_area
            .classes
            .get(&"java/lang/Class".to_owned())
            .ok_or("class not found in method area 1 :(")?;
        let klass_java_clone = self.heap.allocate_class(klass);

        let klass = self
            .method_area
            .classes
            .get_mut(&class.this_class.name.to_owned());

        if let Some(klass) = klass {
            klass.constant_pool = Some(Rc::new(RuntimeConstantPool { pool }));
            klass
                .static_field_values
                .as_mut()
                .unwrap()
                .append(&mut field_values);
            klass.java_clone = Some(klass_java_clone);
        } else {
            return Err("what?".into());
        }

        Ok(())
    }

    fn init_class(&mut self, class_name: String) -> Result<(), Box<dyn Error>> {
        println!("init class {class_name}");

        let class = self
            .method_area
            .classes
            .get_mut(&class_name.to_owned())
            .ok_or("class not found")?;

        if class.is_initialized() {
            println!("Class {class_name} already linked, skip init :^)");
            return Ok(());
        }
        class.initialized = true;

        if let Some(_) = class
            .parsed_class
            .as_ref()
            .unwrap()
            .deref()
            .methods
            .iter()
            .find(|m| m.name == "<clinit>")
        {
            println!("found clinit method for class");
            let current_frame = Frame::new(
                self,
                class_name.to_owned(),
                "<clinit>".into(),
                MethodDescriptor {
                    parameter_descriptors: vec![],
                    return_descriptor: crate::parse::ReturnDescriptor::VoidDescriptor,
                },
            )?;
            let mut init_thread = Thread {
                thread_memory: ThreadMemory { jvm_stack: vec![] },
            };
            init_thread.thread_memory.jvm_stack.push(current_frame);
            init_thread.run(self)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct Heap {
    data: Vec<HeapItem>,
}

impl Heap {
    fn new() -> Heap {
        let mut h = Heap { data: vec![] };
        h.store("null".to_owned(), vec![]);
        return h;
    }

    fn store(&mut self, field_ref: String, data: Vec<u32>) -> u32 {
        self.data.push(HeapItem {
            field_descriptor: field_ref,
            data,
        });
        return (self.data.len() - 1) as u32;
    }

    fn allocate_class(&mut self, klass: &Klass) -> u32 {
        let mut field_values = vec![];
        for field in klass.fields.to_owned().unwrap().iter() {
            for _ in 0..field.field_width {
                field_values.push(0);
            }
        }

        return self.store(format!("L{};", klass.name), field_values);
    }
}

#[derive(Debug)]
struct HeapItem {
    // header
    field_descriptor: String,
    // data
    data: Vec<u32>,
}

#[derive(Debug)]
struct MethodArea {
    classes: HashMap<String, Klass>,
}

impl MethodArea {
    fn add_class(&mut self, class_name: String, mut class: Klass) {
        let parsed_class = &**class.parsed_class.as_ref().unwrap();

        let mut fields = vec![];
        let mut static_fields = vec![];

        for field in parsed_class.fields.iter() {
            match field.descriptor.field_type {
                crate::parse::FieldType::Integer
                | crate::parse::FieldType::Boolean
                | crate::parse::FieldType::Array(_)
                | crate::parse::FieldType::Byte
                | crate::parse::FieldType::Char
                | crate::parse::FieldType::Short
                | crate::parse::FieldType::Float
                | crate::parse::FieldType::ClassInstance(_) => {
                    let klass_field = KlassField {
                        class_name: class_name.to_owned(),
                        field_name: field.name.to_owned(),
                        field_type: field.descriptor.field_type.to_owned(),
                        field_width: 1,
                        _parsed_field: field.to_owned(),
                    };
                    if field.access.r#static {
                        static_fields.push(klass_field);
                    } else {
                        fields.push(klass_field)
                    }
                }
                crate::parse::FieldType::LongInteger | crate::parse::FieldType::Double => {
                    let klass_field = KlassField {
                        class_name: class_name.to_owned(),
                        field_name: field.name.to_owned(),
                        field_type: field.descriptor.field_type.to_owned(),
                        field_width: 2,
                        _parsed_field: field.to_owned(),
                    };
                    if field.access.r#static {
                        static_fields.push(klass_field);
                    } else {
                        fields.push(klass_field)
                    }
                }
            }
        }

        if let Some(class_info) = &parsed_class.super_class {
            let mut parent_fields = self
                .classes
                .get(&class_info.name.to_owned())
                .unwrap()
                .fields
                .clone()
                .unwrap();
            parent_fields.append(&mut fields);
            fields = parent_fields;
        };

        class.fields = Some(fields);
        class.static_fields = Some(static_fields);

        self.classes.insert(class_name, class);
    }
}

#[derive(Debug, Clone)]
struct KlassField {
    class_name: String,
    field_name: String,
    field_type: FieldType,
    field_width: usize,
    _parsed_field: Field,
}

#[derive(Debug)]
struct Klass {
    name: String,
    parsed_class: Option<Rc<ParsedClass>>,
    constant_pool: Option<Rc<RuntimeConstantPool>>,
    static_field_values: Option<Vec<u32>>,
    fields: Option<Vec<KlassField>>,
    static_fields: Option<Vec<KlassField>>,
    java_clone: Option<u32>,
    initialized: bool,
}

impl Klass {
    fn is_linked(&self) -> bool {
        self.constant_pool.is_some()
    }
    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn static_field_offset_with_strings(
        &self,
        searched_class_name: String,
        searched_field_name: String,
    ) -> Result<usize, Box<dyn Error>> {
        let mut offset = 0;

        for KlassField {
            class_name,
            field_name,
            field_width,
            ..
        } in self.static_fields.as_ref().unwrap().iter()
        {
            if searched_class_name != class_name.to_owned() {
                offset += field_width;
                continue;
            }
            if searched_field_name != field_name.to_owned() {
                offset += field_width;
                continue;
            }

            return Ok(offset);
        }
        // FIXME: 0 is not a error case :^)
        Err(format!("couldnt calculate static field offset for  \"{searched_class_name}\"\"{searched_field_name}\" because field was not found").into())
    }
    fn static_field_offset(&self, field_ref_constant: Constant) -> Result<usize, Box<dyn Error>> {
        println!("field_ref_constant {field_ref_constant:?}");
        let field_ref = field_ref_constant.as_field_ref().unwrap();
        let searched_class_name = field_ref.0.name;
        let searched_field_name = field_ref
            .1
            .to_owned()
            .as_name_and_type()
            .ok_or("not a name_and_type")?
            .0;
        self.static_field_offset_with_strings(searched_class_name, searched_field_name)
    }
    fn field_offset_with_strings(
        &self,
        searched_class_name: String,
        searched_field_name: String,
    ) -> Result<usize, Box<dyn Error>> {
        let mut offset = 0;

        for KlassField {
            class_name,
            field_name,
            field_width,
            ..
        } in self.fields.as_ref().unwrap().iter()
        {
            // if searched_class_name != class_name.to_owned() {
            //     offset += field_width;
            //     continue;
            // }
            if searched_field_name != field_name.to_owned() {
                offset += field_width;
                continue;
            }

            return Ok(offset);
        }
        // FIXME: 0 is not a error case :^)
        Err(format!("couldnt calculate field offset for  \"{searched_class_name}\"\"{searched_field_name}\" because field was not found: {:?}", self.fields).into())
    }
    fn field_offset(&self, field_ref_constant: Constant) -> Result<usize, Box<dyn Error>> {
        println!("field_ref_constant {field_ref_constant:?}");
        let field_ref = field_ref_constant.as_field_ref().unwrap();
        let searched_class_name = field_ref.0.name;
        let searched_field_name = field_ref
            .1
            .to_owned()
            .as_name_and_type()
            .ok_or("not a name_and_type")?
            .0;
        self.field_offset_with_strings(searched_class_name, searched_field_name)
    }
}

#[derive(Debug)]
struct RuntimeConstantPool {
    pool: Vec<Constant>,
}

#[derive(Debug)]
struct Thread {
    thread_memory: ThreadMemory,
}

// FIXME: do proper binding!
fn run_native_functions(
    thread: &mut Thread,
    global_memory: &mut GlobalMemory,
) -> Result<(), Box<dyn Error>> {
    let current_frame = thread
        .thread_memory
        .jvm_stack
        .last_mut()
        .ok_or("no item on jvm stack")?;

    match current_frame.class_name.as_str() {
        "java/lang/Object" => match current_frame.method.name.as_str() {
            "getClass" => {
                let this_ref = current_frame
                    .local_variables
                    .first()
                    .ok_or("no item in local_variables")?;
                // FIXME: check if this_ref is null
                let heap_item = global_memory
                    .heap
                    .data
                    .get(this_ref.to_owned() as usize)
                    .ok_or("this_ref not found on heap")?;
                let descriptor = parse_field_descriptor(&heap_item.field_descriptor)?;
                println!("descriptor: {descriptor:?}");

                let class_name = descriptor
                    .field_type
                    .as_class_instance()
                    .ok_or("not a class descriptor")?;
                let klass_java_clone = global_memory
                    .method_area
                    .classes
                    .get(&class_name.to_owned())
                    .ok_or("no class")?
                    .java_clone
                    .unwrap();

                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                frame.operand_stack.push(klass_java_clone);
            }
            "registerNatives" => {
                // noop for now?
            }
            "hashCode" => {
                let this_ref = current_frame
                    .local_variables
                    .first()
                    .ok_or("no item in local_variables")?
                    .to_owned();
                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                if this_ref == 0 {
                    frame.operand_stack.push(0);
                } else {
                    // FIXME: proper hash
                    frame.operand_stack.push(this_ref);
                }
            }
            method @ _ => {
                unimplemented!("{method} has no native impl")
            }
        },
        "java/lang/Class" => match current_frame.method.name.as_str() {
            "registerNatives" => {
                // noop for now?
            }
            "initClassName" => {
                let this_ref = current_frame
                    .local_variables
                    .first()
                    .ok_or("no item in local_variables")?;
                // FIXME: check if this_ref is null
                let heap_item = global_memory
                    .heap
                    .data
                    .get(this_ref.to_owned() as usize)
                    .ok_or("this_ref not found on heap")?;
                let descriptor = parse_field_descriptor(&heap_item.field_descriptor)?;
                println!("descriptor: {descriptor:?}");

                let class_name = descriptor
                    .field_type
                    .as_class_instance()
                    .ok_or("not a class descriptor")?;

                let string_ref =
                    java_string_from_string(current_frame, global_memory, class_name.to_owned())?;
                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                frame.operand_stack.push(string_ref);
            }
            "desiredAssertionStatus0" => {
                // no idea what this method does!
                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                frame.operand_stack.push(1);
            }
            "getPrimitiveClass" => {
                let primitive_type_ref = current_frame
                    .local_variables
                    .first()
                    .ok_or("no item in local_variables")?;
                let heap_item = global_memory
                    .heap
                    .data
                    .get(primitive_type_ref.to_owned() as usize)
                    .ok_or("this_ref not found on heap")?;
                let bytes_offset = global_memory
                    .method_area
                    .classes
                    .get("java/lang/String")
                    .unwrap()
                    .field_offset_with_strings("java/lang/String".to_owned(), "value".to_owned())?;
                let bytes_ref = heap_item
                    .data
                    .get(bytes_offset)
                    .ok_or("no data at bytes_offset")?;

                let bytes_bytes = &global_memory
                    .heap
                    .data
                    .get(*bytes_ref as usize)
                    .as_ref()
                    .ok_or("no bytes for string")?
                    .data;

                let text = String::from_utf8(bytes_bytes.iter().map(|b32| *b32 as u8).collect())?;

                println!("text: {:?}", text.bytes());

                let java_clone_ref;
                // NOTE: for some reason match didn't work here?
                if text == "int" {
                    global_memory.load_class("java/lang/Integer".to_owned())?;
                    global_memory.link_class("java/lang/Integer".to_owned())?;
                    global_memory.init_class("java/lang/Integer".to_owned())?;

                    java_clone_ref = Some(
                        global_memory
                            .method_area
                            .classes
                            .get("java/lang/Integer")
                            .ok_or("class not found")?
                            .java_clone
                            .ok_or("no java clone")?,
                    );
                } else if text == "float" {
                    global_memory.load_class("java/lang/Float".to_owned())?;
                    global_memory.link_class("java/lang/Float".to_owned())?;
                    global_memory.init_class("java/lang/Float".to_owned())?;

                    java_clone_ref = Some(
                        global_memory
                            .method_area
                            .classes
                            .get("java/lang/Float")
                            .ok_or("class not found")?
                            .java_clone
                            .ok_or("no java clone")?,
                    );
                } else if text == "double" {
                    global_memory.load_class("java/lang/Double".to_owned())?;
                    global_memory.link_class("java/lang/Double".to_owned())?;
                    global_memory.init_class("java/lang/Double".to_owned())?;

                    java_clone_ref = Some(
                        global_memory
                            .method_area
                            .classes
                            .get("java/lang/Double")
                            .ok_or("class not found")?
                            .java_clone
                            .ok_or("no java clone")?,
                    );
                } else {
                    unimplemented!("{}", text)
                }
                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                frame
                    .operand_stack
                    .push(java_clone_ref.ok_or("no java_clone found")?);
            }
            method @ _ => {
                unimplemented!("{method} has no native impl");
            }
        },
        "java/lang/System" => match current_frame.method.name.as_str() {
            "registerNatives" => {
                // noop for now?
            }
            "arraycopy" => {
                let src_ref = current_frame
                    .local_variables
                    .get(0)
                    .ok_or("no item in local_variables")?;
                let src_pos = Cursor::new(
                    current_frame
                        .local_variables
                        .get(1)
                        .ok_or("no item in local_variables")?
                        .to_be_bytes(),
                )
                .read_i32::<BigEndian>()?;
                let dest_ref = current_frame
                    .local_variables
                    .get(2)
                    .ok_or("no item in local_variables")?;
                let dest_pos = Cursor::new(
                    current_frame
                        .local_variables
                        .get(3)
                        .ok_or("no item in local_variables")?
                        .to_be_bytes(),
                )
                .read_i32::<BigEndian>()?;
                let length = Cursor::new(
                    current_frame
                        .local_variables
                        .get(4)
                        .ok_or("no item in local_variables")?
                        .to_be_bytes(),
                )
                .read_i32::<BigEndian>()?;
                println!("{} {} {} ", src_pos, dest_pos, length);
                // FIXME: handle longs?
                // FIXME: check if is actually an array

                println!(
                    "{:?}",
                    global_memory
                        .method_area
                        .classes
                        .get("java/lang/String")
                        .as_ref()
                        .unwrap()
                        .static_field_values
                        .as_ref()
                        .unwrap()
                );

                let src_array_data = global_memory
                    .heap
                    .data
                    .get(*src_ref as usize)
                    .ok_or("array not on heap")?
                    .data
                    .to_owned();
                let target_array = global_memory
                    .heap
                    .data
                    .get_mut(*dest_ref as usize)
                    .ok_or("array not on heap")?;

                for i in 0..length {
                    target_array.data[(dest_pos + i) as usize] =
                        src_array_data[(src_pos + i) as usize];
                }
            }
            method @ _ => {
                unimplemented!("{method} has no native impl");
            }
        },
        "java/lang/StringUTF16" => match current_frame.method.name.as_str() {
            "isBigEndian" => {
                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                frame.operand_stack.push(1);
            }
            method @ _ => {
                unimplemented!("{method} has no native impl");
            }
        },
        "java/lang/Float" => match current_frame.method.name.as_str() {
            "floatToRawIntBits" => {
                let float_read_as_u32 = Cursor::new(
                    current_frame
                        .local_variables
                        .get(0)
                        .ok_or("no item in local_variables")?
                        .to_be_bytes(),
                )
                .read_u32::<BigEndian>()?;

                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                frame.operand_stack.push(float_read_as_u32);
            }
            method @ _ => {
                unimplemented!("{method} has no native impl");
            }
        },
        "java/lang/Double" => match current_frame.method.name.as_str() {
            "doubleToRawLongBits" => {
                let double_part1 = Cursor::new(
                    current_frame
                        .local_variables
                        .get(0)
                        .ok_or("no item in local_variables")?
                        .to_be_bytes(),
                )
                .read_u32::<BigEndian>()?;
                let double_part2 = Cursor::new(
                    current_frame
                        .local_variables
                        .get(1)
                        .ok_or("no item in local_variables")?
                        .to_be_bytes(),
                )
                .read_u32::<BigEndian>()?;

                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                frame.operand_stack.push(double_part1);
                frame.operand_stack.push(double_part2);
            }
            "longBitsToDouble" => {
                let long_part1 = Cursor::new(
                    current_frame
                        .local_variables
                        .get(0)
                        .ok_or("no item in local_variables")?
                        .to_be_bytes(),
                )
                .read_u32::<BigEndian>()?;
                let long_part2 = Cursor::new(
                    current_frame
                        .local_variables
                        .get(1)
                        .ok_or("no item in local_variables")?
                        .to_be_bytes(),
                )
                .read_u32::<BigEndian>()?;

                let invoker_frame_index = thread.thread_memory.jvm_stack.len() - 2;
                let frame = thread
                    .thread_memory
                    .jvm_stack
                    .get_mut(invoker_frame_index)
                    .ok_or("no invoker")?;

                frame.operand_stack.push(long_part1);
                frame.operand_stack.push(long_part2);
            }
            method @ _ => {
                unimplemented!("{method} has no native impl");
            }
        },
        _ => {
            unimplemented!("{} {}", current_frame.class_name, current_frame.method.name)
        }
    }

    Ok(())
}

fn java_string_from_string(
    current_frame: &mut Frame,
    global_memory: &mut GlobalMemory,
    string: String,
) -> Result<u32, Box<dyn Error>> {
    let klass = global_memory
        .method_area
        .classes
        .get("java/lang/String")
        .ok_or("class not found in method area 2 :(")?;

    let string_objectref = global_memory.heap.allocate_class(klass);

    let bytes = string.bytes().map(|b| b as u32).collect::<Vec<u32>>();

    let array_objectref = global_memory.heap.store("[B".to_owned(), bytes);

    let string_klass = *global_memory
        .method_area
        .classes
        .get("java/lang/String")
        .as_ref()
        .unwrap();

    let value_field_offset = string_klass
        .field_offset_with_strings("java/lang/String".to_owned(), "value".to_owned())?;
    let coder_field_offset = string_klass
        .field_offset_with_strings("java/lang/String".to_owned(), "coder".to_owned())?;

    global_memory
        .heap
        .data
        .get_mut(string_objectref.to_owned() as usize)
        .as_mut()
        .ok_or("no object at byte location")?
        .data[value_field_offset] = array_objectref;
    global_memory
        .heap
        .data
        .get_mut(string_objectref.to_owned() as usize)
        .as_mut()
        .ok_or("no object at byte location")?
        .data[coder_field_offset] = 1;

    return Ok(string_objectref);
}

impl Thread {
    fn run(&mut self, global_memory: &mut GlobalMemory) -> Result<(), Box<dyn Error>> {
        loop {
            let current_frame = self
                .thread_memory
                .jvm_stack
                .last_mut()
                .ok_or("no item on jvm stack")?;

            if current_frame.code_bytes.is_none() {
                run_native_functions(self, global_memory)?;

                self.thread_memory.jvm_stack.pop();

                continue;
            }

            let code_bytes = current_frame
                .code_bytes
                .as_ref()
                .ok_or("expected code bytes")?;
            let instruction = code_bytes
                .get(current_frame.instruction_counter)
                .ok_or("no instruction at instruction_counter")?;
            println!(
                "instruction: ptr {} {instruction:#0x} in {} {:?}, {:?} {:?}",
                current_frame.instruction_counter,
                current_frame.class_name,
                current_frame.method.name,
                current_frame.operand_stack,
                current_frame.local_variables
            );
            match instruction {
                // iconst_i
                0x1 => {
                    current_frame.operand_stack.push(0);
                    current_frame.instruction_counter += 1;
                }
                // iconst_i
                instruction @ (0x2 | 0x3 | 0x4 | 0x5 | 0x6 | 0x7 | 0x8) => {
                    let topush = *instruction as i32 - 0x3;
                    current_frame
                        .operand_stack
                        .push(Cursor::new(topush.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // bipush
                0x10 => {
                    current_frame.instruction_counter += 1;
                    let byte = code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;

                    // read this as i8
                    let as_i8 = Cursor::new(byte.to_be_bytes()).read_i8()?;
                    current_frame
                        .operand_stack
                        .push(Cursor::new((as_i8 as i32).to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // sipush
                0x11 => {
                    current_frame.instruction_counter += 1;
                    let byte1 = *code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")? as u16;
                    current_frame.instruction_counter += 1;
                    let byte2 = *code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")? as u16;

                    let mut sign_extended = Cursor::new((byte1 << 8 | byte2).to_be_bytes());
                    let value = sign_extended.read_i16::<BigEndian>()?;
                    current_frame
                        .operand_stack
                        .push(Cursor::new((value as i32).to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // ldc
                0x12 => {
                    current_frame.instruction_counter += 1;
                    let index = code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;
                    let loadable_constant = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();
                    match loadable_constant {
                        Constant::String(string) => {
                            let string_objectref =
                                java_string_from_string(current_frame, global_memory, string)?;
                            current_frame.operand_stack.push(string_objectref);
                        }
                        Constant::Integer(value) => {
                            let integer =
                                Cursor::new(value.to_be_bytes()).read_u32::<BigEndian>()?;
                            println!("{}", integer);
                            current_frame.operand_stack.push(integer);
                        }
                        Constant::Class(class_info) => {
                            let klass_java_clone = global_memory
                                .method_area
                                .classes
                                .get(&class_info.name.to_owned())
                                .ok_or("no class")?
                                .java_clone
                                .unwrap();
                            current_frame.operand_stack.push(klass_java_clone);
                        }
                        Constant::Float(value) => {
                            let float = Cursor::new(value.to_be_bytes()).read_u32::<BigEndian>()?;
                            println!("{}", float);
                            current_frame.operand_stack.push(float);
                        }
                        // FIXME: Some are not actually unreachable
                        _ => unreachable!("{:?}", loadable_constant),
                    }
                    current_frame.instruction_counter += 1;
                }
                // ldc2_w
                0x14 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let loadable_constant = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();

                    match loadable_constant {
                        Constant::Long(value) => {
                            let mut csr = Cursor::new(value.to_be_bytes());

                            let part1 = csr.read_u32::<BigEndian>()?;
                            let part2 = csr.read_u32::<BigEndian>()?;

                            current_frame.operand_stack.push(part1);
                            current_frame.operand_stack.push(part2);
                        }
                        // FIXME: Some are not actually unreachable
                        _ => unreachable!("{:?}", loadable_constant),
                    }
                    current_frame.instruction_counter += 1;
                }
                // iload | aload
                0x15 | 0x19 => {
                    current_frame.instruction_counter += 1;
                    let index = code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;
                    let integer = current_frame.local_variables[*index as usize];
                    current_frame.operand_stack.push(integer);
                    current_frame.instruction_counter += 1;
                }
                // iload_n
                instruction @ (0x1a | 0x1b | 0x1c | 0x1d) => {
                    let integer = current_frame.local_variables[(instruction - 0x1a) as usize];
                    current_frame.operand_stack.push(integer);

                    current_frame.instruction_counter += 1;
                }
                // aload_n
                instruction @ (0x2a | 0x2b | 0x2c | 0x2d) => {
                    let integer = current_frame.local_variables[(instruction - 0x2a) as usize];
                    current_frame.operand_stack.push(integer);

                    current_frame.instruction_counter += 1;
                }
                // baload
                0x33 => {
                    let index = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack 1")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()?;
                    println!("index: {index}");
                    let arrayref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack 2")?;

                    let value = global_memory
                        .heap
                        .data
                        .get_mut(arrayref as usize)
                        .ok_or("arrayref not on heap")?
                        .data
                        .get(index as usize)
                        .ok_or("arrays not that big")?;

                    current_frame.operand_stack.push(*value);

                    current_frame.instruction_counter += 1;
                }
                // caload
                0x34 => {
                    let index = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack 1")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()?;
                    println!("index: {index}");
                    let arrayref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack 2")?;

                    let value = global_memory
                        .heap
                        .data
                        .get_mut(arrayref as usize)
                        .ok_or("arrayref not on heap")?
                        .data
                        .get(index as usize)
                        .ok_or("arrays not that big")?;

                    current_frame.operand_stack.push(*value);

                    current_frame.instruction_counter += 1;
                }
                // istore | astore
                0x36 | 0x3a => {
                    current_frame.instruction_counter += 1;
                    let index = code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;
                    let integer = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    current_frame.local_variables[*index as usize] = integer;

                    current_frame.instruction_counter += 1;
                }
                // istore_n
                instruction @ (0x3b | 0x3c | 0x3d | 0x3e) => {
                    let integer = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    current_frame.local_variables[(instruction - 0x3b) as usize] = integer;

                    current_frame.instruction_counter += 1;
                }
                // astore_n
                instruction @ (0x4b | 0x4c | 0x4d | 0x4e) => {
                    let reference = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    current_frame.local_variables[(instruction - 0x4b) as usize] = reference;

                    current_frame.instruction_counter += 1;
                }
                // bastore, iastore
                0x4f | 0x54 => {
                    // value does not need to be unwrapped, as it will be stored as a java integer
                    // anyway
                    let value = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let index = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()?;
                    let arrayref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;

                    global_memory
                        .heap
                        .data
                        .get_mut(arrayref as usize)
                        .ok_or("arrayref not on heap")?
                        .data[index as usize] = value;

                    current_frame.instruction_counter += 1;
                }
                // castore
                0x55 => {
                    let value = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()?;
                    let index = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()?;
                    let arrayref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;

                    global_memory
                        .heap
                        .data
                        .get_mut(arrayref as usize)
                        .ok_or("arrayref not on heap")?
                        .data[index as usize] = value as u16 as u32;

                    current_frame.instruction_counter += 1;
                }
                // pop
                0x57 => {
                    current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    current_frame.instruction_counter += 1;
                }
                // dup
                0x59 => {
                    let top_stack_value = current_frame
                        .operand_stack
                        .last()
                        .ok_or("operand stack is empty, so duplication is not possible :(")?
                        .to_owned();
                    current_frame.operand_stack.push(top_stack_value);
                    current_frame.instruction_counter += 1;
                }
                // dup_x1
                0x5a => {
                    // not tested at all
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("nothing to duplicate")?;
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("nothing to duplicate")?;

                    current_frame.operand_stack.push(value1);
                    current_frame.operand_stack.push(value2);
                    current_frame.operand_stack.push(value1);
                }
                // iadd
                0x60 => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?
                        + Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // ladd
                0x61 => {
                    let value2_part2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value2_part1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value1_part2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value1_part1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;

                    let value1 = Cursor::new(((value1_part1 << 16) | value1_part2).to_be_bytes())
                        .read_i64::<BigEndian>()?;
                    let value2 = Cursor::new(((value2_part1 << 16) | value2_part2).to_be_bytes())
                        .read_i64::<BigEndian>()?;

                    let result = value1 + value2;
                    let mut csr = Cursor::new(result.to_be_bytes());
                    let result_part1 = csr.read_u32::<BigEndian>()?;
                    let result_part2 = csr.read_u32::<BigEndian>()?;

                    current_frame.operand_stack.push(result_part1);
                    current_frame.operand_stack.push(result_part2);
                    current_frame.instruction_counter += 1;
                }
                // isub
                0x64 => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?;
                    let value2 = Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    println!("{value1} {value2}");
                    let result = value1 - value2;
                    println!("result is {result}");
                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // imul
                0x68 => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    // FIXME: not handling overflow properly
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()? as i64
                        * Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()? as i64;
                    current_frame
                        .operand_stack
                        .push(Cursor::new((result as i32).to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // lmul
                0x69 => {
                    let value2_part2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value2_part1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value1_part2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value1_part1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;

                    let value1 = Cursor::new(((value1_part1 << 16) | value1_part2).to_be_bytes())
                        .read_i64::<BigEndian>()?;
                    let value2 = Cursor::new(((value2_part1 << 16) | value2_part2).to_be_bytes())
                        .read_i64::<BigEndian>()?;

                    let result = value1 * value2;
                    let mut csr = Cursor::new(result.to_be_bytes());
                    let result_part1 = csr.read_u32::<BigEndian>()?;
                    let result_part2 = csr.read_u32::<BigEndian>()?;

                    current_frame.operand_stack.push(result_part1);
                    current_frame.operand_stack.push(result_part2);
                    current_frame.instruction_counter += 1;
                }
                // idiv
                0x6c => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    // TODO: check if rounding is equals?
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?
                        / Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    println!("result is {result}");
                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // irem
                0x70 => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?
                        % Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    println!("result is {result}");
                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // ineg
                0x74 => {
                    let value = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let result = -Cursor::new(value.to_be_bytes()).read_i32::<BigEndian>()?;
                    println!("result is {result}");
                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }

                // ishl
                0x78 => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    println!("value2: {value2}");
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;

                    let result =
                        Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()? << value2;

                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);

                    current_frame.instruction_counter += 1;
                }
                // lshl
                0x79 => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1_part2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value1_part1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;

                    let value1 = Cursor::new(((value1_part1 << 16) | value1_part2).to_be_bytes())
                        .read_i64::<BigEndian>()?;
                    let value2 = Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;

                    let result = value1 << value2;

                    let mut csr = Cursor::new(result.to_be_bytes());
                    let result_part1 = csr.read_u32::<BigEndian>()?;
                    let result_part2 = csr.read_u32::<BigEndian>()?;

                    current_frame.operand_stack.push(result_part1);
                    current_frame.operand_stack.push(result_part2);
                    current_frame.instruction_counter += 1;
                }
                // ishr
                0x7a => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    println!("value2: {value2}");
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;

                    // Arithmetic! shift -> shift with sign bit preserved
                    // >> is arithmetic on signed integer types in rust
                    let result =
                        Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()? >> value2;

                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // iushr
                0x7c => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    // Logical! shift. Therefore, we dont read the value1 as i8, so we can just
                    // shift the bytes
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;

                    let v1 = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?;
                    let s = value2 & 0x1f;
                    println!("s {value1} {value2} {s}");
                    let result;

                    result = value1.wrapping_shr(value2 as u32);

                    current_frame.operand_stack.push(result);
                    current_frame.instruction_counter += 1;
                }
                // iand
                0x7e => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?
                        & Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // land
                0x7f => {
                    let value2_part2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value2_part1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value1_part2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;
                    let value1_part1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?
                        as u64;

                    let value1 = Cursor::new(((value1_part1 << 16) | value1_part2).to_be_bytes())
                        .read_i64::<BigEndian>()?;
                    let value2 = Cursor::new(((value2_part1 << 16) | value2_part2).to_be_bytes())
                        .read_i64::<BigEndian>()?;

                    let result = value1 & value2;
                    let mut csr = Cursor::new(result.to_be_bytes());
                    let result_part1 = csr.read_u32::<BigEndian>()?;
                    let result_part2 = csr.read_u32::<BigEndian>()?;

                    current_frame.operand_stack.push(result_part1);
                    current_frame.operand_stack.push(result_part2);
                    current_frame.instruction_counter += 1;
                }
                // ior
                0x80 => {
                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?
                        | Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // iinc
                0x84 => {
                    current_frame.instruction_counter += 1;
                    let index = *code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;
                    current_frame.instruction_counter += 1;
                    let the_const = Cursor::new(
                        (*code_bytes
                            .get(current_frame.instruction_counter)
                            .ok_or("no bytes")?)
                        .to_be_bytes(),
                    )
                    .read_i8()?;

                    let value = Cursor::new(
                        current_frame
                            .local_variables
                            .get(index as usize)
                            .ok_or("no variable in local storage index")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()?;
                    let new_value = value + the_const as i32;
                    println!("new_value: {new_value}");
                    current_frame.local_variables[index as usize] =
                        Cursor::new(new_value.to_be_bytes()).read_u32::<BigEndian>()?;
                    current_frame.instruction_counter += 1;
                }
                // i2l
                0x85 => {
                    let value = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()? as i64;

                    let mut csr = Cursor::new(value.to_be_bytes());

                    let part1 = csr.read_u32::<BigEndian>()?;
                    let part2 = csr.read_u32::<BigEndian>()?;

                    current_frame.operand_stack.push(part1);
                    current_frame.operand_stack.push(part2);

                    current_frame.instruction_counter += 1;
                }
                //i2b
                0x91 => {
                    let value = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()? as u8 as i32;
                    current_frame
                        .operand_stack
                        .push(Cursor::new(value.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                //i2c
                0x92 => {
                    let value = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()? as u16 as i32;
                    current_frame
                        .operand_stack
                        .push(Cursor::new(value.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // ifeq
                instruction @ (0x99 | 0x9a | 0x9b | 0x9c | 0x9d | 0x9e) => {
                    current_frame.instruction_counter += 1;
                    let branchbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let branchoffset =
                        Cursor::new(((branchbyte1 << 8) | branchbyte2).to_be_bytes())
                            .read_i16::<BigEndian>()?;

                    let value = Cursor::new(
                        current_frame
                            .operand_stack
                            .pop()
                            .ok_or("no item on the operand_stack")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()?;
                    let mut result = false;
                    if *instruction == 0x99 {
                        result = value == 0;
                    } else if *instruction == 0x9a {
                        result = value != 0;
                    } else if *instruction == 0x9b {
                        result = value < 0;
                    } else if *instruction == 0x9c {
                        result = value >= 0;
                    } else if *instruction == 0x9d {
                        result = value > 0;
                    } else if *instruction == 0x9e {
                        result = value <= 0;
                    }
                    if result {
                        current_frame.instruction_counter =
                            current_frame.instruction_counter - 2 + branchoffset as usize;
                    } else {
                        current_frame.instruction_counter += 1;
                    }
                }
                // if_icmp<cond>
                instruction @ (0x9f | 0xa0 | 0xa1 | 0xa2 | 0xa3 | 0xa4) => {
                    current_frame.instruction_counter += 1;
                    let branchbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let branchoffset =
                        Cursor::new(((branchbyte1 << 8) | branchbyte2).to_be_bytes())
                            .read_i16::<BigEndian>()?;

                    let value2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let v1 = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?;
                    let v2 = Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    println!("compare: {v1} {v2}");

                    let mut result = false;
                    if *instruction == 0x9f {
                        result = v1 == v2;
                    } else if *instruction == 0xa0 {
                        // ne
                        result = v1 != v2;
                    } else if *instruction == 0xa1 {
                        // lt
                        result = v1 < v2;
                    } else if *instruction == 0xa2 {
                        // ge
                        result = v1 >= v2;
                    } else if *instruction == 0xa3 {
                        // gt
                        result = v1 > v2;
                    } else if *instruction == 0xa4 {
                        // le
                        result = v1 <= v2;
                    }

                    if result {
                        current_frame.instruction_counter =
                            current_frame.instruction_counter - 2 + branchoffset as usize;
                    } else {
                        current_frame.instruction_counter += 1;
                    }
                }
                // goto
                0xa7 => {
                    current_frame.instruction_counter += 1;
                    let branchbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let branchoffset =
                        Cursor::new(((branchbyte1 << 8) | branchbyte2).to_be_bytes())
                            .read_i16::<BigEndian>()?;
                    println!("offset: {branchoffset}");
                    current_frame.instruction_counter =
                        ((current_frame.instruction_counter - 2) as isize + branchoffset as isize)
                            as usize;
                }
                // ireturn, areturn
                0xac | 0xb0 => {
                    let value = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no return value on operand stack")?;

                    let invoker_frame_index = self.thread_memory.jvm_stack.len() - 2;
                    let frame = self
                        .thread_memory
                        .jvm_stack
                        .get_mut(invoker_frame_index)
                        .ok_or("no invoker")?;

                    frame.operand_stack.push(value);
                    self.thread_memory.jvm_stack.pop();
                }
                0xaf => {
                    let value_part2 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value_part1 = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;

                    let invoker_frame_index = self.thread_memory.jvm_stack.len() - 2;
                    let frame = self
                        .thread_memory
                        .jvm_stack
                        .get_mut(invoker_frame_index)
                        .ok_or("no invoker")?;

                    frame.operand_stack.push(value_part1);
                    frame.operand_stack.push(value_part2);
                    self.thread_memory.jvm_stack.pop();
                }
                // return
                0xb1 => {
                    if self.thread_memory.jvm_stack.len() == 1 {
                        break;
                    }
                    self.thread_memory.jvm_stack.pop();
                }
                // getstatic
                0xb2 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let field_ref_constant = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();

                    let (class_info, name_and_type) = field_ref_constant
                        .as_field_ref()
                        .ok_or(format!("not a field_ref 1 {:?}", field_ref_constant))?;
                    let (name, field_descriptor_text) = name_and_type
                        .as_name_and_type()
                        .ok_or("not a NameAndType")?;

                    let type_descriptor = parse_field_descriptor(&field_descriptor_text)?;

                    global_memory.load_class(class_info.name.to_owned())?;
                    global_memory.link_class(class_info.name.to_owned())?;
                    global_memory.init_class(class_info.name.to_owned())?;

                    let class = global_memory.method_area.classes.get(&class_info.name);
                    let class = class.as_ref().unwrap().deref();

                    let static_field_offset = class.static_field_offset(field_ref_constant)?;

                    // TODO: handle longs :^)
                    let v = class
                        .static_field_values
                        .as_ref()
                        .unwrap()
                        .get(static_field_offset as usize)
                        .ok_or("no value in static_field_values")?;

                    current_frame.operand_stack.push(*v);

                    current_frame.instruction_counter += 1;
                }
                // putstatic
                0xb3 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    let value = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no popable value here")?;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let field_ref_constant = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();

                    let (class_info, name_and_type) = field_ref_constant
                        .as_field_ref()
                        .ok_or("not a field_ref 2")?;
                    let (name, field_descriptor_text) = name_and_type
                        .as_name_and_type()
                        .ok_or("not a NameAndType")?;

                    let type_descriptor = parse_field_descriptor(&field_descriptor_text)?;

                    global_memory.load_class(class_info.name.to_owned())?;
                    global_memory.link_class(class_info.name.to_owned())?;
                    global_memory.init_class(class_info.name.to_owned())?;

                    let mut class = global_memory.method_area.classes.get_mut(&class_info.name);
                    let class = class.as_mut().unwrap();

                    let static_field_offset = class.static_field_offset(field_ref_constant)?;

                    // TODO: handle longs :^)
                    class.static_field_values.as_mut().unwrap()[static_field_offset as usize] =
                        value;

                    current_frame.instruction_counter += 1;
                }
                // getfield indexbyte1 indexbyte2
                0xb4 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let constant = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();

                    let objectref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("value is not on the stack")?;

                    let field_ref = global_memory
                        .heap
                        .data
                        .get(objectref as usize)
                        .ok_or(format!("object {objectref} not found on heap!"))?
                        .field_descriptor
                        .to_owned();
                    let field_descriptor = parse_field_descriptor(&field_ref)?;
                    let class_name = field_descriptor
                        .field_type
                        .as_class_instance()
                        .ok_or("not a class instance")?;
                    println!("class_name: {class_name}");
                    let offset = global_memory
                        .method_area
                        .classes
                        .get(&class_name.to_owned())
                        .ok_or(format!("didnt find class {class_name} in method_area"))?
                        .field_offset(constant)?;

                    // fixme: handle longs?
                    let value = global_memory
                        .heap
                        .data
                        .get_mut(objectref as usize)
                        .ok_or("item not on heap")?
                        .data[offset];

                    current_frame.operand_stack.push(value);

                    current_frame.instruction_counter += 1;
                }
                // putfield indexbyte1 indexbyte2
                0xb5 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let constant = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();

                    let value = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("value is not on the stack")?;
                    let objectref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("value is not on the stack")?;

                    let field_ref = global_memory
                        .heap
                        .data
                        .get(objectref as usize)
                        .ok_or(format!("object {objectref} not found on heap!"))?
                        .field_descriptor
                        .to_owned();
                    let field_descriptor = parse_field_descriptor(&field_ref)?;
                    let class_name = field_descriptor
                        .field_type
                        .as_class_instance()
                        .ok_or("not a class instance")?;
                    let offset = global_memory
                        .method_area
                        .classes
                        .get(&class_name.to_owned())
                        .ok_or(format!("didnt find class {class_name} in method_area"))?
                        .field_offset(constant)?;

                    // FIXME: handle longs
                    global_memory
                        .heap
                        .data
                        .get_mut(objectref as usize)
                        .ok_or("item not on heap")?
                        .data[offset] = value;

                    current_frame.instruction_counter += 1;
                }
                // invokevirtual indexbyte1 indexbyte2
                0xb6 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let (class_info, name_and_type) = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned()
                        .as_method_ref()
                        .ok_or("not a field ref")?;
                    let (name, method_descriptor_text) = name_and_type
                        .as_name_and_type()
                        .ok_or("not a NameAndType")?;
                    let type_descriptor = parse_method_descriptor(method_descriptor_text)?;

                    global_memory.load_class(class_info.name.to_owned())?;
                    global_memory.link_class(class_info.name.to_owned())?;

                    println!("name {name} type_descriptor {type_descriptor:?}");

                    let mut nargs = vec![];
                    // this loop is probably incorrect, as doubles and stuff take up 2 bytes
                    for _ in 0..type_descriptor.parameter_descriptors.len() {
                        let narg = current_frame
                            .operand_stack
                            .pop()
                            .ok_or("nargs is not on the stack")?;
                        nargs.insert(0, narg);
                    }
                    let object_ref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("object_ref is not on the stack")?;

                    let mut new_frame =
                        Frame::new(global_memory, class_info.name, name, type_descriptor)?;
                    // FIXME: this probably doesnt handle longs correctly?
                    new_frame.local_variables[0] = object_ref;
                    for narg in nargs.iter().enumerate() {
                        new_frame.local_variables[narg.0 + 1] = *narg.1;
                    }

                    current_frame.instruction_counter += 1;

                    self.thread_memory.jvm_stack.push(new_frame);
                }
                // invokespecial
                0xb7 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let (class_info, name_and_type) = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned()
                        .as_method_ref()
                        .ok_or("not a field ref")?;
                    let (name, method_descriptor_text) = name_and_type
                        .as_name_and_type()
                        .ok_or("not a NameAndType")?;

                    global_memory.load_class(class_info.name.to_owned())?;
                    global_memory.link_class(class_info.name.to_owned())?;
                    global_memory.init_class(class_info.name.to_owned())?;
                    let type_descriptor = parse_method_descriptor(method_descriptor_text)?;

                    let mut nargs = vec![];
                    // this loop is probably incorrect, as doubles and stuff take up 2 bytes
                    for _ in 0..type_descriptor.parameter_descriptors.len() {
                        let narg = current_frame
                            .operand_stack
                            .pop()
                            .ok_or("nargs is not on the stack")?;
                        nargs.insert(0, narg);
                    }
                    let object_ref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("object_ref is not on the stack")?;

                    let mut new_frame =
                        Frame::new(global_memory, class_info.name, name, type_descriptor)?;
                    // FIXME: this probably doesnt handle longs correctly?
                    new_frame.local_variables[0] = object_ref;
                    for narg in nargs.iter().enumerate() {
                        new_frame.local_variables[narg.0 + 1] = *narg.1;
                    }

                    current_frame.instruction_counter += 1;

                    self.thread_memory.jvm_stack.push(new_frame);
                }
                // invokestatic
                0xb8 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let (class_info, name_and_type) = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned()
                        .as_method_ref()
                        .ok_or("not a field ref")?;
                    let (name, method_descriptor_text) = name_and_type
                        .as_name_and_type()
                        .ok_or("not a NameAndType")?;

                    global_memory.load_class(class_info.name.to_owned())?;
                    global_memory.link_class(class_info.name.to_owned())?;
                    global_memory.init_class(class_info.name.to_owned())?;

                    let type_descriptor = parse_method_descriptor(method_descriptor_text)?;
                    println!("type_descriptor: {type_descriptor:?}");
                    let mut nargs = vec![];

                    // this loop is probably incorrect, as doubles and stuff take up 2 bytes
                    for _ in 0..type_descriptor.parameter_descriptors.len() {
                        let narg = current_frame
                            .operand_stack
                            .pop()
                            .ok_or("nargs is not on the stack")?;
                        nargs.insert(0, narg);
                    }

                    let mut new_frame =
                        Frame::new(global_memory, class_info.name, name, type_descriptor)?;
                    // FIXME: this probably doesnt handle longs correctly?
                    for narg in nargs.iter().enumerate() {
                        new_frame.local_variables[narg.0] = *narg.1;
                    }
                    current_frame.instruction_counter += 1;

                    self.thread_memory.jvm_stack.push(new_frame)
                }
                // new
                0xbb => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let constant = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();
                    let class = constant.as_class().ok_or("ClassNotFound :(")?;

                    global_memory.load_class(class.name.to_owned())?;
                    global_memory.link_class(class.name.to_owned())?;
                    global_memory.init_class(class.name.to_owned())?;

                    let klass = global_memory
                        .method_area
                        .classes
                        .get(&class.name)
                        .ok_or("class not found in method area 3 :(")?;

                    let objectref = global_memory.heap.allocate_class(klass);
                    current_frame.operand_stack.push(objectref);

                    current_frame.instruction_counter += 1;
                }
                // newarray
                0xbc => {
                    current_frame.instruction_counter += 1;
                    let atype = *code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;

                    let count = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let data = vec![0; count as usize];

                    // FIXME: get type from atype and put it in type field
                    let objectref = global_memory.heap.store("[B".to_string(), data);
                    println!("objectref newarray: {}", objectref);
                    current_frame.operand_stack.push(objectref);

                    current_frame.instruction_counter += 1;
                }
                // anewarray
                0xbd => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let constant = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();
                    let class = constant.as_class().ok_or("ClassNotFound :(")?;

                    let count = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let data = vec![0; count as usize];

                    // FIXME: get type from atype and put it in type field
                    let objectref = global_memory.heap.store(format!("[L{};", class.name), data);
                    current_frame.operand_stack.push(objectref);

                    current_frame.instruction_counter += 1;
                }
                //arraylength
                0xbe => {
                    let arrayref = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("nothing to pop here")?;
                    let heap_item = global_memory
                        .heap
                        .data
                        .get(arrayref as usize)
                        .ok_or("no ref")?;
                    let field_info = parse_field_descriptor(&heap_item.field_descriptor)?;
                    if !matches!(field_info.field_type, FieldType::Array(_)) {
                        println!("{:?}", field_info.field_type);
                        return Err("expected an array".into());
                    }
                    let length = heap_item.data.len();
                    let length_bytes =
                        Cursor::new((length as i32).to_be_bytes()).read_u32::<BigEndian>()?;
                    current_frame.operand_stack.push(length_bytes);
                    current_frame.instruction_counter += 1;
                }
                // entermonitor
                0xc2 => {
                    // FIXME: Implement
                    current_frame.instruction_counter += 1;
                }
                // ifnull
                0xc6 => {
                    current_frame.instruction_counter += 1;
                    let branchbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let branchoffset =
                        Cursor::new(((branchbyte1 << 8) | branchbyte2).to_be_bytes())
                            .read_i16::<BigEndian>()?;

                    println!("branchoffset: {}", branchbyte2);

                    let value = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    println!("value: {value}");

                    if value == 0 {
                        current_frame.instruction_counter =
                            (current_frame.instruction_counter - 2) + branchoffset as usize;
                    } else {
                        current_frame.instruction_counter += 1;
                    }
                }
                // ifnonnull
                0xc7 => {
                    current_frame.instruction_counter += 1;
                    let branchbyte1 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let branchoffset =
                        Cursor::new(((branchbyte1 << 8) | branchbyte2).to_be_bytes())
                            .read_i16::<BigEndian>()?;

                    println!("branchoffset: {}", branchbyte2);

                    let value = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    println!("value: {value}");

                    if value != 0 {
                        current_frame.instruction_counter =
                            (current_frame.instruction_counter - 2) + branchoffset as usize;
                    } else {
                        current_frame.instruction_counter += 1;
                    }
                }

                i @ _ => return Err(format!("unknown instruction {i:#0x}").into()),
            }

            // println!("vm: {:?} {:?}", self, global_memory.heap)
        }

        Ok(())
    }
}

#[derive(Debug)]
struct VM {
    global_memory: GlobalMemory,
    main_thread: Thread,
}

impl VM {
    fn new() -> Rc<RefCell<VM>> {
        let vm = VM {
            global_memory: GlobalMemory {
                method_area: MethodArea {
                    classes: HashMap::new(),
                },
                heap: Heap::new(),
            },
            main_thread: Thread {
                thread_memory: ThreadMemory {
                    jvm_stack: Vec::new(),
                },
            },
        };

        let vmref = Rc::new(RefCell::new(vm));

        return vmref;
    }

    fn run(&mut self, name: String) -> Result<(), Box<dyn Error>> {
        self.global_memory
            .load_class("java/lang/Class".to_owned())?;
        self.global_memory
            .link_class("java/lang/Class".to_owned())?;
        self.global_memory
            .init_class("java/lang/Class".to_owned())?;

        self.global_memory
            .load_class("java/lang/String".to_owned())?;
        self.global_memory
            .link_class("java/lang/String".to_owned())?;
        self.global_memory
            .init_class("java/lang/String".to_owned())?;

        self.global_memory.load_class(name.to_owned())?;
        self.global_memory.link_class(name.to_owned())?;
        self.global_memory.init_class(name.to_owned())?;

        let current_frame = Frame::new(
            &mut self.global_memory,
            name,
            "main".into(),
            MethodDescriptor {
                parameter_descriptors: vec![FieldType::Array(Box::new(FieldType::ClassInstance(
                    "java/lang/String".to_owned(),
                )))],
                return_descriptor: crate::parse::ReturnDescriptor::VoidDescriptor,
            },
        )?;
        self.main_thread.thread_memory.jvm_stack.push(current_frame);
        self.main_thread.run(&mut self.global_memory)?;

        Ok(())
    }
}

pub fn run(filename: String) {
    let rt = VM::new();
    let class_name = filename;
    (*rt).borrow_mut().run(class_name.to_owned()).unwrap();
}
