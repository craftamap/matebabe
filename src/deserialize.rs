use std::{
    error::Error,
    fs::File,
    io::{BufReader, Cursor, Read},
};

use byteorder::{BigEndian, ReadBytesExt};

#[derive(Debug)]
pub struct DeserializedClassFile {
    pub magic: u32,
    pub minor_version: u16,
    pub major_version: u16,
    pub constant_pool_count: u16,
    pub constant_pool: Vec<CPInfo>,
    pub access_flags: u16,
    pub this_class: u16,
    pub super_class: u16,
    pub interfaces_count: u16,
    pub interfaces: Vec<u16>,
    pub fields_count: u16,
    pub fields: Vec<FieldInfo>,
    pub methods_count: u16,
    pub methods: Vec<MethodInfo>,
    pub attributes_count: u16,
    pub attributes: Vec<AttributeInfo>,
}

#[derive(Debug)]
pub enum CPInfo {
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.1
    ConstantClassInfo {
        tag: u8,
        name_index: u16,
    },
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.2
    ConstantMethodRefInfo {
        tag: u8,
        class_index: u16,
        name_and_type_index: u16,
    },
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.2
    ConstantFieldRefInfo {
        tag: u8,
        class_index: u16,
        name_and_type_index: u16,
    },
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.3
    ConstantStringInfo {
        tag: u8,
        string_index: u16,
    },
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.4
    ConstantIntegerInfo {
        tag: u8,
        bytes: u32,
    },
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.6
    ConstantNameAndTypeInfo {
        tag: u8,
        name_index: u16,
        descriptor_index: u16,
    },
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.7
    ConstantUtf8Info {
        tag: u8,
        length: u16,
        bytes: Vec<u8>,
    },
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.8
    ConstantMethodHandleInfo {
        tag: u8,
        reference_kind: u8,
        reference_index: u16,
    },
    // https://docs.oracle.com/javase/specs/jvms/se11/html/jvms-4.html#jvms-4.4.10
    ConstantInvokeDynamicInfo {
        tag: u8,
        bootstrap_method_attr_index: u16,
        name_and_type_index: u16,
    },
}

#[derive(Debug)]
pub struct FieldInfo {
    pub access_flags: u16,
    pub name_index: u16,
    pub descriptor_index: u16,
    pub attributes_count: u16,
    pub attributes: Vec<AttributeInfo>,
}

#[derive(Debug)]
pub struct AttributeInfo {
    pub attribute_name_index: u16,
    pub attribute_length: u32,
    pub info: Vec<u8>,
}

#[derive(Debug)]
pub struct MethodInfo {
    pub access_flags: u16,
    pub name_index: u16,
    pub descriptor_index: u16,
    pub attributes_count: u16,
    pub attributes: Vec<AttributeInfo>,
}

fn deserialize_constant_pool(rdr: &mut Cursor<Vec<u8>>) -> Result<CPInfo, Box<dyn Error>> {
    let tag = rdr.read_u8()?;
    println!("tag: {tag}");

    match tag {
        // CONSTANT_Utf8
        1 => {
            let length = rdr.read_u16::<BigEndian>()?;
            let mut buf = vec![];
            rdr.take(length.into()).read_to_end(&mut buf)?;
            // let str = String::from_utf8(buf.to_owned()).unwrap();
            // println!("{str}");

            Ok(CPInfo::ConstantUtf8Info {
                tag,
                length,
                bytes: buf,
            })
        }
        // CONSTANT_Integer
        3 => {
            let bytes = rdr.read_u32::<BigEndian>()?;
            Ok(CPInfo::ConstantIntegerInfo { tag, bytes })
        }
        // CONSTANT_Class
        7 => {
            let name_index = rdr.read_u16::<BigEndian>()?;

            Ok(CPInfo::ConstantClassInfo { tag, name_index })
        }
        // CONSTANT_String
        8 => {
            let string_index = rdr.read_u16::<BigEndian>()?;

            Ok(CPInfo::ConstantStringInfo { tag, string_index })
        }
        // CONSTANT_Fieldref
        9 => {
            let class_index = rdr.read_u16::<BigEndian>()?;
            let name_and_type_index = rdr.read_u16::<BigEndian>()?;

            Ok(CPInfo::ConstantFieldRefInfo {
                tag,
                class_index,
                name_and_type_index,
            })
        }
        // CONSTANT_Methodref
        10 => {
            let class_index = rdr.read_u16::<BigEndian>()?;
            let name_and_type_index = rdr.read_u16::<BigEndian>()?;

            Ok(CPInfo::ConstantMethodRefInfo {
                tag,
                class_index,
                name_and_type_index,
            })
        }
        // CONSTANT_NameAndType
        12 => {
            let name_index = rdr.read_u16::<BigEndian>()?;
            let descriptor_index = rdr.read_u16::<BigEndian>()?;

            Ok(CPInfo::ConstantNameAndTypeInfo {
                tag,
                name_index,
                descriptor_index,
            })
        }
        15 => {
            let reference_kind = rdr.read_u8()?;
            let reference_index = rdr.read_u16::<BigEndian>()?;
            Ok(CPInfo::ConstantMethodHandleInfo {
                tag,
                reference_kind,
                reference_index,
            })
        }
        18 => {
            let bootstrap_method_attr_index = rdr.read_u16::<BigEndian>()?;
            let name_and_type_index = rdr.read_u16::<BigEndian>()?;
            Ok(CPInfo::ConstantInvokeDynamicInfo {
                tag,
                bootstrap_method_attr_index,
                name_and_type_index,
            })
        }
        _ => todo!(),
    }
}

fn deserialize_attributes(
    rdr: &mut Cursor<Vec<u8>>,
    attributes_count: u16,
) -> Result<Vec<AttributeInfo>, Box<dyn Error>> {
    let mut attributes = vec![];
    for _ in 0..attributes_count {
        let attribute_name_index = rdr.read_u16::<BigEndian>()?;
        let attribute_length = rdr.read_u32::<BigEndian>()?;

        let mut buf = vec![];
        rdr.take(attribute_length.into()).read_to_end(&mut buf)?;
        attributes.push(AttributeInfo {
            attribute_name_index,
            attribute_length,
            info: buf,
        })
    }

    return Ok(attributes);
}

pub fn deserialize_class_file(path: String) -> Result<DeserializedClassFile, Box<dyn Error>> {
    let f = File::open(path)?;
    let mut reader = BufReader::new(f);
    let mut buffer = Vec::new();

    reader.read_to_end(&mut buffer)?;

    let mut rdr = Cursor::new(buffer);
    let magic = rdr.read_u32::<BigEndian>()?;
    if magic != 0xcafebabe {
        // error case!
        return Err("no cafebabe :(".into());
    }

    let minor_version = rdr.read_u16::<BigEndian>()?;
    let major_version = rdr.read_u16::<BigEndian>()?;
    // We support java 11 for now, so make sure that major_version is in between 45 and 55
    if major_version > 55 {
        return Err("unsupported major_version {major_version}".into());
    }

    println!("{magic:#0x} {minor_version} {major_version}");

    let constant_pool_count = rdr.read_u16::<BigEndian>()?;
    println!("constant_pool_count: {constant_pool_count}");
    let mut constant_pool: Vec<CPInfo> = Vec::new();
    for _ in 0..constant_pool_count - 1 {
        let cp_info = deserialize_constant_pool(&mut rdr)?;
        println!("{cp_info:?}");
        constant_pool.push(cp_info);
    }

    let access_flags = rdr.read_u16::<BigEndian>()?;
    let this_class = rdr.read_u16::<BigEndian>()?;
    let super_class = rdr.read_u16::<BigEndian>()?;

    let interfaces_count = rdr.read_u16::<BigEndian>()?;
    let mut interfaces = vec![];
    for _ in 0..interfaces_count {
        interfaces.push(rdr.read_u16::<BigEndian>()?);
    }

    let fields_count = rdr.read_u16::<BigEndian>()?;
    let mut fields = vec![];
    for _ in 0..fields_count {
        let access_flags = rdr.read_u16::<BigEndian>()?;
        let name_index = rdr.read_u16::<BigEndian>()?;
        let descriptor_index = rdr.read_u16::<BigEndian>()?;
        let attributes_count = rdr.read_u16::<BigEndian>()?;

        let attributes = deserialize_attributes(&mut rdr, attributes_count)?;

        fields.push(FieldInfo {
            access_flags,
            name_index,
            descriptor_index,
            attributes_count,
            attributes,
        })
    }

    let methods_count = rdr.read_u16::<BigEndian>()?;
    let mut methods = vec![];
    for _ in 0..methods_count {
        let access_flags = rdr.read_u16::<BigEndian>()?;
        let name_index = rdr.read_u16::<BigEndian>()?;
        let descriptor_index = rdr.read_u16::<BigEndian>()?;
        let attributes_count = rdr.read_u16::<BigEndian>()?;

        let attributes = deserialize_attributes(&mut rdr, attributes_count)?;

        methods.push(MethodInfo {
            access_flags,
            name_index,
            descriptor_index,
            attributes_count,
            attributes,
        })
    }
    let attributes_count = rdr.read_u16::<BigEndian>()?;
    let attributes = deserialize_attributes(&mut rdr, attributes_count)?;

    let deserialized_class_file = DeserializedClassFile {
        magic,
        minor_version,
        major_version,
        constant_pool_count,
        constant_pool,
        access_flags,
        this_class,
        super_class,
        interfaces_count,
        interfaces,
        fields_count,
        fields,
        methods_count,
        methods,
        attributes_count,
        attributes,
    };
    println!("deserialize_class_file: {deserialized_class_file:?}");

    return Ok(deserialized_class_file);
}
