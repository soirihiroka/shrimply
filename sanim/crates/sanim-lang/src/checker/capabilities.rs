//! Function effects are explicit contracts, checked at their source operations and calls.
use super::*;
use sanim_core::{FnCapabilities, FnCapability};

impl Checker<'_> {
    pub(super) fn require_capabilities(
        &self,
        required: FnCapabilities,
        span: Span,
        reason: &str,
    ) -> Result<(), Diagnostic> {
        let Some(allowed) = self.function_capabilities else {
            return Ok(());
        };
        if allowed.allows(required) {
            return Ok(());
        }
        let capabilities = allowed.union(required);
        let name = self
            .function
            .as_deref()
            .expect("active function capability contract");
        let (label, declaration) = if name.starts_with("$fn") {
            (
                "anonymous function".to_owned(),
                format!("fn<{capabilities}> (...)"),
            )
        } else {
            (
                format!("function '{name}'"),
                format!("fn<{capabilities}> {name}"),
            )
        };
        Err(self.error("S133", format!("{label} requires capabilities <{capabilities}>: {reason}; declare {declaration} or move this operation to a capable caller"), span))
    }
    pub(super) fn require_capability(
        &self,
        capability: FnCapability,
        span: Span,
        reason: &str,
    ) -> Result<(), Diagnostic> {
        let mut required = FnCapabilities::default();
        required.insert(capability);
        self.require_capabilities(required, span, reason)
    }
    pub(super) fn resolve_callable_type(
        &self,
        capabilities: FnCapabilities,
        parameters: &[(BindingKind, String, TypeExpr)],
        result: &TypeExpr,
        variables: &BTreeMap<String, Type>,
        depth: usize,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        let mut seen = BTreeSet::new();
        let parameters = parameters
            .iter()
            .map(|(kind, name, ty)| {
                if !seen.insert(name) {
                    return Err(self.error(
                        "S103",
                        format!("duplicate function parameter '{name}'"),
                        span,
                    ));
                }
                if prelude::protected(name) {
                    return Err(self.error(
                        "S101",
                        format!("'{name}' is protected and cannot be a function parameter"),
                        span,
                    ));
                }
                let ty = self.resolve(ty, variables, depth + 1, span)?;
                if ty.contains_signal() {
                    return Err(self.error(
                        "S106",
                        "write sig before the function parameter name",
                        span,
                    ));
                }
                Ok(sanim_core::FunctionParameter {
                    name: name.clone(),
                    ty,
                    kind: *kind,
                })
            })
            .collect::<Result<Vec<_>, Diagnostic>>()?;
        let result = self.resolve(result, variables, depth + 1, span)?;
        let mut required = FnCapabilities::default();
        for parameter in &parameters {
            match parameter.kind {
                BindingKind::Mut => {
                    required.insert(FnCapability::Mut);
                }
                BindingKind::Sig => {
                    required.insert(FnCapability::Sig);
                }
                BindingKind::Let => {}
            }
        }
        if result.contains_signal() {
            required.insert(FnCapability::Sig);
        }
        if !capabilities.allows(required) {
            return Err(self.error("S133", format!("function type requires capabilities <{}>; write fn<{}>(...) with explicit permissions for its mutable/signal interface", capabilities.union(required), capabilities.union(required)), span));
        }
        Ok(Type::Function(Signature {
            capabilities,
            generics: vec![],
            parameters,
            result: Box::new(result),
            default_count: 0,
            mutates: capabilities.contains(FnCapability::Mut),
        }))
    }
    pub(super) fn check_name_capture(
        &self,
        name: &str,
        env: &Environment,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let Some(function) = &self.function else {
            return Ok(());
        };
        let binding = self
            .binding_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name));
        if binding.is_some_and(|binding| {
            binding.function.as_ref() != Some(function)
                && (binding.kind != BindingKind::Let
                    || matches!(env.get(name).map(|ty| ty.as_ref()), Some(Type::Signal(_))))
        }) {
            self.require_capability(
                FnCapability::Ref,
                span,
                &format!("reading outer mutable or signal binding '{name}' is an external read"),
            )?;
        }
        Ok(())
    }
    pub(super) fn check_call_capabilities(
        &self,
        expr: &Expr,
        env: &Environment,
    ) -> Result<(), Diagnostic> {
        let ExprKind::Call { callee, args, .. } = &expr.kind else {
            return Ok(());
        };
        if self.function_capabilities.is_none() {
            return Ok(());
        }
        let scope = (env, &self.result_types);
        if let Some(call) = crate::timing::builtin_call(callee, args, &self.native_methods) {
            let mut required = call.capabilities(&scope);
            if let Some(callback) = crate::list_functions::callback_type(&call, &scope) {
                for signature in callback.signatures() {
                    required = required.union(signature.capabilities);
                }
            }
            self.require_capabilities(
                required,
                expr.span,
                &format!(
                    "calling '{}' uses its declared effect contract",
                    call.builtin.name
                ),
            )
        } else if let Some(ty) = crate::analysis::lookup(callee, &scope) {
            let mut required = FnCapabilities::default();
            for signature in ty.signatures() {
                required = required.union(signature.capabilities);
            }
            self.require_capabilities(required, expr.span, "a pure function cannot call an impure function; callee capabilities must be allowed by its caller")
        } else {
            Ok(())
        }
    }
}
