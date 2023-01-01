use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    io::Cursor,
    path::Path,
    rc::{Rc, Weak},
    vec,
};

use byteorder::{BigEndian, ReadBytesExt};

use crate::{
    deserialize::deserialize_class_file,
    parse::{parse, parse_method_descriptor, Attribute, Class as ParsedClass, Constant, Method},
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
    program_counter: usize, // unused, as we use ic for now :^) might get relevant when we allow
                            // frame switching?
}

impl Frame {
    fn run(&mut self, global_memory: &mut GlobalMemory) -> Result<(), Box<dyn Error>> {
        let mut ic = 0;
        while ic < self.code_bytes.len() {
            let instruction = self.code_bytes.get(ic).ok_or("no bytes")?;
            println!("instruction: {instruction:0x}");
            match instruction {
                // bipush
                0x10 => {
                    ic += 1;
                    let byte = self.code_bytes.get(ic).ok_or("no bytes")?;

                    let mut sign_extended = Cursor::new(byte.to_be_bytes());
                    let as_i8 = sign_extended.read_i8()?;
                    println!("as i8 {as_i8}");
                    self.operand_stack
                        .push(Cursor::new((as_i8 as i32).to_be_bytes()).read_u32::<BigEndian>()?);
                    ic += 1;
                }
                // ldc
                0x12 => {
                    ic += 1;
                    let index = self.code_bytes.get(ic).ok_or("no bytes")?;
                    let a = self
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();
                    match a {
                        // FIXME: That's not how to construct a new class at all
                        Constant::String(string) => {
                            let reference = global_memory.heap.store(string.bytes().collect());
                            self.operand_stack.push(reference);
                        }
                        // FIXME: Some are not actually unreachable
                        _ => unreachable!(),
                    }
                    ic += 1;
                }
                // iload
                0x15 => {
                    ic += 1;
                    let index = self.code_bytes.get(ic).ok_or("no bytes")?;
                    let integer = self.local_variables[*index as usize];
                    self.operand_stack.push(integer);
                    ic += 1;
                }
                // iload_n
                instruction @ (0x1a | 0x1b | 0x1c | 0x1d) => {
                    let integer = self.local_variables[(instruction - 0x1a) as usize];
                    self.operand_stack.push(integer);

                    ic += 1;
                }
                // istore
                0x36 => {
                    ic += 1;
                    let index = self.code_bytes.get(ic).ok_or("no bytes")?;
                    let integer = self
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    self.local_variables[*index as usize] = integer;

                    ic += 1;
                }
                // istore_n
                instruction @ (0x3b | 0x3c | 0x3d | 0x3e) => {
                    let integer = self
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    self.local_variables[(instruction - 0x3b) as usize] = integer;

                    ic += 1;
                }
                // iadd
                0x60 => {
                    let value2 = self
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = self
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let result = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?
                        + Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    self.operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    ic += 1;
                }
                // isub
                0x64 => {
                    let value2 = self
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let value1 = self
                        .operand_stack
                        .pop()
                        .ok_or("no item on the operand_stack")?;
                    let v2 = Cursor::new(value2.to_be_bytes()).read_i32::<BigEndian>()?;
                    let v1 = Cursor::new(value1.to_be_bytes()).read_i32::<BigEndian>()?;
                    println!("v1 is {v1} and v2 is {v2}");
                    let result = v1 - v2;
                    println!("result is {result}");
                    self.operand_stack
                        .push(Cursor::new(result.to_be_bytes()).read_u32::<BigEndian>()?);
                    ic += 1;
                }
                // return
                0xb1 => {
                    ic += 1;
                    // stop evaluating ?
                    break;
                }
                // getstatic
                0xb2 => {
                    ic += 1;
                    let indexbyte1 = (*self.code_bytes.get(ic).ok_or("no bytes")?) as u16;
                    ic += 1;
                    let indexbyte2 = (*self.code_bytes.get(ic).ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let a = self
                        .constant_pool
                        .clone()
                        .upgrade()
                        .ok_or("no constant_pool")?
                        .pool
                        .get((index - 1) as usize)
                        .ok_or("expected ur mom")?
                        .to_owned();
                    println!("a {a:?}");
                    // TODO: parse descriptor
                    // TODO: resolve reference
                    // TODO: push value onto stack
                    self.operand_stack.push(0x1337);

                    ic += 1;
                }
                // invokevirtual indexbyte1 indexbyte2
                0xb6 => {
                    ic += 1;
                    let indexbyte1 = (*self.code_bytes.get(ic).ok_or("no bytes")?) as u16;
                    ic += 1;
                    let indexbyte2 = (*self.code_bytes.get(ic).ok_or("no bytes")?) as u16;

                    let index = (indexbyte1 << 8) | indexbyte2;

                    let (class_info, name_and_type) = self
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
                        let narg = self
                            .operand_stack
                            .pop()
                            .ok_or("nargs is not on the stack")?;
                        nargs.insert(0, narg);
                    }
                    let object_ref = self
                        .operand_stack
                        .pop()
                        .ok_or("object_ref is not on the stack")?;

                    // ugly workaround for println
                    if object_ref == 0x1337 {
                        let arg_type = type_descriptor
                            .parameter_descriptors
                            .first()
                            .ok_or("no argument - wtf?")?;
                        match arg_type {
                            crate::parse::FieldType::ClassInstance(class_name) => {
                                if class_name == "java/lang/String" {
                                    let heap_item = global_memory
                                        .heap
                                        .data
                                        .get(*nargs.first().ok_or("no first argument?")? as usize)
                                        .ok_or("no heap item")?;
                                    let string_text = std::str::from_utf8(&heap_item.data)?;
                                    println!("\x1b[93mOUT:\x1b[0m {}", string_text);
                                } else {
                                    return Err("no ugly workaround for this class type :(".into());
                                }
                            }
                            crate::parse::FieldType::Integer => {
                                let argument_as_int_bytes =
                                    nargs.first().ok_or("no first argument?")?;
                                let integer = Cursor::new(argument_as_int_bytes.to_be_bytes())
                                    .read_i32::<BigEndian>()?;
                                println!("\x1b[93mOUT:\x1b[0m {}", integer);
                            }
                            _ => todo!(),
                        }
                    } else {
                        return Err("no ugly workaround for this method :(".into());
                    }

                    ic += 1;
                }

                i @ _ => return Err(format!("unknown instruction {i}").into()),
            }

            println!("vm: {:?} {:?}", self, global_memory.heap)
        }

        Ok(())
    }
}

#[derive(Debug)]
struct GlobalMemory {
    heap: Heap,
    method_area: MethodArea,
}

#[derive(Debug)]
struct Heap {
    data: Vec<HeapItem>,
}

impl Heap {
    fn store(&mut self, data: Vec<u8>) -> u32 {
        self.data.push(HeapItem { data });
        return (self.data.len() - 1) as u32;
    }
}

#[derive(Debug)]
struct HeapItem {
    data: Vec<u8>,
}

#[derive(Debug)]
struct MethodArea {
    class_specific_data: HashMap<String, MethodAreaClassSpecificData>,
}

#[derive(Debug)]
struct MethodAreaClassSpecificData {
    parsed_class: ParsedClass,
    constant_pool: Rc<RuntimeConstantPool>,
}

#[derive(Debug)]
struct RuntimeConstantPool {
    pool: Vec<Constant>,
}

#[derive(Debug)]
struct VM {
    global_memory: GlobalMemory,
    thread_memory: ThreadMemory,
}

impl VM {
    fn new() -> VM {
        VM {
            global_memory: GlobalMemory {
                method_area: MethodArea {
                    class_specific_data: HashMap::new(),
                },
                heap: Heap { data: vec![] },
            },
            thread_memory: ThreadMemory {
                jvm_stack: Vec::new(),
            },
        }
    }

    fn load_class(&mut self, name: String) -> Result<(), Box<dyn Error>> {
        if self
            .global_memory
            .method_area
            .class_specific_data
            .contains_key(&name)
        {
            println!("class already loaded, skipping");
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

        let mut pool = vec![];
        for item in class.constant_pool.iter() {
            match item {
                Constant::Class(_)
                | Constant::String(_)
                | Constant::MethodRef(_, _)
                | Constant::FieldRef(_, _) => pool.push(item.to_owned()),
                _ => {}
            }
        }

        self.global_memory.method_area.class_specific_data.insert(
            name,
            MethodAreaClassSpecificData {
                parsed_class: class,
                constant_pool: Rc::new(RuntimeConstantPool { pool: pool }),
            },
        );
        // TODO: also load super classes => we dont have them yet :(

        Ok(())
    }

    fn run(&mut self, name: String) -> Result<(), Box<dyn Error>> {
        self.load_class(name.to_owned())?;

        let current_class = self
            .global_memory
            .method_area
            .class_specific_data
            .get(&name)
            .ok_or("Class not found :(")?;

        let current_method = current_class
            .parsed_class
            .methods
            .iter()
            .filter(|method| method.name == "main")
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
            constant_pool: Rc::downgrade(&current_class.constant_pool.to_owned()),
            local_variables: vec![0; 10],
            operand_stack: vec![],
            program_counter: 0,
            code_bytes: code_bytes.to_owned(),
        };
        self.thread_memory.jvm_stack.push(current_frame);

        let current_frame = self
            .thread_memory
            .jvm_stack
            .last_mut()
            .ok_or("no item on stack")?;

        current_frame.run(&mut self.global_memory)?;

        Ok(())
    }
}

pub fn run(filename: String) {
    let mut rt = VM::new();
    let class_name = filename.trim_end_matches(".class");
    rt.run(class_name.to_owned()).unwrap();
}
