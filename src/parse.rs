use std::{
    error::Error,
    io::{Cursor, Read},
    str::Chars,
};

use byteorder::{BigEndian, ReadBytesExt};

use crate::deserialize::{AttributeInfo, CPInfo, DeserializedClassFile, FieldInfo, MethodInfo};

#[derive(Debug)]
pub struct ClassAccess {
    pub public: bool,
    pub is_final: bool,
    pub is_super: bool,
    pub interface: bool,
}

impl ClassAccess {
    fn new(access_flags: u16) -> ClassAccess {
        let public = access_flags & 0x0001 == 0x0001;
        let is_final = access_flags & 0x0010 == 0x0010;
        let is_super = access_flags & 0x0020 == 0x0020;
        let interface = access_flags & 0x0200 == 0x0200;
        // TODO: add remaining access flags!

        return ClassAccess {
            public,
            is_final,
            is_super,
            interface,
        };
    }
}


#[derive(Debug, Clone)]
pub struct FieldAccess {
    pub public: bool,
    pub private: bool,
    pub protected: bool,
    pub r#static: bool,
    pub r#final: bool,
    pub volatile: bool,
    pub transient: bool,
    pub synthetic: bool,
    pub r#enum: bool,
}

impl FieldAccess {
    fn new(access_flags: u16) -> FieldAccess {
        let public = access_flags & 0x0001 == 0x0001;
        let private = access_flags & 0x0002 == 0x0002;
        let protected = access_flags & 0x0004 == 0x0004;
        let r#static = access_flags & 0x0008 == 0x0008;
        // TODO: add remaining access flags!

        return FieldAccess {
            public,
            private,
            protected,
            r#static,
            r#final: false,
            volatile: false,
            transient: false,
            synthetic: false,
            r#enum: false,
        };
    }
}

#[derive(Debug, Clone)]
pub struct MethodAccess {
    pub public: bool,
    pub private: bool,
    pub protected: bool,
    pub r#static: bool,
    pub r#final: bool,
    pub synchronized: bool,
    pub bridge: bool,
    pub varargs: bool,
    pub native: bool,
    pub r#abstract: bool,
    pub strict: bool,
    pub synthetic: bool,
}

impl MethodAccess {
    fn new(access_flags: u16) -> MethodAccess {
        let public = access_flags & 0x0001 == 0x0001;
        let private = access_flags & 0x0002 == 0x0002;
        let protected = access_flags & 0x0004 == 0x0004;
        let r#static = access_flags & 0x0008 == 0x0008;
        let r#final = access_flags & 0x0010 == 0x0010;
        let r#synchronized = access_flags & 0x0020 == 0x0020;
        let bridge = access_flags & 0x0040 == 0x0040;
        let varargs = access_flags & 0x0080 == 0x0080;
        let native = access_flags & 0x0100 == 0x0100;
        let r#abstract = access_flags & 0x0400 == 0x0400;
        let strict = access_flags & 0x0800 == 0x0800;
        let synthetic = access_flags & 0x1000 == 0x1000;

        return MethodAccess {
            public,
            private,
            protected,
            r#static,
            r#final,
            synchronized,
            bridge,
            varargs,
            native,
            r#abstract,
            strict,
            synthetic,
        };
    }
}

// https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-5.html#jvms-5.4.3.5-220
#[derive(Clone, Debug)]
pub enum RefKind {
    InvokeStatic,
}

#[derive(Clone, Debug)]
pub enum Constant {
    Class(ClassInfo),
    Utf8(String),
    String(String),
    MethodRef(ClassInfo, Box<crate::parse::Constant>),
    FieldRef(ClassInfo, Box<crate::parse::Constant>),
    NameAndType(String, String),
    InvokeDynamic(u16, Box<crate::parse::Constant>),
    MethodHandle(RefKind, Box<crate::parse::Constant>),
    MethodType(String),
    Integer(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Placeholder,
}

impl Constant {
    pub fn as_class(&self) -> Option<&ClassInfo> {
        if let Self::Class(v) = self {
            Some(v)
        } else {
            None
        }
    }

    fn as_utf8(&self) -> Option<&String> {
        if let Self::Utf8(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_method_ref(&self) -> Option<(ClassInfo, Box<Constant>)> {
        if let Self::MethodRef(value1, value2) = self {
            Some((value1.to_owned(), value2.to_owned()))
        } else {
            None
        }
    }
    pub fn as_field_ref(&self) -> Option<(ClassInfo, Box<Constant>)> {
        if let Self::FieldRef(value1, value2) = self {
            Some((value1.to_owned(), value2.to_owned()))
        } else {
            None
        }
    }
    pub fn as_name_and_type(&self) -> Option<(String, String)> {
        if let Self::NameAndType(value1, value2) = self {
            Some((value1.to_owned(), value2.to_owned()))
        } else {
            None
        }
    }
}

fn parse_or_get_constant(
    constant_pool: &mut Vec<Constant>,
    deserialized_constant_pool: &Vec<CPInfo>,
    index: u16,
) -> Result<Constant, Box<dyn Error>> {
    if !matches!(
        constant_pool
            .get((index - 1) as usize)
            .expect("constant pool to have the correct size"),
        Constant::Placeholder
    ) {
        return match constant_pool
            .get((index - 1) as usize)
            .ok_or("correct size")
        {
            Ok(v) => Ok(v.to_owned()),
            Err(e) => Err(e.into()),
        };
    }

    let cp_info = deserialized_constant_pool
        .get((index - 1) as usize)
        .ok_or("invalid index")?;

    let constant = match cp_info {
        CPInfo::ConstantClassInfo { tag, name_index } => Constant::Class(parse_class_info(
            cp_info,
            constant_pool,
            deserialized_constant_pool,
        )?),
        CPInfo::ConstantMethodRefInfo {
            tag,
            class_index,
            name_and_type_index,
        } => {
            let v = parse_or_get_constant(constant_pool, deserialized_constant_pool, *class_index)?;
            let class = v.as_class().ok_or("is not a class")?;
            let name_and_type = parse_or_get_constant(
                constant_pool,
                deserialized_constant_pool,
                *name_and_type_index,
            )?;
            Constant::MethodRef(class.to_owned(), name_and_type.into())
        }
        CPInfo::ConstantFieldRefInfo {
            tag,
            class_index,
            name_and_type_index,
        } => {
            let v = parse_or_get_constant(constant_pool, deserialized_constant_pool, *class_index)?;
            let class = v.as_class().ok_or("is not a class")?;
            let name_and_type = parse_or_get_constant(
                constant_pool,
                deserialized_constant_pool,
                *name_and_type_index,
            )?;
            Constant::FieldRef(class.to_owned(), name_and_type.into())
        }
        CPInfo::ConstantStringInfo { tag, string_index } => {
            let string_constant =
                parse_or_get_constant(constant_pool, deserialized_constant_pool, *string_index)?;
            let string = string_constant.as_utf8().ok_or("no utf8")?;
            Constant::String(string.to_owned())
        }
        CPInfo::ConstantNameAndTypeInfo {
            tag,
            name_index,
            descriptor_index,
        } => {
            let name_constant =
                parse_or_get_constant(constant_pool, deserialized_constant_pool, *name_index)?;
            let name = name_constant.as_utf8().ok_or("no utf8")?;
            let descriptor_text_constant = parse_or_get_constant(
                constant_pool,
                deserialized_constant_pool,
                *descriptor_index,
            )?;
            let descriptor_text = descriptor_text_constant.as_utf8().ok_or("no utf8")?;
            Constant::NameAndType(name.to_owned(), descriptor_text.to_owned())
        }
        info @ CPInfo::ConstantUtf8Info { .. } => Constant::Utf8(parse_utf8_info(info)),
        CPInfo::ConstantInvokeDynamicInfo {
            tag,
            bootstrap_method_attr_index,
            name_and_type_index,
        } => {
            let name_and_type = parse_or_get_constant(
                constant_pool,
                deserialized_constant_pool,
                *name_and_type_index,
            )?;

            Constant::InvokeDynamic(bootstrap_method_attr_index.to_owned(), name_and_type.into())
        }
        info @ CPInfo::ConstantMethodHandleInfo {
            tag,
            reference_kind,
            reference_index,
        } => {
            // FIXME: Derive RefKind from reference_kind
            // FIXME: decide which kind of reference to resolve using RefKind
            // FIXME: somehow check the class file version number for version specific behaviour

            let methodref_or_interface_method_ref =
                parse_or_get_constant(constant_pool, deserialized_constant_pool, *reference_index)?;
            Constant::MethodHandle(
                RefKind::InvokeStatic,
                methodref_or_interface_method_ref.into(),
            )
        }
        CPInfo::ConstantIntegerInfo { tag, bytes } => {
            Constant::Integer(Cursor::new(bytes.to_be_bytes()).read_i32::<BigEndian>()?)
        }
        CPInfo::ConstantLongInfo {
            tag,
            high_bytes,
            low_bytes,
        } => Constant::Long(
            Cursor::new((((*high_bytes as u64) << 32) + *low_bytes as u64).to_be_bytes())
                .read_i64::<BigEndian>()?,
        ),
        CPInfo::ConstantInterfaceMethodRefInfo {
            tag,
            class_index,
            name_and_type_index,
        } => {
            let v = parse_or_get_constant(constant_pool, deserialized_constant_pool, *class_index)?;
            let class = v.as_class().ok_or("is not a class")?;
            let name_and_type = parse_or_get_constant(
                constant_pool,
                deserialized_constant_pool,
                *name_and_type_index,
            )?;
            Constant::MethodRef(class.to_owned(), name_and_type.into())
        }
        CPInfo::ConstantMethodTypeInfo {
            tag,
            descriptor_index,
        } => {
            let descriptor_constant = parse_or_get_constant(
                constant_pool,
                deserialized_constant_pool,
                *descriptor_index,
            )?;
            let descriptor = descriptor_constant.as_utf8().ok_or("no utf8")?;
            Constant::MethodType(descriptor.to_owned())
        }
        CPInfo::ConstantFloatInfo { tag, bytes } => {
            Constant::Float(Cursor::new(bytes.to_be_bytes()).read_f32::<BigEndian>()?)
        }
        CPInfo::ConstantDoubleInfo {
            tag,
            high_bytes,
            low_bytes,
        } => Constant::Double(
            Cursor::new((((*high_bytes as u64) << 32) + *low_bytes as u64).to_be_bytes())
                .read_f64::<BigEndian>()?,
        ),
    };

    constant_pool[(index - 1) as usize] = constant.to_owned();

    Ok(constant)
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub name: String,
}

fn parse_utf8_info(info: &CPInfo) -> String {
    // FIXME: this all can fail, properage!
    match info {
        CPInfo::ConstantUtf8Info { bytes, .. } => String::from_utf8(bytes.to_owned()).unwrap(),
        _ => unreachable!(),
    }
}

fn parse_class_info(
    class_info: &CPInfo,
    constant_pool: &mut Vec<Constant>,
    deserialized_constant_pool: &Vec<CPInfo>,
) -> Result<ClassInfo, Box<dyn Error>> {
    match class_info {
        CPInfo::ConstantClassInfo { name_index, .. } => {
            let name =
                parse_or_get_constant(constant_pool, deserialized_constant_pool, *name_index)?;
            match name {
                Constant::Utf8(name_str) => Ok(ClassInfo { name: name_str }),
                _ => unreachable!(),
            }
        }
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone)]
pub struct Field {
    pub access: FieldAccess,
    pub name: String,
    pub descriptor: FieldDescriptor,
    pub attributes: Vec<Attribute>,
}

fn parse_field(
    field_info: &FieldInfo,
    constant_pool: &Vec<CPInfo>,
) -> Result<Field, Box<dyn Error>> {
    let access = FieldAccess::new(field_info.access_flags);
    let name_info = constant_pool
        .get((field_info.name_index - 1) as usize)
        .ok_or("failed to get name")?;
    let name = parse_utf8_info(name_info);
    // println!("name: {name}");
    let descriptor_info = constant_pool
        .get((field_info.descriptor_index - 1) as usize)
        .expect("descriptor to be present");
    let descriptor_text = parse_utf8_info(descriptor_info);
    let descriptor = parse_field_descriptor(&descriptor_text)?;

    // println!("descriptor: {descriptor:?}");

    let mut attributes = vec![];
    for attribute_info in field_info.attributes.iter() {
        attributes.push(parse_attribute(attribute_info, constant_pool)?);
    }

    Ok(Field {
        access,
        name,
        descriptor,
        attributes,
    })
}

#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    pub field_type: FieldType,
}

pub fn parse_field_descriptor(
    field_descriptor: &String,
) -> Result<FieldDescriptor, Box<dyn Error>> {
    Ok(FieldDescriptor {
        field_type: parse_field_type(&mut field_descriptor.chars())?,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldType {
    Integer,
    Boolean,
    Byte,
    Char,
    LongInteger,
    Float,
    Double,
    Short,
    ClassInstance(String),
    Array(Box<FieldType>),
}

impl FieldType {
    pub fn as_class_instance(&self) -> Option<&String> {
        if let Self::ClassInstance(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&FieldType> {
        if let Self::Array(v) = self {
            Some(&v)
        } else {
            None
        }
    }
}

fn parse_field_type(chars: &mut Chars) -> Result<FieldType, Box<dyn Error>> {
    // println!("chars: {chars:?}");
    match chars
        .nth(0)
        .ok_or("failed to get first char of field_type")?
    {
        'L' => Ok(FieldType::ClassInstance(
            chars.take_while(|c| *c != ';').collect(),
        )),
        '[' => Ok(FieldType::Array(Box::new(parse_field_type(chars)?))),
        'I' => Ok(FieldType::Integer),
        'J' => Ok(FieldType::LongInteger),
        'Z' => Ok(FieldType::Boolean),
        'B' => Ok(FieldType::Byte),
        'C' => Ok(FieldType::Char),
        'S' => Ok(FieldType::Short),
        'D' => Ok(FieldType::Double),
        'F' => Ok(FieldType::Float),
        char @ _ => unreachable!("encountered {char}"),
    }
}

#[derive(Debug, Clone)]
pub struct ExceptionTableItem {
    pub start_pc: usize,
    pub end_pc: usize,
    pub handler_pc: usize,
    pub catch_type: usize,
}

#[derive(Debug, Clone)]
pub enum Attribute {
    Code {
        max_stack: usize,
        max_locals: usize,
        bytes: Vec<u8>,
        exception_table: Vec<ExceptionTableItem>,
    },
    Placeholder,
}

impl Attribute {
    pub fn as_code(&self) -> Option<(Vec<u8>, usize, usize, Vec<ExceptionTableItem>)> {
        if let Self::Code {
            bytes,
            max_stack,
            max_locals,
            exception_table,
        } = self
        {
            Some((
                bytes.to_owned(),
                *max_stack,
                *max_locals,
                exception_table.to_owned(),
            ))
        } else {
            None
        }
    }
}

fn parse_attribute(
    attribute_info: &AttributeInfo,
    constant_pool: &Vec<CPInfo>,
) -> Result<Attribute, Box<dyn Error>> {
    let name_info = constant_pool
        .get((attribute_info.attribute_name_index - 1) as usize)
        .ok_or("expect name to be present")
        .unwrap();
    let name = parse_utf8_info(name_info);
    // println!("attribute name: {name}");

    if name == "Code" {
        let mut csr = Cursor::new(attribute_info.info.to_owned());
        let max_stack = csr.read_u16::<BigEndian>()?;
        let max_locals = csr.read_u16::<BigEndian>()?;
        let code_length = csr.read_u32::<BigEndian>()?;

        let mut code_bytes = (&mut csr).take(code_length.into());
        let mut code = vec![];
        code_bytes.read_to_end(&mut code)?;
        // println!("code: {code:?}");
        let mut et = vec![];
        let et_length = csr.read_u16::<BigEndian>()?;
        for _ in 0..et_length {
            let start_pc = csr.read_u16::<BigEndian>()?;
            let end_pc = csr.read_u16::<BigEndian>()?;
            let handler_pc = csr.read_u16::<BigEndian>()?;
            let catch_type = csr.read_u16::<BigEndian>()?;
            et.push(ExceptionTableItem {
                start_pc: start_pc as usize,
                end_pc: end_pc as usize,
                handler_pc: handler_pc as usize,
                catch_type: catch_type as usize,
            })
        }

        // TODO: attributes
        return Ok(Attribute::Code {
            max_stack: max_stack as usize,
            max_locals: max_locals as usize,
            bytes: code,
            exception_table: et,
        });
    }
    Ok(Attribute::Placeholder)
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodDescriptor {
    pub parameter_descriptors: Vec<FieldType>,
    pub return_descriptor: ReturnDescriptor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReturnDescriptor {
    FieldType(FieldType),
    VoidDescriptor,
}

pub fn parse_method_descriptor(
    method_descriptor: String,
) -> Result<MethodDescriptor, Box<dyn Error>> {
    let mut chars = method_descriptor.chars();
    // FIXME: assert that first char is '('
    let open = chars.next();

    let mut parameter_descriptors = vec![];
    while chars.to_owned().next().unwrap() != ')' {
        let field_type = parse_field_type(&mut chars)?;
        parameter_descriptors.push(field_type);
    }

    let close = chars.next();

    // parse_return_descriptor
    let return_descriptor = if chars.to_owned().next().unwrap() == 'V' {
        ReturnDescriptor::VoidDescriptor
    } else {
        ReturnDescriptor::FieldType(parse_field_type(&mut chars)?)
    };

    Ok(MethodDescriptor {
        parameter_descriptors,
        return_descriptor,
    })
}

#[derive(Debug, Clone)]
pub struct Method {
    pub access: MethodAccess,
    pub name: String,
    pub descriptor: MethodDescriptor,
    pub attributes: Vec<Attribute>,
}

fn parse_method(
    field_info: &MethodInfo,
    constant_pool: &Vec<CPInfo>,
) -> Result<Method, Box<dyn Error>> {
    let access = MethodAccess::new(field_info.access_flags);
    let name_info = constant_pool
        .get((field_info.name_index - 1) as usize)
        .ok_or("failed to get name")?;
    let name = parse_utf8_info(name_info);
    // println!("name: {name}");
    let descriptor_info = constant_pool
        .get((field_info.descriptor_index - 1) as usize)
        .expect("descriptor to be present");
    let descriptor_text = parse_utf8_info(descriptor_info);
    let descriptor = parse_method_descriptor(descriptor_text)?;

    // println!("descriptor: {descriptor:?}");

    let mut attributes = vec![];
    for attribute_info in field_info.attributes.iter() {
        let attribute = parse_attribute(attribute_info, constant_pool)?;
        attributes.push(attribute);
    }

    Ok(Method {
        access,
        name,
        descriptor,
        attributes,
    })
}

#[derive(Debug)]
pub struct Class {
    pub access: ClassAccess,
    pub constant_pool: Vec<Constant>,
    pub this_class: ClassInfo,
    pub super_class: Option<ClassInfo>,
    pub interfaces: Vec<ClassInfo>,
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
    pub attributes: Vec<Attribute>,
}

pub fn parse(class_file: DeserializedClassFile) -> Result<Class, Box<dyn Error>> {
    // println!("access_flags: 0x{:04x}", class_file.access_flags);

    let access = ClassAccess::new(class_file.access_flags);
    // println!("{access:?}");

    let mut constant_pool = vec![Constant::Placeholder; class_file.constant_pool.len()];

    let this_class = parse_or_get_constant(
        &mut constant_pool,
        &class_file.constant_pool,
        class_file.this_class,
    )?
    .as_class()
    .ok_or("no  class")?
    .to_owned();
    // println!("this_class: {this_class:?}");

    let super_class = if (class_file.super_class == 0) {
        // if super_class is 0, this is most likely java.lang.Object
        None
    } else {
        Some(
            parse_or_get_constant(
                &mut constant_pool,
                &class_file.constant_pool,
                class_file.super_class,
            )?
            .as_class()
            .ok_or("no class")?
            .to_owned(),
        )
    };

    let mut interfaces = vec![];
    for interface_index in class_file.interfaces.iter() {
        let interface = parse_or_get_constant(
            &mut constant_pool,
            &class_file.constant_pool,
            *interface_index,
        )?
        .as_class()
        .ok_or("no class")?
        .to_owned();
        // println!("{interface:?}");
        interfaces.push(interface);
    }

    let mut fields = vec![];
    for field_info in class_file.fields.iter() {
        let field = parse_field(field_info, &class_file.constant_pool)?;
        fields.push(field);
    }

    let mut methods = vec![];
    for method_info in class_file.methods.iter() {
        let method = parse_method(method_info, &class_file.constant_pool)?;
        methods.push(method);
    }

    for attribute_info in class_file.attributes.iter() {
        parse_attribute(&attribute_info, &class_file.constant_pool)?;
    }

    for i in 0..constant_pool.len() {
        if matches!(constant_pool[i], Constant::Placeholder) {
            let e =
                parse_or_get_constant(&mut constant_pool, &class_file.constant_pool, i as u16 + 1)?;
        }
    }
    // println!("constants: {:?}", constant_pool);

    let class = Class {
        access,
        constant_pool,
        this_class,
        super_class,
        interfaces,
        fields,
        methods,
        attributes: vec![],
    };

    // println!("class {:?}", class);

    Ok(class)
}
