//! Bounded static cache-key analysis, shared by repeated executions of source expressions.
use super::*;
pub(super) struct Plan {
    pub fingerprint: Option<String>,
    pub names: Vec<String>,
    pub element_identity: bool,
    pub iteration_limit: bool,
}
impl Runtime<'_, '_> {
    pub(super) fn expression_plan(&mut self, expr: &Expr) -> Result<Arc<Plan>, Diagnostic> {
        let key = expr.cache_id;
        if let Some(plan) = self.expression_plans.get(&key) {
            return Ok(plan);
        }
        let fingerprint = if cacheable(expr) {
            Some(digest(&serde_json::to_vec(&expr.kind).map_err(
                |error| self.error("S301", error.to_string(), expr.span),
            )?))
        } else {
            None
        };
        let mut names = BTreeSet::new();
        if fingerprint.is_some() {
            dependencies(expr, &mut names);
        }
        let element_identity = names.iter().any(|name| {
            prelude::builtin(name).is_some_and(|builtin| builtin.result == Type::Element)
        });
        let iteration_limit = names.contains("range");
        let bytes = std::mem::size_of::<Plan>()
            + std::mem::size_of_val(&key)
            + fingerprint.as_ref().map_or(0, String::len)
            + names
                .iter()
                .map(|name| std::mem::size_of::<String>() + name.len())
                .sum::<usize>();
        let plan = Arc::new(Plan {
            fingerprint,
            names: names.into_iter().collect(),
            element_identity,
            iteration_limit,
        });
        self.expression_plans.insert(key, plan.clone(), bytes);
        Ok(plan)
    }
}
