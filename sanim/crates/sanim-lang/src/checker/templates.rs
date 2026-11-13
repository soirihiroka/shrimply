use super::*;
use sanim_core::TemplatePart;
impl Checker<'_> {
    pub(super) fn template(
        &mut self,
        expr: &Expr,
        parts: &[TemplatePart<Expr>],
        env: &Environment,
        expected: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<Type, Diagnostic> {
        let structured = crate::templates::expected(expected);
        if structured {
            self.template_expressions
                .insert((expr.span.start, expr.span.end), Type::Template);
        }
        for part in parts {
            if let TemplatePart::Hole {
                value,
                format,
                conversion,
            } = part
            {
                let ty = self.expression(value, env, None, variables)?;
                if ty.contains_function() {
                    return Err(self.error("S111","function values cannot be stored in template holes; call the function to produce a value",value.span));
                }
                if let Some(format) = format {
                    let actual = self.expression(format, env, Some(&Type::String), variables)?;
                    self.expect(&Type::String, &actual, format.span)?;
                    if !structured && let ExprKind::String(specification) = &format.kind {
                        let conversion = conversion.map(|conversion| conversion.to_string());
                        sanim_core::math::format::validate(
                            specification,
                            &ty,
                            conversion.as_deref(),
                        )
                        .map_err(|message| {
                            self.error(
                                "S106",
                                format!("string interpolation: {message}"),
                                format.span,
                            )
                        })?;
                    }
                }
            }
        }
        Ok(if structured {
            Type::Template
        } else {
            Type::String
        })
    }
    pub(super) fn sql_template(
        &mut self,
        expr: &Expr,
        env: &Environment,
        variables: &BTreeMap<String, Type>,
    ) -> Result<(), Diagnostic> {
        let ExprKind::Template(parts) = &expr.kind else {
            return Ok(());
        };
        for part in parts {
            if let TemplatePart::Hole {
                value,
                format,
                conversion,
            } = part
            {
                if format.is_some() || conversion.is_some() {
                    return Err(self.error("S125","SQL holes bind values directly: use {value} without formatting or !conversions; format a String first if needed",value.span));
                }
                let ty = self.expression(value, env, None, variables)?;
                if ty != Type::SqlIdent && !sanim_core::sql::parameter_type(&ty) {
                    return Err(self.error("S125",format!("SQL holes require scalar values or SqlIdent(name), found {ty}; project structured data to a scalar first"),value.span));
                }
            }
        }
        Ok(())
    }
}
