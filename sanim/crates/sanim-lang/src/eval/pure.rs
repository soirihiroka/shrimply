//! Pure calls reuse the existing bounded value cache after their work justifies a key.
use super::*;

pub(super) const MIN_MEMO_OPERATIONS: u64 = 64;
const MAX_MEMO_LITERAL_BYTES: usize = 4096;
const MAX_MEMO_KEY_BYTES: usize = 8 * 1024;
pub(super) const MAX_MEMO_INPUT_NODES: usize = 128;
pub(super) fn contains_animation(value: &Value) -> bool {
    match value {
        Value::Animation(_) => true,
        Value::List(list) => list.element_type.contains_animation(),
        Value::Record(fields) => fields.values().any(contains_animation),
        Value::Some(value) => contains_animation(value),
        Value::Variant { fields, .. } => fields.iter().any(contains_animation),
        Value::Array { element_type, .. } => element_type.contains_animation(),
        _ => false,
    }
}
pub(super) fn supported(value: &Value, remaining: &mut usize, max_literal_bytes: usize) -> bool {
    let Some(next) = remaining.checked_sub(1) else {
        return false;
    };
    *remaining = next;
    match value {
        Value::None => true,
        Value::String(text) => text.len() <= max_literal_bytes,
        Value::Template(template) => {
            let Some(next) = remaining.checked_sub(template.parts.len()) else {
                return false;
            };
            *remaining = next;
            template.parts.iter().all(|part| match part {
                TemplatePart::Text(text) => text.len() <= max_literal_bytes,
                TemplatePart::Hole { value, format, .. } => {
                    supported(value, remaining, max_literal_bytes)
                        && format
                            .as_ref()
                            .is_none_or(|value| supported(value, remaining, max_literal_bytes))
                }
            })
        }
        Value::Record(fields) => fields
            .values()
            .all(|value| supported(value, remaining, max_literal_bytes)),
        Value::Some(value) => supported(value, remaining, max_literal_bytes),
        Value::Variant { fields, .. } => fields
            .iter()
            .all(|value| supported(value, remaining, max_literal_bytes)),
        Value::List(list) => supported_schema(&list.element_type),
        Value::Array { element_type, .. } => supported_schema(element_type),
        // Persistent collections carry an immutable content proof, computed at most once.
        _ => supported_schema(&value.ty()),
    }
}
fn supported_schema(ty: &Type) -> bool {
    let mut budget = MAX_MEMO_KEY_BYTES;
    sanim_core::type_budget::charge(ty, &mut budget).is_ok()
        && !ty.contains(&|ty| {
            matches!(
                ty,
                Type::Unknown
                    | Type::Variable(_)
                    | Type::BoundedVariable(_, _)
                    | Type::Function(_)
                    | Type::Overloaded(_)
                    | Type::Data
                    | Type::Dict
                    | Type::Element
                    | Type::Animation
                    | Type::Signal(_)
                    | Type::DataFrame { .. }
                    | Type::Series(_)
            )
        })
}

impl Runtime<'_, '_> {
    pub(super) fn pure_result_type(&self, function: &Function) -> bool {
        function.capabilities.is_pure()
            && self.limits.max_cache_entries != 0
            && self.function_signature(function).is_some_and(|signature| {
                let result = signature.result.as_ref();
                let mut budget = MAX_MEMO_KEY_BYTES;
                sanim_core::type_budget::charge(result, &mut budget).is_ok()
                    && !result.contains_element()
                    && !result.contains_animation()
                    && !result.contains_signal()
                    && !result.contains_function()
            })
    }
    pub(super) fn pure_call_key(
        &mut self,
        function: &Function,
        group: &FunctionGroup,
        scope: &Env,
    ) -> Result<Option<String>, Diagnostic> {
        let started = Instant::now();
        let result = self.pure_call_key_inner(function, group, scope);
        self.statistics.cache_key_micros = self
            .statistics
            .cache_key_micros
            .saturating_add(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
        result
    }
    fn pure_call_key_inner(
        &mut self,
        function: &Function,
        group: &FunctionGroup,
        scope: &Env,
    ) -> Result<Option<String>, Diagnostic> {
        let mut work = MAX_MEMO_INPUT_NODES;
        let mut inputs = Values::new();
        let mut versions = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let mut pending = vec![(
            group.clauses[function.clause].clone(),
            scope.clone(),
            "root".to_owned(),
            true,
        )];
        while let Some((function, scope, namespace, arguments)) = pending.pop() {
            let names = function.references.iter().chain(
                function
                    .parameters
                    .iter()
                    .filter(|_| arguments)
                    .map(|parameter| &parameter.name),
            );
            for name in names {
                let Some(next) = work.checked_sub(1) else {
                    return Ok(None);
                };
                work = next;
                let Some(binding) = scope.get(name) else {
                    continue;
                };
                let value = match binding {
                    Binding::Value(value) => value.clone(),
                    Binding::Deferred(id) => {
                        let Some(value) =
                            self.reactive
                                .cached(*id, self.cursor, &self.animation.reactive)
                        else {
                            return Ok(None);
                        };
                        value.clone()
                    }
                    _ => return Ok(None),
                };
                if let Value::Function(key) = value {
                    if !seen.insert(key.clone()) {
                        continue;
                    }
                    let Some(dependency) = self.function_group(&key).cloned() else {
                        continue;
                    };
                    if dependency
                        .clauses
                        .iter()
                        .any(|function| !function.capabilities.is_pure())
                    {
                        return Ok(None);
                    }
                    self.record_function(&key, &dependency);
                    let interfaces = dependency
                        .clauses
                        .iter()
                        .filter_map(|function| {
                            self.loaded
                                .iter()
                                .find(|module| module.id == function.module)
                                .map(|module| module.interface_key.clone())
                        })
                        .collect::<Vec<_>>();
                    versions.insert(key.clone(), (dependency.fingerprint.clone(), interfaces));
                    for function in &dependency.clauses {
                        let captures = dependency
                            .captures
                            .clone()
                            .or_else(|| self.globals.get(&function.module).cloned())
                            .unwrap_or_default();
                        pending.push((function.clone(), captures, key.clone(), false));
                    }
                } else {
                    if !supported(&value, &mut work, MAX_MEMO_LITERAL_BYTES) {
                        return Ok(None);
                    }
                    inputs.insert(format!("{namespace}::{name}"), value);
                }
            }
        }
        let inputs: BTreeMap<_, _> = inputs
            .iter()
            .map(|(name, value)| {
                let value = match value {
                    Value::Font(font) => CacheDependency::Font(self.font_keys.key(font)),
                    value => CacheDependency::Value(sanim_core::value_key::ValueKey(value)),
                };
                (name, value)
            })
            .collect();
        let mut bytes = [0u8; MAX_MEMO_KEY_BYTES];
        let mut output = std::io::Cursor::new(bytes.as_mut_slice());
        let serialized = serde_json::to_writer(
            &mut output,
            &(
                "pure-call",
                SEMANTICS_VERSION,
                &group.fingerprint,
                function.clause,
                self.loaded
                    .iter()
                    .find(|module| module.id == function.module)
                    .map(|module| &module.interface_key),
                &self.type_bindings,
                self.limits.data,
                self.limits.max_iterations,
                self.limits.max_call_depth,
                (
                    self.animation.width,
                    self.animation.height,
                    self.animation.dpi,
                    self.animation.rate,
                ),
                sanim_data::font::system_font_epoch(),
                versions,
                inputs,
            ),
        );
        match serialized {
            Ok(()) => {
                let written = output.position() as usize;
                Ok(Some(digest(&bytes[..written])))
            }
            Err(error) if error.is_io() => Ok(None),
            Err(error) => Err(self.error("S301", error.to_string(), function.span)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_schemas_cannot_hide_dynamic_values_from_pure_cache_policy() {
        for leaf in [
            Type::Dict,
            Type::Data,
            Type::DataFrame {
                row: Box::new(Type::Record(Default::default())),
                groups: vec![],
            },
            Type::Series(Box::new(Type::Numeric(Unit::Number))),
            Type::Signal(Box::new(Type::Numeric(Unit::Number))),
        ] {
            let nested = Type::Record(
                std::iter::once(("nested".into(), Type::Option(Box::new(leaf)))).collect(),
            );
            let list = Value::List(List::typed(nested.clone(), vec![]));
            let array = Value::Array {
                element_type: Arc::new(nested),
                shape: vec![0],
                values: Default::default(),
            };
            for value in [list, array] {
                let mut remaining = MAX_MEMO_INPUT_NODES;
                assert!(!supported(&value, &mut remaining, MAX_MEMO_LITERAL_BYTES));
            }
        }
        let values = Value::List(List::typed(Type::VoxelGrid, vec![]));
        let mut remaining = MAX_MEMO_INPUT_NODES;
        assert!(
            supported(&values, &mut remaining, MAX_MEMO_LITERAL_BYTES),
            "immutable native geometry remains cacheable without a payload walk"
        );
        let text = Value::String("x".repeat(MAX_MEMO_LITERAL_BYTES + 1).into());
        let mut remaining = MAX_MEMO_INPUT_NODES;
        assert!(!supported(&text, &mut remaining, MAX_MEMO_LITERAL_BYTES));
        let mut remaining = MAX_MEMO_INPUT_NODES;
        assert!(
            supported(&text, &mut remaining, 2 * MAX_MEMO_LITERAL_BYTES),
            "result retention uses cache byte budget, not the smaller serialized-key budget"
        );
    }
}
