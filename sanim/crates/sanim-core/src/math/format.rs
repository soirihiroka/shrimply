//! Bounded Python-style numeric and string formatting for labels/interpolation.
use crate::{Fraction, Type, Value};
use fraction::Sign;

pub const MAX_WIDTH: usize = 16_384;
pub const MAX_PRECISION: usize = 1_024;
pub const MAX_OUTPUT_BYTES: usize = 1_048_576;

#[derive(Default)]
struct Spec {
    fill: Option<char>,
    align: Option<char>,
    sign: Option<char>,
    normalize_zero: bool,
    alternate: bool,
    zero: bool,
    width: usize,
    grouping: Option<char>,
    precision: Option<usize>,
    kind: Option<char>,
}

impl Spec {
    fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_WIDTH {
            return Err("format specification is too long".into());
        }
        let chars = text.chars().collect::<Vec<_>>();
        let mut spec = Self::default();
        let mut at = 0;
        if chars.get(1).is_some_and(|c| "<>=^".contains(*c)) {
            spec.fill = Some(chars[0]);
            spec.align = Some(chars[1]);
            at = 2;
        } else if chars.first().is_some_and(|c| "<>=^".contains(*c)) {
            spec.align = Some(chars[0]);
            at = 1;
        }
        if chars.get(at).is_some_and(|c| "+- ".contains(*c)) {
            spec.sign = Some(chars[at]);
            at += 1;
        }
        if chars.get(at) == Some(&'z') {
            spec.normalize_zero = true;
            at += 1;
        }
        if chars.get(at) == Some(&'#') {
            spec.alternate = true;
            at += 1;
        }
        if chars.get(at) == Some(&'0') {
            spec.zero = true;
            at += 1;
        }
        spec.width = digits(&chars, &mut at, MAX_WIDTH)?.unwrap_or(0);
        if chars.get(at).is_some_and(|c| ",_".contains(*c)) {
            spec.grouping = Some(chars[at]);
            at += 1;
        }
        if chars.get(at) == Some(&'.') {
            at += 1;
            spec.precision = Some(
                digits(&chars, &mut at, MAX_PRECISION)?
                    .ok_or("format precision requires digits after '.'")?,
            );
        }
        if let Some(kind) = chars.get(at) {
            if !"bcdoxXeEfFgGs%".contains(*kind) {
                return Err(format!(
                    "unsupported format type '{kind}'; use b/o/x/X/d/c, f/e/g/%, or s"
                ));
            }
            spec.kind = Some(*kind);
            at += 1;
        }
        if at != chars.len() {
            return Err(format!("invalid format specification '{text}'"));
        }
        Ok(spec)
    }

    fn string_options(&self) -> Result<(), String> {
        if self.sign.is_some()
            || self.alternate
            || self.normalize_zero
            || self.grouping.is_some()
            || self.align == Some('=')
            || self.kind.is_some_and(|kind| kind != 's')
        {
            return Err("numeric format options cannot be applied to String; use !s/!r with string alignment and precision".into());
        }
        Ok(())
    }

    fn numeric_options(&self) -> Result<(), String> {
        if self.kind == Some('s') {
            return Err("String format 's' cannot be applied to Number; use !s".into());
        }
        if self.kind.is_some_and(|kind| "bcdoxX".contains(kind))
            && (self.precision.is_some() || self.normalize_zero)
        {
            return Err("integer formats require an integral value without precision or z; use f/e/g for fractions".into());
        }
        if self.grouping == Some(',') && self.kind.is_some_and(|kind| "boxX".contains(kind)) {
            return Err("comma grouping is only supported for decimal numbers; use '_' with binary/octal/hex".into());
        }
        if self.kind == Some('c')
            && (self.sign.is_some() || self.alternate || self.grouping.is_some())
        {
            return Err("character format 'c' requires an unsigned Unicode scalar without sign, alternate, or grouping options".into());
        }
        Ok(())
    }

    fn known_type(&self, ty: &Type) -> Result<(), String> {
        match ty {
            Type::Unknown | Type::Variable(_) | Type::BoundedVariable(..) | Type::Data => Ok(()),
            Type::Union(alternatives) => {
                let mut error = None;
                for ty in alternatives {
                    match self.known_type(ty) {
                        Ok(()) => return Ok(()),
                        Err(message) => error = Some(message),
                    }
                }
                error.map_or(Ok(()), Err)
            }
            Type::Numeric(_) | Type::RefinedNumeric(_) | Type::Fraction | Type::Bool => {
                self.numeric_options()
            }
            _ => self.string_options(),
        }
    }
}

/// Validate literal format syntax and statically known value types without
/// evaluating the value, opening resources, or allocating formatted output.
/// A union is rejected only if every alternative is incompatible.
pub fn validate(format: &str, ty: &Type, conversion: Option<&str>) -> Result<(), String> {
    let spec = Spec::parse(format)?;
    conversion_options(conversion)?;
    if conversion.is_some() {
        spec.string_options()
    } else {
        spec.known_type(ty)
    }
}

fn conversion_options(conversion: Option<&str>) -> Result<(), String> {
    if let Some(conversion) = conversion
        && !matches!(conversion, "s" | "r" | "a")
    {
        return Err(format!(
            "unknown conversion '!{conversion}'; use !s, !r, or !a"
        ));
    }
    Ok(())
}

fn digits(chars: &[char], at: &mut usize, limit: usize) -> Result<Option<usize>, String> {
    let start = *at;
    let mut value = 0usize;
    while let Some(c) = chars.get(*at).filter(|c| c.is_ascii_digit()) {
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(*c as usize - '0' as usize))
            .filter(|value| *value <= limit)
            .ok_or_else(|| format!("format width/precision exceeds the supported limit {limit}"))?;
        *at += 1;
    }
    Ok((*at > start).then_some(value))
}

pub fn value(value: &Value, format: &str, conversion: Option<&str>) -> Result<String, String> {
    let spec = Spec::parse(format)?;
    conversion_options(conversion)?;
    if let Some(conversion) = conversion {
        let text = match conversion {
            "s" => plain(value)?,
            "r" | "a" => match value {
                Value::String(text) => quote(text, conversion == "a")?,
                _ => {
                    let text = plain(value)?;
                    if conversion == "a" {
                        let mut out = String::new();
                        for c in text.chars() {
                            if c.is_ascii() {
                                out.push(c);
                            } else {
                                unicode_escape(&mut out, c);
                            }
                            if out.len() > MAX_OUTPUT_BYTES {
                                return Err("ascii conversion exceeds the output byte limit".into());
                            }
                        }
                        out
                    } else {
                        text
                    }
                }
            },
            _ => unreachable!("validated conversion"),
        };
        return string(&text, &spec);
    }
    match value {
        Value::Number(number) => number_value(number.value, &spec),
        Value::Fraction(number) => number_value(*number, &spec),
        Value::Bool(boolean) if !format.is_empty() => {
            number_value(Fraction::from(u64::from(*boolean)), &spec)
        }
        Value::String(text) => string(text, &spec),
        _ => string(&plain(value)?, &spec),
    }
}

fn plain(value: &Value) -> Result<String, String> {
    match value {
        Value::Template(template) => template.render(MAX_OUTPUT_BYTES),
        Value::String(text) => {
            if text.len() > MAX_OUTPUT_BYTES {
                return Err("formatted string exceeds the output byte limit".into());
            }
            Ok(text.to_string())
        }
        Value::Number(number) => number_value(number.value, &Spec::default()),
        Value::Fraction(number) => number_value(*number, &Spec::default()),
        Value::Bool(boolean) => Ok(boolean.to_string()),
        other => {
            let mut output = BoundedOutput(Vec::new());
            serde_json::to_writer(&mut output, other).map_err(|error| error.to_string())?;
            String::from_utf8(output.0).map_err(|error| error.to_string())
        }
    }
}

struct BoundedOutput(Vec<u8>);
impl std::io::Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_OUTPUT_BYTES {
            return Err(std::io::Error::other(
                "formatted value exceeds the output byte limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn quote(text: &str, ascii: bool) -> Result<String, String> {
    let delimiter = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(delimiter);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == delimiter => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() || ascii && !c.is_ascii() => {
                unicode_escape(&mut out, c);
            }
            c => out.push(c),
        }
        if out.len() >= MAX_OUTPUT_BYTES {
            return Err("repr exceeds the output byte limit".into());
        }
    }
    out.push(delimiter);
    Ok(out)
}

fn unicode_escape(out: &mut String, c: char) {
    use std::fmt::Write;
    let n = c as u32;
    if n <= 0xff {
        write!(out, "\\x{n:02x}").expect("write String");
    } else if n <= 0xffff {
        write!(out, "\\u{n:04x}").expect("write String");
    } else {
        write!(out, "\\U{n:08x}").expect("write String");
    }
}

fn string(text: &str, spec: &Spec) -> Result<String, String> {
    spec.string_options()?;
    let text = if let Some(precision) = spec.precision {
        text.chars().take(precision).collect::<String>()
    } else {
        if text.len() > MAX_OUTPUT_BYTES {
            return Err("formatted string exceeds the output byte limit".into());
        }
        text.to_owned()
    };
    Ok(pad(&text, 0, spec, false))
}

fn number_value(value: Fraction, spec: &Spec) -> Result<String, String> {
    spec.numeric_options()?;
    let Fraction::Rational(sign, ratio) = value else {
        return Err("cannot format a non-finite number".into());
    };
    let mut negative = sign == Sign::Minus;
    let integral = *ratio.denom() == 1;
    let integer_kind = spec.kind.is_some_and(|kind| "bcdoxX".contains(kind));
    if integer_kind && !integral {
        return Err("integer formats require an integral value without precision or z; use f/e/g for fractions".into());
    }
    let mut prefix = String::new();
    let mut body = if integer_kind || spec.kind.is_none() && integral && spec.precision.is_none() {
        let integer = *ratio.numer();
        let kind = spec.kind.unwrap_or('d');
        match kind {
            'b' => {
                if spec.alternate {
                    prefix.push_str("0b");
                }
                format!("{integer:b}")
            }
            'o' => {
                if spec.alternate {
                    prefix.push_str("0o");
                }
                format!("{integer:o}")
            }
            'x' => {
                if spec.alternate {
                    prefix.push_str("0x");
                }
                format!("{integer:x}")
            }
            'X' => {
                if spec.alternate {
                    prefix.push_str("0X");
                }
                format!("{integer:X}")
            }
            'c' => {
                if negative {
                    return Err("character format 'c' requires an unsigned Unicode scalar without sign, alternate, or grouping options".into());
                }
                return Ok(pad(
                    &char::from_u32(
                        u32::try_from(integer)
                            .map_err(|_| "character code exceeds Unicode range")?,
                    )
                    .ok_or("character code is not a Unicode scalar")?
                    .to_string(),
                    0,
                    spec,
                    true,
                ));
            }
            _ => integer.to_string(),
        }
    } else {
        let float = super::to_f64(value)?.abs();
        let precision = spec.precision.unwrap_or(6);
        let kind = spec.kind.unwrap_or('\0');
        let mut result = match kind {
            'f' | 'F' => format!("{float:.precision$}"),
            '%' => format!("{:.precision$}%", float * 100.),
            'e' | 'E' => exponent(&format!("{float:.precision$e}"), kind == 'E')?,
            'g' | 'G' => general(float, precision.max(1), spec.alternate, kind == 'G', false)?,
            '\0' if spec.precision.is_some() => {
                general(float, precision.max(1), spec.alternate, false, true)?
            }
            '\0' if float != 0. && !(1e-4..1e16).contains(&float) => {
                exponent(&format!("{float:e}"), false)?
            }
            '\0' => float.to_string(),
            _ => return Err("unsupported numeric format".into()),
        };
        if spec.alternate && !result.contains('.') {
            let at = result.find(['e', 'E', '%']).unwrap_or(result.len());
            result.insert(at, '.');
        }
        if spec.normalize_zero {
            let mantissa = result.split(['e', 'E', '%']).next().unwrap_or(&result);
            if mantissa.chars().all(|c| c == '0' || c == '.') {
                negative = false;
            }
        }
        result
    };
    let sign = if negative {
        "-"
    } else {
        match spec.sign {
            Some('+') => "+",
            Some(' ') => " ",
            _ => "",
        }
    };
    if let Some(separator) = spec.grouping {
        let group = if integer_kind && spec.kind.is_some_and(|kind| "boxX".contains(kind)) {
            4
        } else {
            3
        };
        if spec.zero && spec.fill.is_none() && spec.align.is_none_or(|align| align == '=') {
            let end = if integer_kind {
                body.len()
            } else {
                body.find(['.', 'e', 'E', '%']).unwrap_or(body.len())
            };
            let target = spec
                .width
                .saturating_sub(sign.len() + prefix.len() + body.len() - end);
            let (mut low, mut high) = (end, target.max(end));
            while low < high {
                let middle = low + (high - low) / 2;
                if middle + middle.saturating_sub(1) / group < target {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            body.insert_str(0, &"0".repeat(low - end));
        }
        body = grouped(&body, separator, group, integer_kind);
    }
    let head = sign.len() + prefix.len();
    Ok(pad(&format!("{sign}{prefix}{body}"), head, spec, true))
}

fn grouped(body: &str, separator: char, group: usize, integer: bool) -> String {
    let end = if integer {
        body.len()
    } else {
        body.find(['.', 'e', 'E', '%']).unwrap_or(body.len())
    };
    let mut out = String::with_capacity(body.len() + end / group);
    for (index, c) in body[..end].chars().enumerate() {
        if index > 0 && (end - index).is_multiple_of(group) {
            out.push(separator);
        }
        out.push(c);
    }
    out.push_str(&body[end..]);
    out
}

fn pad(body: &str, head: usize, spec: &Spec, numeric: bool) -> String {
    let padding = spec.width.saturating_sub(body.chars().count());
    if padding == 0 {
        return body.into();
    }
    let fill = spec.fill.unwrap_or(if spec.zero { '0' } else { ' ' });
    let align = spec.align.unwrap_or(if numeric {
        if spec.zero { '=' } else { '>' }
    } else {
        '<'
    });
    let repeat = |count| std::iter::repeat_n(fill, count).collect::<String>();
    match align {
        '<' => format!("{body}{}", repeat(padding)),
        '^' => format!(
            "{}{body}{}",
            repeat(padding / 2),
            repeat(padding - padding / 2)
        ),
        '=' => format!("{}{}{}", &body[..head], repeat(padding), &body[head..]),
        _ => format!("{}{body}", repeat(padding)),
    }
}

fn exponent(body: &str, uppercase: bool) -> Result<String, String> {
    let (mantissa, exp) = body.split_once('e').ok_or("invalid scientific format")?;
    let exp = exp.parse::<i32>().map_err(|error| error.to_string())?;
    Ok(format!(
        "{mantissa}{}{exp:+03}",
        if uppercase { 'E' } else { 'e' }
    ))
}

fn general(
    value: f64,
    precision: usize,
    alternate: bool,
    uppercase: bool,
    default_type: bool,
) -> Result<String, String> {
    let scientific = format!("{value:.p$e}", p = precision - 1);
    let (mantissa, exp) = scientific.split_once('e').ok_or("invalid general format")?;
    let exp = exp.parse::<i32>().map_err(|error| error.to_string())?;
    if exp < -4 || exp >= precision.saturating_sub(usize::from(default_type)) as i32 {
        let mantissa = if alternate {
            mantissa
        } else {
            mantissa.trim_end_matches('0').trim_end_matches('.')
        };
        exponent(&format!("{mantissa}e{exp}"), uppercase)
    } else {
        let places = (precision as i32 - exp - 1).max(0) as usize;
        let fixed = format!("{value:.places$}");
        let mut result: String = if !alternate && fixed.contains('.') {
            fixed.trim_end_matches('0').trim_end_matches('.').into()
        } else {
            fixed
        };
        if default_type && !result.contains('.') {
            result.push_str(".0");
        }
        Ok(result)
    }
}
