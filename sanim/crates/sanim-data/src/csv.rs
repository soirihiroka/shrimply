use crate::{Limits, validate};
use sanim_core::{Type, Value};
use serde_json::Value as Json;

pub fn decode(bytes: &[u8], ty: &Type, limits: Limits) -> Result<Value, String> {
    let Type::Table(row) = ty else {
        return Err("CSV requires Table<Row>".into());
    };
    let Type::Record(fields) = row.as_ref() else {
        return Err("CSV row type must be a record".into());
    };
    let text = std::str::from_utf8(bytes)
        .map_err(|e| format!("CSV UTF-8: {e}"))?
        .trim_start_matches('\u{feff}');
    let rows = parse(text, limits.max_values)?;
    let Some(header) = rows.first() else {
        return Err("CSV has no header".into());
    };
    let mut names = std::collections::BTreeSet::new();
    for name in header {
        if !names.insert(name) {
            return Err(format!("duplicate CSV column '{name}'"));
        }
        if !fields.contains_key(name) {
            return Err(format!("unknown CSV column '{name}'"));
        }
    }
    for name in fields.keys() {
        if !names.contains(name) {
            return Err(format!("missing CSV column '{name}'"));
        }
    }
    let mut out = vec![];
    for (index, row) in rows.iter().enumerate().skip(1) {
        if row.len() != header.len() {
            return Err(format!(
                "CSV row {} has {} fields, expected {}",
                index + 1,
                row.len(),
                header.len()
            ));
        }
        let mut object = serde_json::Map::new();
        for (name, text) in header.iter().zip(row) {
            let ty = &fields[name];
            let inner = if let Type::Option(inner) = ty {
                inner.as_ref()
            } else {
                ty
            };
            let value = if matches!(ty, Type::Option(_)) && text.is_empty() {
                Json::Null
            } else if *inner == Type::Bool {
                match text.as_str() {
                    "true" => Json::Bool(true),
                    "false" => Json::Bool(false),
                    _ => {
                        return Err(format!(
                            "CSV row {} column {name}: expected true or false",
                            index + 1
                        ));
                    }
                }
            } else {
                Json::String(text.clone())
            };
            object.insert(name.clone(), value);
        }
        out.push(Json::Object(object));
    }
    validate(&Json::Array(out), ty, limits)
}

fn parse(text: &str, limit: usize) -> Result<Vec<Vec<String>>, String> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut closed = false;
    let mut count = 0usize;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !closed => quoted = true,
            ',' | '\n' | '\r' => {
                row.push(std::mem::take(&mut field));
                closed = false;
                count += 1;
                if count > limit {
                    return Err("CSV field limit exceeded".into());
                }
                if c != ',' {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    rows.push(std::mem::take(&mut row));
                }
            }
            _ if closed => return Err("characters after closing CSV quote".into()),
            '"' => return Err("quote in unquoted CSV field".into()),
            _ => field.push(c),
        }
    }
    if quoted {
        return Err("unterminated CSV quoted field".into());
    }
    if !row.is_empty() || !field.is_empty() || closed {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}
