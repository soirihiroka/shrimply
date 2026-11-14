//! Native constructor and host-operation argument/type checking.
use super::*;

impl Checker<'_> {
    pub(super) fn native(
        &mut self,
        expr: &Expr,
        name: &str,
        args: &[(Option<String>, Expr)],
        env: &Environment,
        expected: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<Type, Diagnostic> {
        let ExprKind::Call { types, .. } = &expr.kind else {
            unreachable!()
        };
        let builtin = prelude::builtin(name)
            .ok_or_else(|| self.error("S120", format!("unknown function '{name}'"), expr.span))?;
        if builtin.call_schedules(args)
            && let Some(function) = &self.function
        {
            self.effects.insert(function.clone());
        }
        if let Some(duration) = builtin.duration_argument(args) {
            let ty = self.expression(
                duration,
                env,
                Some(&Type::Numeric(Unit::Duration)),
                variables,
            )?;
            self.expect(&Type::Numeric(Unit::Duration), &ty, duration.span)?;
        }
        if let Some(ease) = builtin.easing_argument(args) {
            let ty = self.expression(ease, env, Some(&Type::String), variables)?;
            self.expect(&Type::String, &ty, ease.span)?;
        }
        if crate::dataframe::definition(name).is_some() {
            if !types.is_empty() {
                return Err(self.error("S123", "DataFrame operations infer their schema from the typed input; no explicit type arguments are needed", expr.span));
            }
            return self.frame_native(expr, builtin, args, env, variables);
        }
        if matches!(builtin.execute, prelude::Execution::List) {
            if !types.is_empty() {
                return Err(self.error(
                    "S123",
                    "List methods infer type arguments from the items and callback",
                    expr.span,
                ));
            }
            return self.list_function(expr, builtin, args, env, expected, variables);
        }
        if builtin.result == Type::Data && builtin.name == "Data" {
            if !types.is_empty() {
                return Err(self.error("S123", "Data does not take type arguments", expr.span));
            }
            let params = builtin
                .parameters
                .iter()
                .map(|p| (p.name, p.ty.clone(), p.required))
                .collect::<Vec<_>>();
            let ordered = self.arguments(args, &params, expr.span)?;
            self.data_expression(ordered[0].expect("required data argument"), env, variables)?;
            return Ok(Type::Data);
        }
        if matches!(
            builtin.execute,
            prelude::Execution::Decode | prelude::Execution::Query
        ) {
            if matches!(builtin.execute, prelude::Execution::Decode)
                && types.is_empty()
                && expected.is_none()
            {
                return Err(self.error(
                    "S125",
                    "write decode<ResultType>(data) or annotate the binding",
                    expr.span,
                ));
            }
            let result = if types.len() == 1 {
                self.resolve(&types[0], variables, 0, expr.span)?
            } else if types.is_empty() {
                let expected = expected.cloned().unwrap_or(Type::List(
                    Box::new(Type::Dict),
                    sanim_core::ListLength::Unknown,
                ));
                if matches!(builtin.execute, prelude::Execution::Query) {
                    let Type::List(row, _) = expected else {
                        return Err(self.error("S125", "query produces List<Row>", expr.span));
                    };
                    *row
                } else {
                    expected
                }
            } else {
                return Err(self.error(
                    "S125",
                    format!("{name} requires one type argument"),
                    expr.span,
                ));
            };
            if !result.readable() || result == Type::Font {
                return Err(self.error(
                    "S125",
                    format!("{result} is not a data schema"),
                    expr.span,
                ));
            }
            let params = builtin
                .parameters
                .iter()
                .map(|p| (p.name, p.ty.clone(), p.required))
                .collect::<Vec<_>>();
            let ordered = self.arguments(args, &params, expr.span)?;
            let empty_tables = ordered.get(1).is_some_and(|arg| matches!(arg, Some(Expr { kind: ExprKind::Dict(fields), .. }) if fields.is_empty()));
            if matches!(builtin.execute, prelude::Execution::Query) {
                self.sql_template(ordered[0].expect("required SQL input"), env, variables)?;
            }
            let mut actuals = vec![];
            for (param, arg) in builtin.parameters.iter().zip(ordered) {
                if param.name == "duration" {
                    continue;
                }
                if let Some(arg) = arg {
                    let actual = self.expression(arg, env, Some(&param.ty), variables)?;
                    if param.name == "sql" && Type::String.accepts(&actual) {
                        return Err(self.error("S125","query accepts structured Template or Sql, not an already rendered String. Pass the literal directly, use let statement:Template=\"...\", or construct Sql(\"...\"); string concatenation is not a safe SQL template",arg.span));
                    }
                    self.expect(&param.ty, &actual, arg.span)?;
                    actuals.push(actual);
                } else {
                    actuals.push(Type::Unit);
                }
            }
            if matches!(builtin.execute, prelude::Execution::Decode) {
                if !matches!(actuals[0], Type::Data | Type::Dict) {
                    return Err(self.error("S106", "decode<T> requires Data or Dict", expr.span));
                }
                self.result_types
                    .insert((expr.span.start, expr.span.end), result.clone());
                return Ok(result);
            }
            sanim_data::sqlite::validate_row_type(&result)
                .map_err(|message| self.error("S125", message, expr.span))?;
            let empty = BTreeMap::new();
            let tables =
                match &actuals[1] {
                    Type::Record(tables) => tables,
                    Type::Unit => &empty,
                    Type::Dict if empty_tables => &empty,
                    _ => return Err(self.error(
                        "S106",
                        "query inputs must be named lists, for example tables={samples=samples}",
                        expr.span,
                    )),
                };
            for (name, table) in tables {
                let Type::List(row, _) = table else {
                    return Err(self.error(
                        "S106",
                        format!("SQL input '{name}' must be a List<T>, found {table}"),
                        expr.span,
                    ));
                };
                sanim_data::sqlite::validate_row_type(row).map_err(|message| {
                    self.error("S125", format!("SQL input '{name}': {message}"), expr.span)
                })?;
            }
            let length = if let Some(Type::List(_, length)) = expected {
                *length
            } else {
                sanim_core::ListLength::Unknown
            };
            let result = Type::List(Box::new(result), length);
            self.result_types
                .insert((expr.span.start, expr.span.end), result.clone());
            return Ok(result);
        }
        if matches!(builtin.execute, prelude::Execution::Read) {
            let ty = if types.len() == 1 {
                self.resolve(&types[0], variables, 0, expr.span)?
            } else if types.is_empty() {
                expected.cloned().ok_or_else(||self.error("S125","cannot infer read result. Write read<List<Row>>(\"file.csv\") or annotate the binding.",expr.span))?
            } else {
                return Err(self.error("S125", "read requires one type argument", expr.span));
            };
            if !ty.readable() {
                return Err(self.error(
                    "S125",
                    format!("{ty:?} is not a readable data type"),
                    expr.span,
                ));
            }
            let params = builtin
                .parameters
                .iter()
                .map(|p| (p.name, p.ty.clone(), p.required))
                .collect::<Vec<_>>();
            let ordered = self.arguments(args, &params, expr.span)?;
            let path = ordered[0].ok_or_else(|| self.error("S125", "missing path", expr.span))?;
            let actual = self.expression(path, env, Some(&Type::String), variables)?;
            self.expect(&Type::String, &actual, path.span)?;
            self.result_types
                .insert((expr.span.start, expr.span.end), ty.clone());
            return Ok(ty);
        }
        if !types.is_empty() {
            return Err(self.error(
                "S123",
                format!("'{name}' does not accept explicit type arguments"),
                expr.span,
            ));
        }
        let parameters = builtin
            .parameters
            .iter()
            .map(|p| (p.name, p.ty.clone(), p.required))
            .collect::<Vec<_>>();
        let ordered = self.arguments(args, &parameters, expr.span)?;
        let mut actuals = vec![];
        for (parameter, value) in builtin.parameters.iter().zip(ordered.iter().copied()) {
            if matches!(parameter.name, "duration" | "ease") {
                continue;
            }
            if let Some(value) = value {
                let ty = self.expression(value, env, Some(&parameter.ty), variables)?;
                if ty.contains_function() {
                    return Err(self.error("S111","function values cannot be passed as arguments or stored in containers; call the function directly",value.span));
                }
                self.expect(&parameter.ty, &ty, value.span)?;
                actuals.push(ty);
            } else {
                actuals.push(Type::Unit);
            }
        }
        if builtin.name == "Sql" {
            self.sql_template(ordered[0].expect("required SQL template"), env, variables)?;
        }
        if builtin.name == "str" {
            let argument = |name| {
                builtin
                    .parameters
                    .iter()
                    .position(|parameter| parameter.name == name)
                    .and_then(|index| ordered[index])
            };
            let format = argument("format");
            let conversion = argument("conversion");
            let known_conversion = conversion.map(|expression| match &expression.kind {
                ExprKind::String(value) => value.as_str(),
                // Every valid runtime conversion produces String; its actual
                // label is checked when the dynamic value is evaluated.
                _ => "s",
            });
            if let Some(conversion) = conversion
                && let ExprKind::String(label) = &conversion.kind
            {
                sanim_core::math::format::validate("", &Type::Unknown, Some(label)).map_err(
                    |message| self.error("S106", format!("str: {message}"), conversion.span),
                )?;
            }
            if let Some(format) = format
                && let ExprKind::String(specification) = &format.kind
            {
                sanim_core::math::format::validate(specification, &actuals[0], known_conversion)
                    .map_err(|message| {
                        self.error("S106", format!("str: {message}"), format.span)
                    })?;
            }
        }
        let mut result = (builtin.check)(builtin, &actuals)
            .map_err(|e| self.error("S106", format!("{name}: {e}"), expr.span))?;
        if name == "range" {
            let count = |name, default| {
                let at = builtin
                    .parameters
                    .iter()
                    .position(|parameter| parameter.name == name)?;
                crate::list_functions::constant_count(ordered[at], default)
            };
            if let (Some(start), Some(end), Some(step)) = (
                count("start", Some(0)),
                count("end", None),
                count("step", Some(1)),
            ) {
                let length = sanim_core::math::range_count(start, end, step)
                    .map_err(|reason| self.error("S106", reason, expr.span))?;
                result = Type::List(
                    Box::new(Type::Numeric(Unit::Number)),
                    sanim_core::ListLength::Exact(length),
                );
            }
            self.result_types.insert(
                (expr.span.start, expr.span.end),
                self.copy_type(&result, expr.span)?,
            );
        }
        Ok(result)
    }
}
