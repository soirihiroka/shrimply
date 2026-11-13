use crate::Limits;
use sanim_core::{Fraction, Number, Type, Unit, Value};
use std::io::{Cursor, Read};

pub fn archive(bytes: &[u8], ty: &Type, limits: Limits) -> Result<Value, String> {
    let Type::Record(fields) = ty else {
        return Err("NPZ requires a record of Array<T> fields".into());
    };
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("NPZ: {e}"))?;
    if zip.len() != fields.len() {
        return Err("NPZ entry count does not match schema".into());
    }
    let mut output = std::collections::BTreeMap::new();
    let mut budget = limits.max_bytes;
    for (name, ty) in fields {
        let mut file = zip
            .by_name(&format!("{name}.npy"))
            .map_err(|e| format!("NPZ {name}: {e}"))?;
        let size = usize::try_from(file.size()).map_err(|_| "NPZ entry too large")?;
        budget = budget
            .checked_sub(size)
            .ok_or("NPZ expanded byte limit exceeded")?;
        let mut content = vec![];
        file.by_ref()
            .take((size as u64) + 1)
            .read_to_end(&mut content)
            .map_err(|e| e.to_string())?;
        if content.len() != size {
            return Err("NPZ entry size mismatch".into());
        }
        output.insert(name.clone(), decode(&content, ty, limits)?);
    }
    Ok(Value::Record(output.into_iter().collect()))
}

pub fn decode(bytes: &[u8], ty: &Type, limits: Limits) -> Result<Value, String> {
    let (element, array) = if let Type::Array(inner) = ty {
        (inner.as_ref(), true)
    } else {
        (ty, false)
    };
    if matches!(element, Type::Vector(..) | Type::Matrix4 | Type::Fraction) {
        let raw = decode_array(
            bytes,
            &Type::Array(Box::new(if *element == Type::Fraction {
                Type::Numeric(Unit::Number)
            } else {
                Type::Data
            })),
            limits,
        )?;
        let Value::Array {
            mut shape, values, ..
        } = raw
        else {
            return Err("invalid numeric array".into());
        };
        let width = match element {
            Type::Vector(n, unit) => {
                if unit.is_some_and(|u| u != Unit::Number) {
                    return Err(
                        "NPY does not encode units; vector elements must be dimensionless".into(),
                    );
                }
                if shape.pop() != Some(*n) {
                    return Err(format!("NPY final dimension must be {n} for {element}"));
                }
                *n
            }
            Type::Matrix4 => {
                if shape.len() < 2 || shape[shape.len() - 2..] != [4, 4] {
                    return Err("NPY Matrix4 values require trailing shape (4, 4)".into());
                }
                shape.truncate(shape.len() - 2);
                16
            }
            _ => 1,
        };
        if !array && !shape.is_empty() {
            return Err(format!(
                "NPY shape has extra array dimensions; use Array<{element}>"
            ));
        }
        let mut converted = vec![];
        let mut components = values.iter();
        for _ in 0..values.len() / width {
            let mut chunk = components.by_ref().take(width);
            let value = if *element == Type::Fraction {
                Value::Fraction(
                    chunk
                        .next()
                        .expect("validated NPY component count")
                        .number()?
                        .value,
                )
            } else {
                let numbers = chunk
                    .map(|v| match v {
                        Value::Data(v) => v
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .ok_or_else(|| "NPY component must be finite f64".to_string()),
                        _ => Err("invalid NPY numeric component".into()),
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                match element {
                    Type::Vector(..) => Value::from_vector(numbers, Unit::Number)?,
                    Type::Matrix4 => {
                        let mut m = [[0.0; 4]; 4];
                        for (i, v) in numbers.into_iter().enumerate() {
                            m[i / 4][i % 4] = v;
                        }
                        Value::Matrix4(Box::new(m))
                    }
                    _ => return Err("unsupported NPY math type".into()),
                }
            };
            converted.push(value);
        }
        return if array {
            Ok(Value::Array {
                element_type: element.clone().into(),
                shape,
                values: converted.into(),
            })
        } else {
            converted.pop().ok_or("NPY scalar value is missing".into())
        };
    }
    decode_array(bytes, ty, limits)
}

fn decode_array(bytes: &[u8], ty: &Type, limits: Limits) -> Result<Value, String> {
    let Type::Array(inner) = ty else {
        return Err("NPY requires Array<Number> or Array<Bool>".into());
    };
    if bytes.get(..6) != Some(b"\x93NUMPY") {
        return Err("invalid NPY magic".into());
    }
    let version = *bytes.get(6).ok_or("truncated NPY version")?;
    let (offset, size): (usize, usize) = match version {
        1 => (
            10,
            usize::from(u16::from_le_bytes(
                bytes
                    .get(8..10)
                    .ok_or("truncated NPY header")?
                    .try_into()
                    .map_err(|_| "NPY header")?,
            )),
        ),
        2 | 3 => (
            12,
            usize::try_from(u32::from_le_bytes(
                bytes
                    .get(8..12)
                    .ok_or("truncated NPY header")?
                    .try_into()
                    .map_err(|_| "NPY header")?,
            ))
            .map_err(|_| "NPY header too large")?,
        ),
        _ => return Err(format!("unsupported NPY version {version}")),
    };
    let end = offset.checked_add(size).ok_or("NPY header overflow")?;
    let header = std::str::from_utf8(bytes.get(offset..end).ok_or("truncated NPY header")?)
        .map_err(|_| "NPY header must be UTF-8")?;
    let field = |name: &str| -> Result<&str, String> {
        let (_, value) = header
            .split_once(&format!("'{name}'"))
            .or_else(|| header.split_once(&format!("\"{name}\"")))
            .ok_or_else(|| format!("missing NPY {name}"))?;
        Ok(value
            .trim_start()
            .strip_prefix(':')
            .ok_or("invalid NPY dictionary")?
            .trim_start())
    };
    if !field("fortran_order")?.starts_with("False") {
        return Err("Fortran-ordered NPY arrays are unsupported; use C order".into());
    }
    let descr = field("descr")?;
    let quote = descr.chars().next().ok_or("missing NPY dtype")?;
    if !matches!(quote, '\'' | '"') {
        return Err("structured and object NPY dtypes are unsupported".into());
    }
    let dtype = descr[1..].split(quote).next().ok_or("invalid NPY dtype")?;
    let dims = field("shape")?
        .strip_prefix('(')
        .ok_or("invalid NPY shape")?
        .split(')')
        .next()
        .ok_or("invalid NPY shape")?;
    let shape = dims
        .split(',')
        .filter(|v| !v.trim().is_empty())
        .map(|v| {
            v.trim()
                .parse::<usize>()
                .map_err(|_| "invalid NPY dimension".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let count = shape
        .iter()
        .try_fold(1usize, |n, d| n.checked_mul(*d))
        .ok_or("NPY shape overflow")?;
    if count > limits.max_values {
        return Err("NPY value limit exceeded".into());
    }
    let dtype = dtype.as_bytes();
    if dtype.len() < 3 {
        return Err("invalid NPY dtype".into());
    }
    let little = match dtype[0] {
        b'<' | b'|' => true,
        b'>' => false,
        b'=' => cfg!(target_endian = "little"),
        _ => return Err("invalid NPY byte order".into()),
    };
    let kind = dtype[1];
    let width = std::str::from_utf8(&dtype[2..])
        .map_err(|_| "invalid dtype")?
        .parse::<usize>()
        .map_err(|_| "invalid dtype size")?;
    if !matches!(
        (kind, width),
        (b'i' | b'u', 1 | 2 | 4 | 8) | (b'f', 4 | 8) | (b'b', 1)
    ) {
        return Err("unsupported NPY dtype (numeric and boolean scalars only)".into());
    }
    let expected = count.checked_mul(width).ok_or("NPY payload overflow")?;
    let payload = bytes.get(end..).ok_or("truncated NPY payload")?;
    if payload.len() != expected {
        return Err("NPY payload length does not match shape".into());
    }
    let mut values = Vec::with_capacity(count);
    for chunk in payload.chunks_exact(width) {
        if kind == b'b' {
            if **inner != Type::Bool {
                return Err("NPY boolean dtype requires Array<Bool>".into());
            }
            if chunk[0] > 1 {
                return Err("invalid NPY boolean".into());
            }
            values.push(Value::Bool(chunk[0] != 0));
            continue;
        }
        if **inner != Type::Numeric(Unit::Number) && **inner != Type::Data {
            return Err("numeric NPY dtype requires Array<Number>".into());
        }
        let mut padded = [0u8; 8];
        if little {
            padded[..width].copy_from_slice(chunk);
        } else {
            for (i, v) in chunk.iter().rev().enumerate() {
                padded[i] = *v;
            }
        }
        let bits = u64::from_le_bytes(padded);
        if **inner == Type::Data {
            let number = match kind {
                b'u' => serde_json::Value::from(bits),
                b'i' => {
                    let shift = 64 - width * 8;
                    serde_json::Value::from(((bits << shift) as i64) >> shift)
                }
                b'f' => {
                    let number = if width == 4 {
                        f32::from_bits(bits as u32) as f64
                    } else {
                        f64::from_bits(bits)
                    };
                    serde_json::Number::from_f64(number)
                        .map(serde_json::Value::Number)
                        .ok_or("NPY contains non-finite value")?
                }
                _ => return Err("unsupported NPY math component".into()),
            };
            values.push(Value::Data(number.into()));
            continue;
        }
        let value = match kind {
            b'u' => Fraction::from(bits),
            b'i' => {
                let shift = 64 - width * 8;
                Fraction::from(((bits << shift) as i64) >> shift)
            }
            b'f' => {
                let value = if width == 4 {
                    f32::from_bits(bits as u32) as f64
                } else {
                    f64::from_bits(bits)
                };
                if !value.is_finite() {
                    return Err("NPY contains non-finite value".into());
                }
                sanim_core::math::from_f64(value)?
            }
            _ => return Err("unsupported NPY kind".into()),
        };
        values.push(Value::Number(Number {
            value,
            unit: Unit::Number,
        }));
    }
    Ok(Value::Array {
        element_type: inner.clone().into(),
        shape,
        values: values.into(),
    })
}
