//! Persistent homogeneous lists with structural sharing.
use crate::{Type, Value};
mod values;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
pub use values::Values;

const MAX_INLINE_COMPARISON_ITEMS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ListEdit {
    Append(Value),
    Insert { index: usize, value: Value },
    RemoveAt(usize),
    ReplaceAt { index: usize, value: Value },
    Move { from_index: usize, to_index: usize },
}

/// Proof of an edit performed by the collection itself, rather than a caller's
/// asserted relationship between two independently supplied snapshots.
#[derive(Clone, Debug)]
pub struct ListEditResult {
    source: List,
    output: List,
    added: Option<Value>,
    removed: Option<Value>,
    changed: bool,
    operation: ListEdit,
}
impl ListEditResult {
    pub fn source(&self) -> &List {
        &self.source
    }
    pub fn output(&self) -> &List {
        &self.output
    }
    pub fn added(&self) -> Option<&Value> {
        self.added.as_ref()
    }
    pub fn removed(&self) -> Option<&Value> {
        self.removed.as_ref()
    }
    pub fn changed(&self) -> bool {
        self.changed
    }
    pub fn moved(&self) -> bool {
        matches!(self.operation, ListEdit::Move { .. })
    }
    pub fn operation(&self) -> &ListEdit {
        &self.operation
    }
}

/// The element schema survives empty results. Edits share unchanged tree branches.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct List {
    pub element_type: Arc<Type>,
    pub values: Values,
}
impl PartialEq for List {
    fn eq(&self, other: &Self) -> bool {
        self.values.ptr_eq(&other.values) || self.values == other.values
    }
}
impl List {
    /// Prove equal values from shared storage or a bounded small-inline comparison.
    /// Independently built larger vectors deliberately fall back to full checking.
    pub fn shares_values(&self, other: &Self) -> bool {
        self.values.ptr_eq(&other.values)
            || self.len() <= MAX_INLINE_COMPARISON_ITEMS && self == other
    }
    pub fn edit(&self, edit: ListEdit) -> Result<ListEditResult, String> {
        let mut output = self.clone();
        let mut added = None;
        let mut removed = None;
        let mut changed = true;
        let operation = edit.clone();
        let check_index = |parameter: &str, index: usize, allow_end: bool| {
            if index > self.len() || !allow_end && index == self.len() {
                Err(format!(
                    "{parameter} {index} is out of bounds for a list of {} items; {}",
                    self.len(),
                    if allow_end {
                        format!(
                            "insertion index must be between 0 and {} inclusive",
                            self.len()
                        )
                    } else if self.is_empty() {
                        "the list is empty; insert an item before removing, replacing, or moving it"
                            .into()
                    } else {
                        format!("valid indices are 0 through {}", self.len() - 1)
                    }
                ))
            } else {
                Ok(())
            }
        };
        let refine = |output: &mut List, value: Value| {
            if let Some(common) = output.element_type.join_list_lengths(&value.ty())
                && common != *output.element_type
            {
                output.element_type = Arc::new(common);
            }
            value.with_type(&output.element_type)
        };
        match edit {
            ListEdit::Append(value) => {
                let value = refine(&mut output, value)?;
                output.values.push_back(value.clone());
                added = Some(value);
            }
            ListEdit::Insert { index, value } => {
                check_index("index", index, true)?;
                let value = refine(&mut output, value)?;
                output.values.insert(index, value.clone());
                added = Some(value);
            }
            ListEdit::RemoveAt(index) => {
                check_index("index", index, false)?;
                removed = Some(output.values.remove(index));
            }
            ListEdit::ReplaceAt { index, value } => {
                check_index("index", index, false)?;
                let value = refine(&mut output, value)?;
                if output.values[index] == value {
                    changed = false;
                } else {
                    removed = Some(output.values.set(index, value.clone()));
                    added = Some(value);
                }
            }
            ListEdit::Move {
                from_index,
                to_index,
            } => {
                check_index("from_index", from_index, false)?;
                check_index("to_index", to_index, false)?;
                if from_index == to_index {
                    changed = false;
                } else {
                    let value = output.values.remove(from_index);
                    output.values.insert(to_index, value);
                }
            }
        }
        Ok(ListEditResult {
            source: self.clone(),
            output,
            added,
            removed,
            changed,
            operation,
        })
    }
    pub fn from_values(element_type: Arc<Type>, values: Values) -> Self {
        Self {
            element_type,
            values,
        }
    }
    pub fn typed(element_type: Type, values: Vec<Value>) -> Self {
        Self::from_values(Arc::new(element_type), values.into())
    }
}
impl std::ops::Deref for List {
    type Target = Values;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl From<Vec<Value>> for List {
    fn from(values: Vec<Value>) -> Self {
        let element_type = values.first().map_or(Type::Unknown, Value::ty);
        Self::typed(element_type, values)
    }
}
impl FromIterator<Value> for List {
    fn from_iter<T: IntoIterator<Item = Value>>(values: T) -> Self {
        let values: Values = values.into_iter().collect();
        let element_type = values.front().map_or(Type::Unknown, Value::ty);
        Self::from_values(Arc::new(element_type), values)
    }
}
