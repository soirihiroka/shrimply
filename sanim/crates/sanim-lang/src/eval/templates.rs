use super::*;
use sanim_core::{Template, TemplatePart};
impl Runtime<'_, '_> {
    pub(super) fn template_context(&self, span: Span) -> Option<&Type> {
        self.loaded
            .iter()
            .find(|module| module.id == self.module)
            .and_then(|module| {
                module
                    .checked
                    .template_expressions
                    .get(&(span.start, span.end))
            })
    }
    pub(super) fn template_expected(&self, span: Span) -> bool {
        self.template_context(span) == Some(&Type::Template)
    }
    pub(super) fn contextual_template(
        &self,
        value: Value,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        if self.template_context(span) == Some(&Type::String)
            && let Value::Template(template) = value
        {
            return template
                .render(self.limits.data.max_bytes)
                .map(|text| Value::String(text.into()))
                .map_err(|message| self.error("S305", message, span));
        }
        Ok(value)
    }
    pub(super) fn template(
        &mut self,
        expr: &Expr,
        parts: &[TemplatePart<Expr>],
        env: &Env,
    ) -> Result<Value, Diagnostic> {
        let mut values = Vec::with_capacity(parts.len());
        for part in parts {
            values.push(match part {
                TemplatePart::Text(text) => TemplatePart::Text(text.clone()),
                TemplatePart::Hole {
                    value,
                    format,
                    conversion,
                } => TemplatePart::Hole {
                    value: self.expression(value, env)?,
                    format: format
                        .as_ref()
                        .map(|format| self.expression(format, env))
                        .transpose()?,
                    conversion: *conversion,
                },
            });
        }
        let template = Template {
            parts: values.into(),
        };
        if self.template_expected(expr.span) {
            Ok(Value::Template(Arc::new(template)))
        } else {
            template
                .render(self.limits.data.max_bytes)
                .map(|text| Value::String(text.into()))
                .map_err(|message| self.error("S305", message, expr.span))
        }
    }
    pub(super) fn sql(
        &self,
        value: &Value,
        span: Span,
    ) -> Result<Arc<sanim_core::sql::Sql>, Diagnostic> {
        match value {
            Value::Sql(sql)=>Ok(sql.clone()),
            Value::Template(template)=>sanim_core::sql::Sql::compile(template,self.limits.data.max_bytes,self.limits.data.max_values).map(Arc::new).map_err(|message|self.error("S309",message,span)),
            value=>Err(self.error("S309",format!("query requires Template or Sql, found {}; write query(\"SELECT ... {{value}}\", tables=...) or let statement:Template=\"...\"",value.ty()),span)),
        }
    }
}
