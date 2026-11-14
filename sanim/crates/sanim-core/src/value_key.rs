//! Cache dependencies use immutable collection content proofs, without changing Value's wire format.
use crate::{Type, Value};
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};

pub struct ValueKey<'a>(pub &'a Value);
struct Fields<'a>(&'a imbl::OrdMap<String, Value>);
struct Items<'a>(&'a [Value]);
impl Serialize for Fields<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in self.0 {
            map.serialize_entry(name, &ValueKey(value))?;
        }
        map.end()
    }
}
impl Serialize for Items<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            sequence.serialize_element(&ValueKey(value))?;
        }
        sequence.end()
    }
}
impl Serialize for ValueKey<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        enum Key<'a> {
            Literal(&'a Value),
            List {
                element_type: &'a Type,
                content: &'a str,
            },
            Array {
                element_type: &'a Type,
                shape: &'a [usize],
                content: &'a str,
            },
            Record(Fields<'a>),
            Some(ValueKey<'a>),
            Variant {
                ty: &'a Type,
                tag: &'a str,
                fields: Items<'a>,
            },
        }
        match self.0 {
            Value::List(list) => Key::List {
                element_type: &list.element_type,
                content: list.values.fingerprint(),
            },
            Value::Array {
                element_type,
                shape,
                values,
            } => Key::Array {
                element_type,
                shape,
                content: values.fingerprint(),
            },
            Value::Record(fields) => Key::Record(Fields(fields)),
            Value::Some(value) => Key::Some(ValueKey(value)),
            Value::Variant { ty, tag, fields } => Key::Variant {
                ty,
                tag,
                fields: Items(fields),
            },
            value => Key::Literal(value),
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{List, Number, Unit};
    fn key(value: &Value) -> String {
        crate::digest(&serde_json::to_vec(&ValueKey(value)).unwrap())
    }
    #[test]
    fn keys_keep_schema_shape_and_mutation_while_sharing_payloads() {
        let value = Value::List(List::typed(
            Type::Numeric(Unit::Number),
            vec![Value::Number(Number::scalar(1))],
        ));
        assert_eq!(key(&value), key(&value.clone()));
        let Value::List(list) = value.clone() else {
            unreachable!()
        };
        let mut other = list.clone();
        other.values.set(0, Value::Number(Number::scalar(2)));
        assert_ne!(key(&value), key(&Value::List(other)));
        let typed = Value::List(List::from_values(
            std::sync::Arc::new(Type::Numeric(Unit::Length)),
            list.values.clone(),
        ));
        assert_ne!(key(&value), key(&typed));
        let array = Value::Array {
            element_type: list.element_type.clone(),
            shape: vec![1, 1],
            values: list.values.clone(),
        };
        assert_ne!(key(&value), key(&array));
        let other_shape = Value::Array {
            element_type: list.element_type,
            shape: vec![1],
            values: list.values,
        };
        assert_ne!(key(&array), key(&other_shape));
        let wrap = |value| {
            Value::Record(
                std::iter::once(("rows".to_string(), Value::Some(std::sync::Arc::new(value))))
                    .collect(),
            )
        };
        assert_ne!(key(&wrap(value)), key(&wrap(typed)));
    }
}
