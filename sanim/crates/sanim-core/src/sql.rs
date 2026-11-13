//! SQL contains trusted literal syntax, quoted identifiers, and bound values.
use crate::{Template, TemplatePart, Type, Value};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const PARAMETER_PREFIX: &str = "__sanim_hole_";
pub fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
pub fn identifier(name: &str) -> Result<Arc<str>, String> {
    if name.is_empty() || name.contains('\0') {
        return Err("SqlIdent requires a nonempty identifier without NUL characters".into());
    }
    Ok(name.into())
}
pub fn parameter_type(ty: &Type) -> bool {
    match ty {
        Type::Bool | Type::String | Type::Numeric(_) | Type::Fraction | Type::Data => true,
        Type::Option(inner) => **inner == Type::Unknown || parameter_type(inner),
        _ => false,
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sql {
    pub text: Arc<str>,
    pub parameters: Value,
}
impl Sql {
    pub fn compile(template: &Template, limit: usize, max_values: usize) -> Result<Self, String> {
        let mut text = String::new();
        let mut parameters = imbl::OrdMap::new();
        for part in template.parts.iter() {
            let rendered;
            let fragment = match part {
                TemplatePart::Text(literal) => {
                    if literal.contains(PARAMETER_PREFIX) {
                        return Err(format!(
                            "SQL literal uses reserved parameter prefix '{PARAMETER_PREFIX}'; interpolate values with {{value}} instead"
                        ));
                    }
                    literal.as_ref()
                }
                TemplatePart::Hole {
                    value,
                    format,
                    conversion,
                } => {
                    if format.is_some() || conversion.is_some() {
                        return Err("SQL interpolation does not accept formatting or !conversions; bind {value} directly, or format a String value before binding it".into());
                    }
                    rendered = match value {
                        Value::SqlIdent(name) => quote_identifier(name),
                        value if parameter_type(&value.ty()) => {
                            if parameters.len() >= max_values {
                                return Err("SQL interpolation parameter count exceeds the configured value budget".into());
                            }
                            let name = format!("{PARAMETER_PREFIX}{}", parameters.len());
                            parameters.insert(name.clone(), value.clone());
                            format!(":{name}")
                        }
                        value => {
                            return Err(format!(
                                "SQL interpolation requires a scalar value or SqlIdent(name), found {}; structured values must be projected to a scalar first",
                                value.ty()
                            ));
                        }
                    };
                    &rendered
                }
            };
            text.len()
                .checked_add(fragment.len())
                .filter(|size| *size <= limit)
                .ok_or("SQL template exceeds the configured byte budget")?;
            text.push_str(fragment);
        }
        Ok(Self {
            text: text.into(),
            parameters: Value::Record(parameters),
        })
    }
}
