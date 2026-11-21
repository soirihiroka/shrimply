use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut unique = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !unique.iter().any(|existing| existing == tag) {
            unique.push(tag.to_owned());
        }
    }
    unique
}

pub fn deserialize_tags<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    Vec::<String>::deserialize(deserializer).map(|tags| normalize_tags(&tags))
}

pub fn serialize_tags<S: Serializer>(tags: &[String], serializer: S) -> Result<S::Ok, S::Error> {
    normalize_tags(tags).serialize(serializer)
}

pub enum TagEdit<'a> {
    Add(&'a str),
    Remove(&'a str),
}

pub fn edit_tags(tags: &[String], edit: TagEdit<'_>) -> Vec<String> {
    let mut next = tags.to_vec();
    match edit {
        TagEdit::Add(tag) => next.push(tag.to_owned()),
        TagEdit::Remove(tag) => next.retain(|current| current != tag),
    }
    normalize_tags(&next)
}
