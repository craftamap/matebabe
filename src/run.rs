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
        ClassInfo, Constant, Field, FieldType, MethodDescriptor,
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
    code_bytes: Vec<u8>,
    instruction_counter: usize,
}

impl Frame {
    fn new(
        global_memory: &mut GlobalMemory,
        class_name: String,
        method_name: String,
        type_descriptor: MethodDescriptor,
    ) -> Result<Frame, Box<dyn Error>> {
        let current_class = global_memory
            .method_area
            .classes
            .get(&class_name)
            .ok_or(format!("Class not found {} :(", class_name))?;

        let parsed_class = current_class
            .parsed_class
            .as_ref()
            .ok_or("no parsed_class")?
            .clone();
        let current_method = parsed_class
            .methods
            .iter()
            .filter(|method| method.name == method_name && method.descriptor == type_descriptor)
            .next()
            .ok_or("methodnotfound :(")?;

        let code = current_method
            .attributes
            .iter()
            .filter(|attr| matches!(attr, Attribute::Code { .. }))
            .next()
            .ok_or("no code :(")?;

        let code_bytes = code.as_code().ok_or("no code :(")?;
        let current_frame = Frame {
            constant_pool: Rc::downgrade(
                &current_class
                    .constant_pool
                    .to_owned()
                    .ok_or("couldnt find stuff")?,
            ),
            local_variables: vec![0; 20],
            operand_stack: vec![],
            code_bytes: code_bytes.to_owned(),
            instruction_counter: 0,
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
            "/tmp/jdk11u/build/linux-x86_64-normal-server-release/jdk/modules/java.base",
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
            name,
            Klass {
                parsed_class: Some(rc_class.clone()),
                field_layout: None,
                constant_pool: None,
                static_fields: Some(vec![]),
            },
        );
        return Ok(());
    }

    fn link_class(&mut self, class_name: String) -> Result<(), Box<dyn Error>> {
        println!("linking class {class_name}");
        let class = self
            .method_area
            .classes
            .get(&class_name)
            .ok_or("class not found")?;

        if class.is_linked() {
            println!("Class {class_name} already linked, skip linking :^)");
            return Ok(());
        }

        let class = class
            .parsed_class
            .as_ref()
            .ok_or("class exists, but we didnt parse it yet")?
            .to_owned();

        if class.super_class.is_some() {
            self.link_class(class.super_class.as_ref().unwrap().name.to_owned())?;
        }

        let mut pool = vec![];
        for item in class.constant_pool.iter() {
            match item {
                Constant::Class(_)
                | Constant::String(_)
                | Constant::Integer(_)
                | Constant::Long(_)
                | Constant::MethodHandle(..)
                | Constant::MethodRef(_, _)
                | Constant::FieldRef(_, _) => pool.push(item.to_owned()),
                _ => {}
            }
        }

        let method_area_class = self
            .method_area
            .classes
            .get_mut(&class.this_class.name.to_owned());

        // preperation
        let mut field_values: Vec<u32> = vec![];
        for field in class.fields.iter() {
            if field.access.r#static {
                match field.descriptor.field_type {
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
        }

        if let Some(method_area_class) = method_area_class {
            method_area_class.constant_pool = Some(Rc::new(RuntimeConstantPool { pool }));
            method_area_class
                .static_fields
                .as_mut()
                .unwrap()
                .append(&mut field_values);
        } else {
            return Err("what?".into());
        }

        println!("global_memory: {:?}", self);
        Ok(())
    }

    fn init_class(&mut self, class_name: String) -> Result<(), Box<dyn Error>> {
        println!("init class {class_name}");

        let class = self
            .method_area
            .classes
            .get(&class_name)
            .ok_or("class not found")?;

        if let Some(_) = class
            .parsed_class
            .as_ref()
            .unwrap()
            .deref()
            .methods
            .iter()
            .find(|m| m.name == "clinit")
        {
            let current_frame = Frame::new(
                self,
                class_name,
                "clinit".into(),
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
        self.data.push(HeapItem { field_ref, data });
        return (self.data.len() - 1) as u32;
    }
}

#[derive(Debug)]
struct HeapItem {
    // header
    field_ref: String,
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

        for field in parsed_class.fields.iter() {
            if field.access.r#static {
                continue;
            }
            match field.descriptor.field_type {
                crate::parse::FieldType::Integer
                | crate::parse::FieldType::Boolean
                | crate::parse::FieldType::Array(_)
                | crate::parse::FieldType::Byte
                | crate::parse::FieldType::Char
                | crate::parse::FieldType::Float
                | crate::parse::FieldType::ClassInstance(_) => {
                    fields.push((class_name.to_owned(), field.name.to_owned(), 1));
                }
                crate::parse::FieldType::LongInteger | crate::parse::FieldType::Double => {
                    fields.push((class_name.to_owned(), field.name.to_owned(), 2));
                }
            }
        }

        if let Some(class_info) = &parsed_class.super_class {
            let mut parent_fields = self
                .classes
                .get(&class_info.name.to_owned())
                .unwrap()
                .field_layout
                .clone()
                .unwrap();
            parent_fields.append(&mut fields);
            fields = parent_fields;
        };

        class.field_layout = Some(fields);

        self.classes.insert(class_name, class);
    }
}

#[derive(Debug)]
struct Klass {
    parsed_class: Option<Rc<ParsedClass>>,
    constant_pool: Option<Rc<RuntimeConstantPool>>,
    static_fields: Option<Vec<u32>>,
    field_layout: Option<Vec<(String, String, usize)>>,
}

impl Klass {
    fn is_linked(&self) -> bool {
        self.constant_pool.is_some()
    }

    fn field_offset_with_strings(
        &self,
        searched_class_name: String,
        searched_field_name: String,
    ) -> usize {
        let mut offset = 0;
        for (class_name, field_name, width) in self.field_layout.as_ref().unwrap().iter() {
            if searched_class_name != class_name.to_owned() {
                offset += width;
                continue;
            }
            if searched_field_name != field_name.to_owned() {
                offset += width;
                continue;
            }

            return offset;
        }
        // FIXME: 0 is not a error case :^)
        0
    }
    // TODO: this should be allowed to return errors
    fn field_offset(&self, field_ref_constant: Constant) -> usize {
        let mut offset = 0;
        let field_ref = field_ref_constant.as_field_ref().unwrap();
        for (class_name, field_name, width) in self.field_layout.as_ref().unwrap().iter() {
            if field_ref.0.name != class_name.to_owned() {
                offset += width;
                continue;
            }
            let (name, _) = field_ref.1.to_owned().as_name_and_type().unwrap();
            if name != field_name.to_owned() {
                offset += width;
                continue;
            }

            return offset;
        }
        // FIXME: 0 is not a error case :^)
        0
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

impl Thread {
    fn run(&mut self, global_memory: &mut GlobalMemory) -> Result<(), Box<dyn Error>> {
        loop {
            let current_frame = self
                .thread_memory
                .jvm_stack
                .last_mut()
                .ok_or("no item on jvm stack")?;
            let instruction = current_frame
                .code_bytes
                .get(current_frame.instruction_counter)
                .ok_or("no bytes")?;
            println!("instruction: {instruction:#0x}");
            match instruction {
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
                    let byte = current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;

                    let mut sign_extended = Cursor::new(byte.to_be_bytes());
                    let as_i8 = sign_extended.read_i8()?;
                    println!("as i8 {as_i8}");
                    current_frame
                        .operand_stack
                        .push(Cursor::new((as_i8 as i32).to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // ldc
                0x12 => {
                    current_frame.instruction_counter += 1;
                    let index = current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;
                    let a = current_frame
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();
                    match a {
                        Constant::String(string) => {
                            let field_layout = global_memory
                                .method_area
                                .classes
                                .get("java/lang/String")
                                .ok_or("class not found in method area :(")?
                                .field_layout
                                .as_ref()
                                .ok_or("no class layout")?;

                            let mut field_values = vec![];
                            for field in field_layout.iter() {
                                for _ in 0..field.2 {
                                    field_values.push(0);
                                }
                            }

                            let string_objectref = global_memory
                                .heap
                                .store("java/lang/String".to_owned(), field_values);

                            let bytes = string.bytes().map(|b| b as u32).collect::<Vec<u32>>();

                            let array_objectref = global_memory.heap.store("[B".to_owned(), bytes);

                            let string_klass = *global_memory
                                .method_area
                                .classes
                                .get("java/lang/String")
                                .as_ref()
                                .unwrap();

                            let field_offset = string_klass.field_offset_with_strings(
                                "java/lang/String".to_owned(),
                                "bytes".to_owned(),
                            );

                            global_memory
                                .heap
                                .data
                                .get_mut(string_objectref.to_owned() as usize)
                                .as_mut()
                                .ok_or("no object at byte location")?
                                .data[field_offset] = array_objectref;

                            current_frame.operand_stack.push(string_objectref);
                        }
                        // FIXME: Some are not actually unreachable
                        _ => unreachable!(),
                    }
                    current_frame.instruction_counter += 1;
                }
                // iload
                0x15 => {
                    current_frame.instruction_counter += 1;
                    let index = current_frame
                        .code_bytes
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
                // istore
                0x36 => {
                    current_frame.instruction_counter += 1;
                    let index = current_frame
                        .code_bytes
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
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?
                        - Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    println!("result is {result}");
                    current_frame
                        .operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    current_frame.instruction_counter += 1;
                }
                // iinc
                0x84 => {
                    current_frame.instruction_counter += 1;
                    let index = *current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;
                    current_frame.instruction_counter += 1;
                    let the_const = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as i32;

                    let value = Cursor::new(
                        current_frame
                            .local_variables
                            .get(index as usize)
                            .ok_or("no variable in local storage index")?
                            .to_be_bytes(),
                    )
                    .read_i32::<BigEndian>()?;
                    let new_value = value + the_const;
                    current_frame.local_variables[index as usize] =
                        Cursor::new(new_value.to_be_bytes()).read_u32::<BigEndian>()?;
                    current_frame.instruction_counter += 1;
                }
                // ifeq
                0x99 => {
                    current_frame.instruction_counter += 1;
                    let branchbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;

                    let branchoffset =
                        Cursor::new(((branchbyte1 << 8) | branchbyte2).to_be_bytes())
                            .read_i16::<BigEndian>()?;

                    let value = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let result = value == 0;
                    if result {
                        current_frame.instruction_counter =
                            current_frame.instruction_counter - 2 + branchoffset as usize;
                    } else {
                        current_frame.instruction_counter += 1;
                    }
                }
                // if_icmpge
                0xa2 => {
                    current_frame.instruction_counter += 1;
                    let branchbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*current_frame
                        .code_bytes
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
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?
                        >= Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;

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
                    let branchbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*current_frame
                        .code_bytes
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
                // ireturn
                0xac => {
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
                    let indexbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*current_frame
                        .code_bytes
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

                    let (class_info, name_and_type) =
                        field_ref_constant.as_field_ref().ok_or("not a field_ref")?;
                    let (name, field_descriptor_text) = name_and_type
                        .as_name_and_type()
                        .ok_or("not a NameAndType")?;
                    let type_descriptor = parse_field_descriptor(field_descriptor_text)?;

                    global_memory.load_class(class_info.name.to_owned())?;
                    global_memory.link_class(class_info.name.to_owned())?;
                    global_memory.init_class(class_info.name.to_owned())?;

                    let class = global_memory.method_area.classes.get(&class_info.name);
                    let class = class.as_ref().unwrap().deref();

                    // TODO: Add helper for getting the static field by name/constant
                    let mut idx = 0;
                    for field in class.parsed_class.as_ref().unwrap().fields.iter() {
                        if field.access.r#static && field.name == name {
                            break;
                        }
                        if field.access.r#static {
                            idx += 1;
                        }
                    }

                    current_frame.operand_stack.push(idx);

                    current_frame.instruction_counter += 1;
                }
                // getfield indexbyte1 indexbyte2
                0xb4 => {
                    current_frame.instruction_counter += 1;
                    let indexbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*current_frame
                        .code_bytes
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

                    let class_name = global_memory
                        .heap
                        .data
                        .get(objectref as usize)
                        .ok_or(format!("object {objectref} not found on heap!"))?
                        .field_ref
                        .to_owned();
                    let offset = global_memory
                        .method_area
                        .classes
                        .get(&class_name)
                        .ok_or(format!("didnt find class {class_name} in method_area"))?
                        .field_offset(constant);

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
                    let indexbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*current_frame
                        .code_bytes
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

                    let class_name = global_memory
                        .heap
                        .data
                        .get(objectref as usize)
                        .ok_or("object not found on heap!")?
                        .field_ref
                        .to_owned();
                    let offset = global_memory
                        .method_area
                        .classes
                        .get(&class_name)
                        .ok_or("didnt find class in method_area")?
                        .field_offset(constant);

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
                    let indexbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*current_frame
                        .code_bytes
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
                    let indexbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*current_frame
                        .code_bytes
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
                    let indexbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*current_frame
                        .code_bytes
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
                    let indexbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let indexbyte2 = (*current_frame
                        .code_bytes
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

                    let field_layout = global_memory
                        .method_area
                        .classes
                        .get(&class.name)
                        .ok_or("class not found in method area :(")?
                        .field_layout
                        .as_ref()
                        .ok_or("no class layout")?;

                    let mut field_values = vec![];
                    for field in field_layout.iter() {
                        for _ in 0..field.2 {
                            field_values.push(0);
                        }
                    }

                    let objectref = global_memory
                        .heap
                        .store(class.name.to_owned(), field_values);
                    current_frame.operand_stack.push(objectref);

                    current_frame.instruction_counter += 1;
                }
                // newarray
                0xbc => {
                    current_frame.instruction_counter += 1;
                    let atype = *current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?;

                    let count = current_frame
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let data = vec![0; count as usize];

                    // FIXME: get type from atype and put it in type field 
                    let objectref = global_memory.heap.store("[".to_string(), data);
                    current_frame.operand_stack.push(objectref);

                    current_frame.instruction_counter += 1;
                }
                // entermonitor
                0xc2 => {
                    // FIXME: Implement
                    current_frame.instruction_counter += 1;
                }
                // ifnonnull
                0xc7 => {
                    current_frame.instruction_counter += 1;
                    let branchbyte1 = (*current_frame
                        .code_bytes
                        .get(current_frame.instruction_counter)
                        .ok_or("no bytes")?) as u16;
                    current_frame.instruction_counter += 1;
                    let branchbyte2 = (*current_frame
                        .code_bytes
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

            println!("vm: {:?} {:?}", self, global_memory.heap)
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
            .load_class("java/lang/String".to_owned())?;
        self.global_memory
            .link_class("java/lang/String".to_owned())?;
        self.global_memory
            .load_class("java/lang/String".to_owned())?;

        self.global_memory.load_class(name.to_owned())?;
        self.global_memory.link_class(name.to_owned())?;
        self.global_memory.load_class(name.to_owned())?;

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
