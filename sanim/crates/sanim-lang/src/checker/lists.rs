//! List type arguments and homogeneous literal inference.
use super::*;
impl Checker<'_> {
    pub(super) fn list_type_arguments<'a>(
        &self,
        args: &'a [TypeExpr],
        span: Span,
    ) -> Result<(&'a TypeExpr, sanim_core::ListLength), Diagnostic> {
        match args {
            [inner]=>Ok((inner,sanim_core::ListLength::Unknown)),
            [inner,TypeExpr::ConstLength(length)]=>Ok((inner,sanim_core::ListLength::Exact(*length))),
            [inner,TypeExpr::Named(name,args)] if name=="_" && args.is_empty()=>Ok((inner,sanim_core::ListLength::Unknown)),
            _=>Err(self.error("S105","List requires an element type and optional fixed length: List<Number>, List<Number,3>, or List<Number,_>; named const generics are not declared by this syntax",span)),
        }
    }
    pub(super) fn list_literal(
        &mut self,
        items: &[Expr],
        env: &Environment,
        expected: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<Type, Diagnostic> {
        let wanted = match expected {
            Some(Type::List(inner, _)) => Some(inner.as_ref()),
            _ => None,
        };
        let mut inner = Type::Unknown;
        for item in items {
            let actual = self.expression(item, env, wanted, variables)?;
            if let Some(wanted) = wanted {
                self.expect(wanted, &actual, item.span)?;
            }
            if inner == Type::Unknown {
                inner = actual;
            } else if !inner.accepts(&actual) {
                let mut remaining = self.type_budget.get();
                sanim_core::type_budget::charge(&inner, &mut remaining)
                    .map_err(|reason| self.error("S105", reason, item.span))?;
                self.type_budget.set(remaining);
                if let Some(joined) = inner.join_list_lengths(&actual) {
                    inner = joined;
                } else if let Some(wanted) = wanted {
                    inner = self.copy_type(wanted, item.span)?;
                } else {
                    self.expect(&inner, &actual, item.span)?;
                }
            }
        }
        if inner == Type::Unknown
            && let Some(wanted) = wanted
        {
            inner = self.copy_type(
                wanted,
                items.first().map_or(Span::default(), |item| item.span),
            )?;
        }
        Ok(Type::List(
            Box::new(inner),
            sanim_core::ListLength::Exact(items.len()),
        ))
    }
}
