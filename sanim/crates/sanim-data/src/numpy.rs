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
    let parsed = parse(bytes, limits)?;
    if matches!(element, Type::List(_, _) | Type::Fraction) {
        let mut shape = parsed.shape.clone();
        let mut scalar = element;
        let mut lengths = vec![];
        while let Type::List(inner, length) = scalar {
            lengths.push(*length);
            scalar = inner;
        }
        if sanim_core::math::operations::scalar_unit(scalar) != Some(Unit::Number) {
            return Err(
                "NPY numeric Lists require dimensionless Number or Fraction elements".into(),
            );
        }
        let split = shape
            .len()
            .checked_sub(lengths.len())
            .ok_or("NPY shape has fewer dimensions than the nested List schema")?;
        let dimensions = shape.split_off(split);
        for (length, actual) in lengths.iter().zip(&dimensions) {
            if !length.allows(*actual) {
                return Err(format!(
                    "NPY dimension {actual} does not match nested List schema {element}"
                ));
            }
        }
        if !array && !shape.is_empty() {
            return Err(format!(
                "NPY shape has extra array dimensions; use Array<{element}>"
            ));
        }
        let count = shape
            .iter()
            .try_fold(1usize, |count, axis| count.checked_mul(*axis))
            .ok_or("NPY shape overflow")?;
        let mut level = count;
        let mut nested_values = count;
        for dimension in &dimensions {
            level = level
                .checked_mul(*dimension)
                .ok_or("NPY nested value count overflow")?;
            nested_values = nested_values
                .checked_add(level)
                .ok_or("NPY nested value count overflow")?;
        }
        if nested_values > limits.max_values {
            return Err("NPY nested List value limit exceeded".into());
        }
        if dimensions.len() > limits.max_depth {
            return Err("NPY nested List depth limit exceeded".into());
        }
        let mut values = parsed.values(scalar)?;
        let converted = (0..count)
            .map(|_| numeric_list(element, &dimensions, &mut values))
            .collect::<Result<Vec<_>, _>>()?;
        return if array {
            Ok(Value::Array {
                element_type: element.clone().into(),
                shape,
                values: converted.into(),
            })
        } else {
            converted
                .into_iter()
                .next()
                .ok_or("NPY scalar value is missing".into())
        };
    }
    let Type::Array(inner) = ty else {
        return Err("NPY requires Array<Number>, Array<Bool>, or nested numeric Lists".into());
    };
    let values = parsed.values(inner)?.collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array {
        element_type: inner.clone().into(),
        shape: parsed.shape,
        values: values.into(),
    })
}

fn numeric_list(
    ty: &Type,
    dimensions: &[usize],
    values: &mut impl Iterator<Item = Result<Value, String>>,
) -> Result<Value, String> {
    if let Type::List(inner, _) = ty {
        return Ok(Value::List(sanim_core::List::typed(
            inner.as_ref().clone(),
            (0..dimensions[0])
                .map(|_| numeric_list(inner, &dimensions[1..], values))
                .collect::<Result<_, _>>()?,
        )));
    }
    values.next().ok_or("NPY numeric component is missing")?
}

struct Parsed<'a> {
    shape: Vec<usize>,
    payload: &'a [u8],
    kind: u8,
    width: usize,
    little: bool,
}
fn parse(bytes: &[u8], limits: Limits) -> Result<Parsed<'_>, String> {
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
    Ok(Parsed {
        shape,
        payload,
        kind,
        width,
        little,
    })
}
impl Parsed<'_> {
    fn values<'a>(
        &'a self,
        inner: &'a Type,
    ) -> Result<impl Iterator<Item = Result<Value, String>> + 'a, String> {
        if self.kind == b'b' && *inner != Type::Bool {
            return Err("NPY boolean dtype requires Bool elements".into());
        }
        if self.kind != b'b'
            && sanim_core::math::operations::scalar_unit(inner) != Some(Unit::Number)
            && *inner != Type::Data
        {
            return Err(
                "numeric NPY dtype requires dimensionless Number or Fraction elements".into(),
            );
        }
        Ok(self
            .payload
            .chunks_exact(self.width)
            .map(move |chunk| self.scalar(chunk, inner)))
    }
    fn scalar(&self, chunk: &[u8], inner: &Type) -> Result<Value, String> {
        let Self {
            kind,
            width,
            little,
            ..
        } = self;
        let (kind, width, little) = (*kind, *width, *little);
        if kind == b'b' {
            if chunk[0] > 1 {
                return Err("invalid NPY boolean".into());
            }
            return Ok(Value::Bool(chunk[0] != 0));
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
        if *inner == Type::Data {
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
            return Ok(Value::Data(number.into()));
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
        let decoded = if *inner == Type::Fraction {
            Value::Fraction(value)
        } else {
            Value::Number(Number {
                value,
                unit: Unit::Number,
            })
        };
        sanim_core::validate_value(inner, &decoded)?;
        Ok(decoded)
    }
}
