use std::{
    collections::HashMap,
    io::{Cursor, Read},
};

#[derive(Debug, Clone)]
pub struct ApiSchema {
    pub name: String,
    pub category: String,
    pub arg_names: Vec<String>,
}

#[derive(Default)]
pub struct SchemaRegistry {
    map: HashMap<i32, ApiSchema>,
}

impl SchemaRegistry {
    pub fn ingest_info(&mut self, doc: &bson::Document) -> anyhow::Result<()> {
        let index = doc.get_i32("I")?;
        let name = doc.get_str("name")?.to_string();
        let category = doc.get_str("category")?.to_string();
        let arg_names: Vec<String> = doc
            .get_array("args")?
            .iter()
            .map(|v| match v {
                // args can be ["name", "type"] or just "name"
                bson::Bson::Array(arr) => arr
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                bson::Bson::String(s) => s.clone(),
                _ => "unknown".to_string(),
            })
            .collect();
        let schema = ApiSchema {
            name,
            category,
            arg_names,
        };
        println!(
            "=======================\n\n{}\n{:?}\n\n=======================",
            doc, schema
        );
        self.map.insert(index, schema);
        Ok(())
    }
    pub fn get(&self, index: i32) -> Option<&ApiSchema> {
        self.map.get(&index)
    }
}

#[derive(Debug, Clone)]
pub struct ApiCall {
    pub api: String,
    pub category: String,
    pub status: String,
    pub return_value: i64,
    pub thread_id: u32,
    pub time: i32,
    pub arguments: Vec<(String, String)>,
}

fn bson_to_string(val: &bson::Bson) -> String {
    match val {
        bson::Bson::String(s) => s.clone(),
        bson::Bson::Int32(i) => format!("0x{:x}", i),
        bson::Bson::Int64(i) => format!("0x{:x}", i),
        bson::Bson::Document(d) => {
            format!("{{...{} fields}}", d.len())
        }
        bson::Bson::Binary(b) => {
            let hex: String = b.bytes.iter().map(|byte| format!("{:02x}", byte)).collect();
            format!("0x{}", hex)
        }
        bson::Bson::Array(arr) => {
            let items: Vec<String> = arr.iter().map(bson_to_string).collect();
            format!("[{}]", items.join(", "))
        }
        bson::Bson::ObjectId(oid) => format!("{}", oid),
        bson::Bson::Boolean(b) => {
            if *b {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        other => format!("{}", other),
    }
}

pub fn read_header<R: Read>(r: &mut R) -> anyhow::Result<(String, u32)> {
    let mut header = String::new();
    let mut buf = [0u8; 1];
    loop {
        r.read_exact(&mut buf)?;
        if buf[0] == b'\n' {
            break;
        }
        header.push(buf[0] as char);
    }
    let parts: Vec<&str> = header.split_whitespace().collect();
    let proto = parts.first().unwrap_or(&"").to_string();
    let pid = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok((proto, pid))
}

pub fn read_bson_frame<R: Read>(r: &mut R) -> anyhow::Result<Option<bson::Document>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len < 5 || len > 20 * 1024 * 1024 {
        return Err(anyhow::anyhow!("invalid BSON frame length: {}", len));
    }
    let mut body = vec![0u8; len - 4];
    r.read_exact(&mut body)?;
    let mut full = Vec::with_capacity(len);
    full.extend_from_slice(&len_buf);
    full.extend_from_slice(&body);
    let doc = bson::Document::from_reader(Cursor::new(&full))?;
    Ok(Some(doc))
}

pub fn process_doc(reg: &mut SchemaRegistry, doc: &bson::Document) -> Option<ApiCall> {
    let doc_type = doc.get_str("type").unwrap_or("");
    if doc_type == "info" {
        match reg.ingest_info(doc) {
            Ok(()) => {}
            Err(e) => println!("couldnt ingest info: {}", e),
        };
        return None;
    }
    let index = doc.get_i32("I").ok()?;
    let schema = reg.get(index)?;
    let thread_id = doc.get_i32("T").unwrap_or(0) as u32;
    let time = doc.get_i32("t").unwrap_or(0);
    let return_value = doc
        .get("args")
        .and_then(|v| {
            let args = v.as_array().unwrap();
            Some(match args[1].as_i32() {
                Some(i) => i as i64,
                None => args[1].as_i64().unwrap(),
            })
        })
        .unwrap_or_default();
    let is_success = doc
        .get("args")
        .and_then(|v| {
            let args = v.as_array().unwrap();
            Some(args[0].as_i32().unwrap() == 1)
        })
        .unwrap_or(true);
    let status = if is_success { "success" } else { "failed" }.to_string();

    let values: Vec<String> = doc.get("args").unwrap().as_array().unwrap()[2..]
        .to_vec()
        .iter()
        .filter_map(|b| Some(bson_to_string(b)))
        .collect();

    let arguments: Vec<(String, String)> = schema.arg_names[2..]
        .iter()
        .zip(&values)
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect();

    let ret = Some(ApiCall {
        api: schema.name.clone(),
        category: schema.category.clone(),
        status,
        return_value,
        thread_id,
        time,
        arguments,
    });
    println!(
        "\n\n\n\n{}\n\n{:?}\n\nargvalues: {:?}\n\n",
        doc,
        ret.as_ref().unwrap(),
        values
    );
    ret
}
