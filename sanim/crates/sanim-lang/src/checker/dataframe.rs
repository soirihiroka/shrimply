use super::*;
use sanim_core::dataframe as frame;

fn names(expr: &Expr) -> Result<Vec<String>, String> {
    match &expr.kind {
        ExprKind::String(name) => Ok(vec![name.clone()]),
        ExprKind::List(values) => values.iter().map(|value| match &value.kind {
            ExprKind::String(name) => Ok(name.clone()),
            _ => Err("column names must be literal Strings so their types are known before execution".into()),
        }).collect(),
        _ => Err("use a literal column name or list of names, such as \"value\" or [\"x\", \"y\"]; dynamic names cannot determine a static schema".into()),
    }
}
fn mapping(expr: &Expr) -> Result<BTreeMap<String, String>, String> {
    let entries: Vec<(&str, &Expr)> =
        match &expr.kind {
            ExprKind::Record(entries) => entries
                .iter()
                .map(|(key, value)| (key.as_str(), value))
                .collect(),
            ExprKind::Dict(entries) => entries
                .iter()
                .map(|(key, value)| match &key.kind {
                    ExprKind::String(key) => Ok((key.as_str(), value)),
                    _ => Err("column mapping keys must be literal Strings"),
                })
                .collect::<Result<_, _>>()?,
            _ => return Err(
                "use a literal column mapping, such as {value=\"mean\"} or {\"value\": \"mean\"}"
                    .into(),
            ),
        };
    entries
        .into_iter()
        .map(|(key, value)| match &value.kind {
            ExprKind::String(value) => Ok((key.to_owned(), value.clone())),
            _ => Err(format!("column mapping '{key}' requires a literal String")),
        })
        .collect()
}
fn frame_type(schema: BTreeMap<String, Type>, groups: Vec<String>) -> Type {
    Type::DataFrame {
        row: Box::new(Type::Record(schema)),
        groups,
    }
}

impl Checker<'_> {
    pub(super) fn frame_index(
        &self,
        row: &Type,
        groups: &[String],
        index: &Expr,
        idx: &Type,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        if !groups.is_empty() {
            return Err(self.error("S122", frame::GROUPED_ROWS_ERROR, index.span));
        }
        let schema = match row {
            Type::Record(schema) => schema,
            Type::BoundedVariable(_, bound) => match bound.as_ref() {
                TypeBound::Type(ty) => match ty.as_ref() {
                    Type::Record(schema) => schema,
                    _ => return Err(self.error("S122", "DataFrame column access requires a known record schema or a record constraint", span)),
                },
                _ => return Err(self.error("S122", "DataFrame column access requires a record constraint", span)),
            },
            _ => return Err(self.error("S122", "DataFrame column access requires a known record schema or a record constraint", span)),
        };
        let result = match idx {
            Type::String | Type::StringLiteral(_) => {
                let names =
                    names(index).map_err(|message| self.error("S122", message, index.span))?;
                schema
                    .get(&names[0])
                    .cloned()
                    .map(|ty| Type::Series(Box::new(ty)))
                    .ok_or_else(|| frame::unknown_column(schema, &names[0]))
            }
            Type::List(inner, _) if Type::String.accepts(inner) => names(index)
                .and_then(|names| frame::selected_schema(schema, &names))
                .map(|schema| frame_type(schema, Vec::new())),
            Type::Series(inner) if **inner == Type::Bool => Ok(Type::DataFrame {
                row: Box::new(row.clone()),
                groups: groups.to_vec(),
            }),
            _ => Err(format!(
                "DataFrame indexing expects a literal column String, List<String>, or Series<Bool> filter; found {idx}"
            )),
        };
        result.map_err(|message| self.error("S122", message, index.span))
    }
    pub(super) fn frame_native(
        &mut self,
        expr: &Expr,
        builtin: &prelude::Builtin,
        args: &[(Option<String>, Expr)],
        env: &Environment,
        variables: &BTreeMap<String, Type>,
    ) -> Result<Type, Diagnostic> {
        let params = builtin
            .parameters
            .iter()
            .map(|p| (p.name, p.ty.clone(), p.required))
            .collect::<Vec<_>>();
        let ordered = self.arguments(args, &params, expr.span)?;
        let mut actual = BTreeMap::new();
        let mut syntax = BTreeMap::new();
        for (parameter, arg) in builtin.parameters.iter().zip(ordered) {
            if let Some(arg) = arg {
                let ty = self.expression(arg, env, None, variables)?;
                self.expect(&parameter.ty, &ty, arg.span)?;
                actual.insert(parameter.name, ty);
                syntax.insert(parameter.name, arg);
            }
        }
        let result = (|| -> Result<Type, String> {
            if builtin.name == "DataFrame" {
                let Type::List(row, _) = &actual["rows"] else {
                    return Err(format!(
                        "DataFrame requires List<Row>, found {}",
                        actual["rows"]
                    ));
                };
                let Type::Record(schema) = row.as_ref() else {
                    return Err("DataFrame rows require a known record schema; annotate empty input as List<Row>".into());
                };
                frame::validate_schema(schema)?;
                return Ok(Type::DataFrame {
                    row: row.clone(),
                    groups: Vec::new(),
                });
            }
            let operation = builtin
                .name
                .strip_prefix("DataFrame.")
                .expect("DataFrame method");
            let receiver = &actual["frame"];
            if !crate::dataframe::methods_for(receiver).any(|method| method.name == builtin.name) {
                return Err(
                    if matches!(receiver, Type::DataFrame { groups, .. } if !groups.is_empty()) {
                        "groupby creates a grouped view; aggregate it with agg(), mean(), sum(), count(), min(), or max() before selecting, sorting, merging, or materializing rows".into()
                    } else {
                        format!("{receiver} does not support {operation}")
                    },
                );
            }
            if let Type::Series(inner) = receiver {
                return match operation {
                    "to_list" => Ok(Type::List(inner.clone(), sanim_core::ListLength::Unknown)),
                    "isna" => Ok(Type::Series(Box::new(Type::Bool))),
                    "fillna" => {
                        let inner = if let Type::Option(inner) = inner.as_ref() {
                            inner.as_ref()
                        } else {
                            inner.as_ref()
                        };
                        let replacement = if let Type::Series(inner) = &actual["value"] {
                            inner.as_ref()
                        } else {
                            &actual["value"]
                        };
                        if !inner.accepts(replacement) {
                            return Err(format!("fillna requires {inner}, found {replacement}"));
                        }
                        Ok(Type::Series(Box::new(inner.clone())))
                    }
                    "mean" | "sum" | "count" | "min" | "max" => {
                        frame::aggregate_type(operation, inner)
                    }
                    _ => Err(format!(
                        "Series.{operation} is unsupported; use arithmetic, isna(), fillna(), a reduction, or to_list()"
                    )),
                };
            }
            let Type::DataFrame { row, groups } = receiver else {
                return Err(format!(
                    "{operation} requires a DataFrame or Series, found {receiver}"
                ));
            };
            match operation {
                "to_list" => return Ok(Type::List(row.clone(), sanim_core::ListLength::Unknown)),
                "head" | "tail" => return Ok(receiver.clone()),
                _ => {}
            }
            let Type::Record(schema) = row.as_ref() else {
                return Err("this DataFrame operation requires a known record schema; schema-preserving head(), tail(), and to_list() also support generic rows".into());
            };
            let selected = |key| names(syntax[key]);
            let result = match operation {
                "select" => frame::selected_schema(schema, &selected("columns")?)?,
                "drop" => {
                    let removed = selected("columns")?;
                    frame::selected_schema(schema, &removed)?;
                    frame::selected_schema(
                        schema,
                        &schema
                            .keys()
                            .filter(|name| !removed.contains(name))
                            .cloned()
                            .collect::<Vec<_>>(),
                    )?
                }
                "rename" => frame::renamed_schema(schema, &mapping(syntax["columns"])?)?,
                "assign" => {
                    let Type::Record(values) = &actual["columns"] else {
                        return Err(
                            "assign requires a named record of scalar/Series expressions".into(),
                        );
                    };
                    frame::assigned_schema(schema, values)?
                }
                "sort_values" => {
                    frame::selected_schema(schema, &selected("by")?)?;
                    return Ok(receiver.clone());
                }
                "groupby" => {
                    let groups = selected("by")?;
                    if groups.is_empty() {
                        return Err("groupby requires at least one column".into());
                    }
                    frame::selected_schema(schema, &groups)?;
                    return Ok(frame_type(schema.clone(), groups));
                }
                "agg" => frame::aggregated_schema(schema, groups, &mapping(syntax["operations"])?)?,
                "mean" | "sum" | "count" | "min" | "max" => {
                    let operations = frame::default_aggregations(schema, groups, operation)?;
                    frame::aggregated_schema(schema, groups, &operations)?
                }
                "merge" => {
                    let Type::DataFrame {
                        row: right,
                        groups: right_groups,
                    } = &actual["right"]
                    else {
                        return Err("merge right must be a DataFrame".into());
                    };
                    if !groups.is_empty() || !right_groups.is_empty() {
                        return Err("aggregate grouped views before merging".into());
                    }
                    let Type::Record(right) = right.as_ref() else {
                        return Err("merge requires record schemas".into());
                    };
                    let how = syntax
                        .get("how")
                        .map(|expr| names(expr))
                        .transpose()?
                        .map(|values| values[0].clone())
                        .unwrap_or_else(|| "inner".into());
                    let suffixes = syntax
                        .get("suffixes")
                        .map(|expr| names(expr))
                        .transpose()?
                        .unwrap_or_else(|| vec!["_x".into(), "_y".into()]);
                    if suffixes.len() != 2 {
                        return Err("merge suffixes requires exactly two Strings".into());
                    }
                    frame::merged_schema(
                        schema,
                        right,
                        &selected("on")?,
                        &how,
                        [&suffixes[0], &suffixes[1]],
                    )?
                    .0
                }
                "dropna" => {
                    if let Some(subset) = syntax.get("subset") {
                        frame::selected_schema(schema, &names(subset)?)?;
                    }
                    return Ok(receiver.clone());
                }
                _ => {
                    return Err(format!(
                        "DataFrame.{operation} is unsupported; isna/fillna act on a column Series"
                    ));
                }
            };
            Ok(frame_type(result, Vec::new()))
        })();
        result.map_err(|message| self.error("S125", message, expr.span))
    }
}
