use crate::Limits;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

// Apply budgets while deserializing, including every expanded alias value.
struct Seed<'a> {
    remaining: &'a mut usize,
    bytes: &'a mut usize,
    depth: usize,
    max_depth: usize,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        if self.depth > self.max_depth {
            return Err(de::Error::custom("YAML nesting limit exceeded"));
        }
        *self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| de::Error::custom("YAML value limit exceeded"))?;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("YAML data with string mapping keys")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("YAML numbers must be finite"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        *self.bytes = self
            .bytes
            .checked_sub(v.len())
            .ok_or_else(|| E::custom("YAML expanded byte budget exceeded"))?;
        Ok(Value::String(v.into()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        *self.bytes = self
            .bytes
            .checked_sub(v.len())
            .ok_or_else(|| E::custom("YAML expanded byte budget exceeded"))?;
        Ok(Value::String(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = vec![];
        while let Some(value) = seq.next_element_seed(Seed {
            remaining: self.remaining,
            bytes: self.bytes,
            depth: self.depth + 1,
            max_depth: self.max_depth,
        })? {
            items.push(value);
        }
        Ok(Value::Array(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut fields = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            *self.bytes = self
                .bytes
                .checked_sub(key.len())
                .ok_or_else(|| de::Error::custom("YAML expanded byte budget exceeded"))?;
            if fields.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate YAML key '{key}'")));
            }
            let value = map.next_value_seed(Seed {
                remaining: self.remaining,
                bytes: self.bytes,
                depth: self.depth + 1,
                max_depth: self.max_depth,
            })?;
            fields.insert(key, value);
        }
        Ok(Value::Object(fields))
    }
}
pub fn parse(bytes: &[u8], limits: Limits) -> Result<Value, String> {
    let mut documents = serde_yaml_ng::Deserializer::from_slice(bytes);
    let document = documents.next().ok_or("YAML document is empty")?;
    let mut remaining = limits.max_values;
    let mut bytes = limits.max_bytes;
    let value = Seed {
        remaining: &mut remaining,
        bytes: &mut bytes,
        depth: 0,
        max_depth: limits.max_depth,
    }
    .deserialize(document)
    .map_err(|e| format!("YAML: {e}"))?;
    if documents.next().is_some() {
        return Err("YAML requires exactly one document".into());
    }
    Ok(value)
}
