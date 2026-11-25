use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use super::{Project, Time};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectChanges {
    changes: Vec<Change>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Change {
    path: Vec<Segment>,
    before: Field,
    after: Field,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Segment {
    Field(String),
    Item(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Field {
    Missing,
    Present(Value),
}

#[derive(Debug)]
pub enum ChangeError {
    Conflict(String),
    Invalid(String),
}

impl std::fmt::Display for ChangeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict(path) => write!(formatter, "project changed at {path}"),
            Self::Invalid(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for ChangeError {}

impl ProjectChanges {
    /// Capture document changes with prior values as merge preconditions.
    pub fn between(before: &Project, after: &Project) -> Result<Self, serde_json::Error> {
        let mut changes = Vec::new();
        let before = shared_value(before)?;
        let after = shared_value(after)?;
        collect(&mut Vec::new(), Some(&before), Some(&after), &mut changes);
        Ok(Self { changes })
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn reverse(mut self) -> Self {
        self.changes.reverse();
        for change in &mut self.changes {
            std::mem::swap(&mut change.before, &mut change.after);
        }
        self
    }

    /// Apply the changes atomically if their prior values still match.
    pub fn apply(&self, project: &Project) -> Result<Project, ChangeError> {
        self.apply_inner(project, false)
    }

    /// Check that an unacknowledged submission is already reflected in the document.
    pub fn confirm(&self, project: &Project) -> Result<Project, ChangeError> {
        self.apply_inner(project, true)
    }

    fn apply_inner(&self, project: &Project, retry: bool) -> Result<Project, ChangeError> {
        let mut candidate =
            shared_value(project).map_err(|error| ChangeError::Invalid(error.to_string()))?;
        for change in &self.changes {
            change.apply(&mut candidate, retry)?;
        }
        let mut candidate: Project = serde_json::from_value(candidate)
            .map_err(|error| ChangeError::Invalid(error.to_string()))?;
        candidate.copy_view_state_from(project);
        candidate.inherit_asset_readers(project);
        candidate.validate().map_err(ChangeError::Invalid)?;
        Ok(candidate)
    }
}

fn shared_value(project: &Project) -> Result<Value, serde_json::Error> {
    let mut project = project.clone();
    project.clear_view_state();
    serde_json::to_value(project)
}

impl Change {
    fn apply(&self, project: &mut Value, retry: bool) -> Result<(), ChangeError> {
        let (last, parents) = self
            .path
            .split_last()
            .ok_or_else(|| ChangeError::Invalid("change must address a document field".into()))?;
        let conflict = || {
            let path = self
                .path
                .iter()
                .map(|part| match part {
                    Segment::Field(name) | Segment::Item(name) => name.as_str(),
                })
                .collect::<Vec<_>>()
                .join("/");
            ChangeError::Conflict(path)
        };
        let parent = parents
            .iter()
            .try_fold(project, |parent, segment| match segment {
                Segment::Field(key) => parent.as_object_mut()?.get_mut(key),
                Segment::Item(id) => parent
                    .as_array_mut()?
                    .iter_mut()
                    .find(|item| item_id(item) == Some(id)),
            })
            .ok_or_else(conflict)?;
        match last {
            Segment::Field(key) => {
                let fields = parent.as_object_mut().ok_or_else(conflict)?;
                if retry {
                    return self
                        .after
                        .matches(fields.get(key))
                        .then_some(())
                        .ok_or_else(conflict);
                }
                if !self.before.matches(fields.get(key)) {
                    return Err(conflict());
                }
                match &self.after {
                    Field::Present(value) => {
                        fields.insert(key.clone(), value.clone());
                    }
                    Field::Missing => {
                        fields.remove(key);
                    }
                }
            }
            Segment::Item(id) => {
                let items = parent.as_array_mut().ok_or_else(conflict)?;
                let index = items.iter().position(|item| item_id(item) == Some(id));
                if retry {
                    return self
                        .after
                        .matches(index.map(|index| &items[index]))
                        .then_some(())
                        .ok_or_else(conflict);
                }
                if !self.before.matches(index.map(|index| &items[index])) {
                    return Err(conflict());
                }
                match (&self.after, index) {
                    (Field::Present(item), Some(index)) if item_id(item) == Some(id) => {
                        items[index] = item.clone();
                    }
                    (Field::Missing, Some(index)) => {
                        items.remove(index);
                    }
                    (Field::Missing, None) => {}
                    (Field::Present(item), None) if item_id(item) == Some(id) => {
                        let start = item_start(item).ok_or_else(conflict)?;
                        let index = items
                            .iter()
                            .position(|existing| {
                                item_start(existing).is_none_or(|time| time >= start)
                            })
                            .unwrap_or(items.len());
                        items.insert(index, item.clone());
                    }
                    _ => {
                        return Err(ChangeError::Invalid(
                            "an item change must preserve its ID".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl Field {
    fn matches(&self, value: Option<&Value>) -> bool {
        match self {
            Self::Missing => value.is_none(),
            Self::Present(expected) => value == Some(expected),
        }
    }
}

fn item_id(value: &Value) -> Option<&str> {
    value.get("id")?.as_str()
}

fn item_start(value: &Value) -> Option<Time> {
    Time::deserialize(value.get("start")?).ok()
}

fn collect(
    path: &mut Vec<Segment>,
    before: Option<&Value>,
    after: Option<&Value>,
    changes: &mut Vec<Change>,
) {
    if before == after {
        return;
    }
    match (before, after) {
        (Some(Value::Object(before)), Some(Value::Object(after))) => {
            let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
            for key in keys {
                path.push(Segment::Field(key.clone()));
                collect(path, before.get(key), after.get(key), changes);
                path.pop();
            }
        }
        (Some(Value::Array(before)), Some(Value::Array(after))) if same_items(before, after) => {
            for (before, after) in before.iter().zip(after) {
                path.push(Segment::Item(
                    item_id(before).expect("validated item array").to_string(),
                ));
                collect(path, Some(before), Some(after), changes);
                path.pop();
            }
        }
        (Some(Value::Array(before_items)), Some(Value::Array(after_items)))
            if collect_timed_items(path, before_items, after_items, changes) => {}
        _ => changes.push(change(path, before, after)),
    }
}

fn change(path: &[Segment], before: Option<&Value>, after: Option<&Value>) -> Change {
    Change {
        path: path.to_vec(),
        before: before.map_or(Field::Missing, |value| Field::Present(value.clone())),
        after: after.map_or(Field::Missing, |value| Field::Present(value.clone())),
    }
}

fn timed_items(items: &[Value]) -> Option<BTreeMap<&str, &Value>> {
    let by_id: BTreeMap<_, _> = items
        .iter()
        .map(|item| Some((item_id(item)?, item_start(item).map(|_| item)?)))
        .collect::<Option<_>>()?;
    (by_id.len() == items.len()).then_some(by_id)
}

fn collect_timed_items(
    path: &mut Vec<Segment>,
    before: &[Value],
    after: &[Value],
    changes: &mut Vec<Change>,
) -> bool {
    let (Some(old), Some(new)) = (timed_items(before), timed_items(after)) else {
        return false;
    };
    let ordered = |items: &[Value]| items.iter().map(item_start).is_sorted();
    let retained_before = before
        .iter()
        .filter_map(item_id)
        .filter(|id| new.contains_key(id));
    let retained_after = after
        .iter()
        .filter_map(item_id)
        .filter(|id| old.contains_key(id));
    if !ordered(before) || !ordered(after) || !retained_before.eq(retained_after) {
        return false;
    }
    for (&id, item) in &old {
        path.push(Segment::Item(id.to_owned()));
        match new.get(id).copied() {
            Some(next) => collect(path, Some(item), Some(next), changes),
            None => changes.push(change(path, Some(item), None)),
        }
        path.pop();
    }
    for (&id, item) in &new {
        if !old.contains_key(id) {
            path.push(Segment::Item(id.to_owned()));
            changes.push(change(path, None, Some(item)));
            path.pop();
        }
    }
    true
}

fn same_items(before: &[Value], after: &[Value]) -> bool {
    let mut ids = BTreeSet::new();
    before.len() == after.len()
        && before.iter().zip(after).all(|(before, after)| {
            item_id(before).is_some_and(|id| item_id(after) == Some(id) && ids.insert(id))
        })
}
