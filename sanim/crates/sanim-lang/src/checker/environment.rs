//! Scope forks share their map and immutable type payloads; updates copy only tree paths.
use super::*;
use std::sync::Arc;

pub(crate) type Environment = imbl::OrdMap<Arc<str>, Arc<Type>>;

pub(crate) trait TypeScope {
    fn get_type(&self, name: &str) -> Option<&Type>;
}
impl TypeScope for Environment {
    fn get_type(&self, name: &str) -> Option<&Type> {
        self.get(name).map(Arc::as_ref)
    }
}
impl TypeScope for BTreeMap<String, Type> {
    fn get_type(&self, name: &str) -> Option<&Type> {
        self.get(name)
    }
}

impl Checker<'_> {
    fn environment_bytes(&self, bytes: usize, span: Span) -> Result<(), Diagnostic> {
        self.type_budget
            .set(self.type_budget.get().checked_sub(bytes).ok_or_else(|| {
                self.error(
                    "S105",
                    "resolved type budget exceeded; simplify or split repeated nested schemas",
                    span,
                )
            })?);
        Ok(())
    }
    fn environment_path(&self, env: &Environment, span: Span) -> Result<(), Diagnostic> {
        // imbl 7.0's ordered B+tree has 16 entry slots; non-root branches have
        // at least half as many children. Bound a write as if every path node is
        // shared and splits, plus a new root. Keys and payloads are Arc-shared.
        const NODE_SLOTS: usize = 16;
        const MIN_CHILDREN: usize = NODE_SLOTS / 2;
        let height = 1 + env.len().checked_ilog(MIN_CHILDREN).unwrap_or(0) as usize;
        let levels = 2 * height + 1;
        let entry = std::mem::size_of::<(Arc<str>, Arc<Type>, Arc<Environment>)>();
        self.environment_bytes(
            levels.saturating_mul(NODE_SLOTS).saturating_mul(entry),
            span,
        )
    }
    pub(super) fn insert_environment(
        &self,
        env: &mut Environment,
        name: String,
        ty: Type,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.environment_path(env, span)?;
        // New type and key allocations each retain two Arc reference counters.
        self.environment_bytes(
            std::mem::size_of::<Type>() + 4 * std::mem::size_of::<usize>() + name.len(),
            span,
        )?;
        env.insert(Arc::from(name), Arc::new(ty));
        Ok(())
    }
    pub(super) fn mutate_environment<'a>(
        &self,
        env: &'a mut Environment,
        name: &str,
        span: Span,
    ) -> Result<&'a mut Type, Diagnostic> {
        self.environment_path(env, span)?;
        let value = env.get_mut(name).expect("existing checked binding");
        if Arc::strong_count(value) > 1 {
            let mut remaining = self.type_budget.get();
            sanim_core::type_budget::charge(value, &mut remaining)
                .map_err(|reason| self.error("S105", reason, span))?;
            self.type_budget.set(remaining);
        }
        Ok(Arc::make_mut(value))
    }
    pub(super) fn fork_environment(
        &self,
        env: &Environment,
        span: Span,
    ) -> Result<Environment, Diagnostic> {
        self.environment_bytes(std::mem::size_of::<Environment>(), span)?;
        Ok(env.clone())
    }
    pub(super) fn copy_substitutions(
        &self,
        values: &BTreeMap<String, Type>,
        span: Span,
    ) -> Result<BTreeMap<String, Type>, Diagnostic> {
        values
            .iter()
            .map(|(name, ty)| self.copy_type(ty, span).map(|ty| (name.clone(), ty)))
            .collect()
    }
    pub(super) fn substitute(
        &self,
        ty: &Type,
        bindings: &BTreeMap<String, Type>,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        let mut remaining = self.type_budget.get();
        sanim_core::type_budget::charge_substitution(ty, bindings, &mut remaining)
            .map_err(|e| self.error("S105", e, span))?;
        self.type_budget.set(remaining);
        Ok(ty.substitute(bindings))
    }
    pub(super) fn copy_type(&self, ty: &Type, span: Span) -> Result<Type, Diagnostic> {
        let mut remaining = self.type_budget.get();
        sanim_core::type_budget::charge(ty, &mut remaining)
            .map_err(|e| self.error("S105", e, span))?;
        self.type_budget.set(remaining);
        Ok(ty.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, MemorySources};

    fn helpers(count: usize) -> String {
        (0..count)
            .map(|index| {
                format!("fn helper_{index}(value:Number):Number{{return value+{index}}}\n")
            })
            .collect()
    }
    fn cost(count: usize) -> usize {
        let limits = crate::Limits::default();
        let source = helpers(count);
        let module =
            crate::parser::parse("helpers.sanim", &source, limits.max_parse_steps).unwrap();
        let budget = std::cell::Cell::new(limits.max_type_bytes);
        super::super::check(
            "helpers.sanim",
            &module,
            &BTreeMap::new(),
            false,
            &budget,
            &limits,
        )
        .unwrap();
        limits.max_type_bytes - budget.get()
    }
    #[test]
    fn many_helpers_fit_the_normal_budget_without_quadratic_scope_charges() {
        let small = cost(40);
        let large = cost(80);
        assert!(
            large < small * 3,
            "doubling helpers must not quadruple scope-copy cost: {small} -> {large}"
        );
        let mut sources = MemorySources::default();
        sources.insert("helpers.sanim", helpers(500));
        sources.insert(
            "main.sanim",
            "import \"helpers\" as helpers\nlet result=helpers.helper_499(1)\nassert(result==500)",
        );
        let prepared = Engine::default()
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap();
        assert_eq!(
            prepared.bindings["result"].number().unwrap().value,
            500.into()
        );
    }
    #[test]
    fn branch_narrowing_and_function_locals_preserve_shared_parent_types() {
        let source = "type Mode=\"cold\"|\"hot\"\nlet mode:Mode=\"hot\"\nfn test(value:Number):Number{if mode==\"cold\"{let local_cold=1;return local_cold}\nlet local_hot=value+1;return local_hot}\nlet result=test(2)";
        let mut sources = MemorySources::default();
        sources.insert("main.sanim", source);
        let report = Engine::default().lint("main.sanim", &sources).unwrap();
        let exports = &report.modules["main.sanim"].checked.exports;
        assert_eq!(
            exports["mode"].string_literals().unwrap(),
            vec!["cold", "hot"]
        );
        assert!(!exports.contains_key("value"));
        assert!(!exports.contains_key("local_cold"));
        assert!(!exports.contains_key("local_hot"));
    }
    #[test]
    fn shared_types_stay_immutable_when_a_fork_is_updated() {
        let schema = Arc::new(Type::Record(BTreeMap::from([(
            "field".into(),
            Type::String,
        )])));
        let old = Environment::unit("shared".into(), schema.clone());
        let mut fork = old.clone();
        assert!(old.ptr_eq(&fork));
        fork.insert("other".into(), Arc::new(Type::Bool));
        assert!(Arc::ptr_eq(&old["shared"], &fork["shared"]));
        *Arc::make_mut(fork.get_mut("shared").unwrap()) = Type::Bool;
        assert_eq!(fork["shared"].as_ref(), &Type::Bool);
        assert_eq!(old["shared"].as_ref(), schema.as_ref());
        assert!(!old.contains_key("other"));
    }
    #[test]
    #[ignore = "manual compiler scope-fork performance measurement"]
    fn checker_environment_benchmark() {
        use std::{hint::black_box, time::Instant};
        const BINDINGS: usize = 500;
        const FORKS: usize = 2000;
        let old = (0..BINDINGS)
            .map(|index| {
                (
                    format!("binding_{index}"),
                    Type::Record(
                        (0..8)
                            .map(|field| (format!("field_{field}"), Type::String))
                            .collect(),
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let shared = old
            .iter()
            .map(|(name, ty)| (Arc::from(name.as_str()), Arc::new(ty.clone())))
            .collect::<Environment>();
        let started = Instant::now();
        for _ in 0..FORKS {
            let mut scope = black_box(&old).clone();
            scope.insert("parameter".into(), Type::Bool);
            black_box(scope);
        }
        let deep = started.elapsed();
        let started = Instant::now();
        for _ in 0..FORKS {
            let mut scope = black_box(&shared).clone();
            scope.insert(Arc::from("parameter"), Arc::new(Type::Bool));
            black_box(scope);
        }
        eprintln!(
            "{FORKS} forks of {BINDINGS} schemas: deep_copy={deep:?}, persistent={:?}",
            started.elapsed()
        );
    }
}
