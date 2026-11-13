//! Ordered native dispatch, shared asset caches, and bounded host work.
use super::*;

impl Runtime<'_, '_> {
    pub(super) fn native(
        &mut self,
        name: &str,
        arguments: Vec<(Option<String>, Value)>,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        self.list_edit = None;
        let builtin = prelude::builtin(name).expect("checked builtin");
        let duration = builtin
            .timing
            .value(builtin.duration_argument(&arguments))
            .map_err(|e| self.error("S304", e, span))?;
        if duration > Fraction::from(0) && !self.root_active {
            return Err(self.error(
                "S115",
                "imported modules cannot schedule animation while being imported",
                span,
            ));
        }
        let start = self.cursor;
        let easing = crate::timing::easing(builtin.easing_argument(&arguments))
            .map_err(|error| self.error("S304", error, span))?;
        let mut controls = [
            builtin.duration_argument(&arguments),
            builtin.easing_argument(&arguments),
        ]
        .into_iter()
        .flatten()
        .filter_map(|value| {
            arguments
                .iter()
                .position(|(_, candidate)| std::ptr::eq(value, candidate))
        })
        .collect::<Vec<_>>();
        controls.sort_unstable();
        controls.dedup();
        let mut arguments = arguments;
        for index in controls.into_iter().rev() {
            arguments.remove(index);
        }
        let value = self.native_inner(builtin, arguments, duration, easing, span)?;
        if builtin.result == Type::Element {
            self.record_element_source(&value, span)?;
        }
        self.finish_call_duration(start, Some(duration), self.cursor, span)?;
        self.last_call.easing = easing;
        Ok(value)
    }
    fn native_inner(
        &mut self,
        builtin: &prelude::Builtin,
        arguments: Vec<(Option<String>, Value)>,
        duration: Fraction,
        easing: Easing,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let name = builtin.name;
        if builtin.schedule && !self.root_active {
            return Err(self.error("S115","imported modules cannot schedule or assert while being imported; call a helper from the root script instead",span));
        }
        let arguments = if name == "DataFrame.assign" {
            let mut arguments = arguments.into_iter();
            let receiver = arguments.next();
            let mut normalized =
                crate::dataframe::named_assignments(arguments.collect(), |columns| {
                    Value::Record(columns.into_iter().collect())
                });
            if let Some(receiver) = receiver {
                normalized.insert(0, receiver)
            }
            normalized
        } else {
            arguments
        };
        let params = builtin
            .parameters
            .iter()
            .map(|v| v.name.to_string())
            .collect::<Vec<_>>();
        let mut args = order_values(&arguments, &params);
        if name == "Image"
            && let Some(src) = args.get("src")
        {
            let value = self.read_media_resource(
                src.string().map_err(|e| self.error("S305", e, span))?,
                span,
            )?;
            args.insert("asset".into(), value);
        }
        if matches!(name, "CodeView" | "CodeView.patch")
            && let Some(src) = args.get("src")
        {
            let language = if name == "CodeView" {
                let language = args
                    .get("lang")
                    .map(Value::string)
                    .transpose()
                    .map_err(|e| self.error("S305", e, span))?
                    .unwrap_or("auto");
                sanim_core::code::Language::parse(language)
                    .map_err(|e| self.error("S305", e, span))?
            } else {
                sanim_core::code::Language::Auto
            };
            let document = self.read_resource(
                src.string().map_err(|e| self.error("S305", e, span))?,
                &Type::Code(language),
                span,
            )?;
            args.insert(
                if name == "CodeView" { "document" } else { "to" }.into(),
                document,
            );
        }
        for parameter in builtin.parameters.iter().filter(|parameter| {
            parameter.name == "font" && parameter.ty == Type::Font && !parameter.required
        }) {
            args.entry(parameter.name.to_owned()).or_insert_with(|| {
                Value::Font(if name == "CodeView" {
                    sanim_core::Font::family("monospace", "monospace")
                        .expect("native monospace family")
                } else {
                    sanim_core::Font::default()
                })
            });
        }
        if let Some(request) = crate::dataframe::materialization(name, &args)
            .map_err(|message| self.error("S309", message, span))?
        {
            return self.materialize_frame(request, span);
        }
        if matches!(builtin.execute, prelude::Execution::Read) {
            let ty = self
                .loaded
                .iter()
                .find(|v| v.id == self.module)
                .and_then(|v| v.checked.read_types.get(&span.start))
                .ok_or_else(|| self.error("S301", "missing read schema", span))?
                .substitute(&self.type_bindings);
            let path = args
                .get("path")
                .ok_or_else(|| self.error("S305", "missing path", span))?
                .string()
                .map_err(|e| self.error("S305", e, span))?;
            return self.read_resource(path, &ty, span);
        }
        if matches!(
            builtin.execute,
            prelude::Execution::Decode | prelude::Execution::Query
        ) {
            let ty = self
                .loaded
                .iter()
                .find(|v| v.id == self.module)
                .and_then(|v| v.checked.read_types.get(&span.start))
                .ok_or_else(|| self.error("S301", "missing prepared data schema", span))?
                .substitute(&self.type_bindings);
            return if matches!(builtin.execute, prelude::Execution::Decode) {
                let resource = format!("{}:{}", self.module, span.line);
                self.progress
                    .emit(Phase::Decoding, None, None, Some(&resource));
                let data =
                    sanim_data::to_json(&args["value"]).map_err(|e| self.error("S306", e, span))?;
                let value = sanim_data::validate(&data, &ty, self.limits.data)
                    .map_err(|e| self.error("S306", format!("decode as {ty}: {e}"), span))?;
                self.report_evaluation(Some(span));
                Ok(value)
            } else {
                let sql = self.sql(&args["sql"], span)?;
                if !args.contains_key("tables")
                    || matches!(args.get("tables"),Some(Value::Dict(tables)) if tables.is_empty())
                {
                    args.insert("tables".into(), Value::Record(Default::default()));
                }
                let key = digest(
                    &serde_json::to_vec(&(
                        "sqlite",
                        SEMANTICS_VERSION,
                        &ty,
                        self.limits.data,
                        &sql.text,
                        &sql.parameters,
                        &args["tables"],
                    ))
                    .map_err(|e| self.error("S301", e.to_string(), span))?,
                );
                self.query_cached(
                    key,
                    digest(
                        &serde_json::to_vec(&(&args["tables"], self.limits.data))
                            .map_err(|error| self.error("S301", error.to_string(), span))?,
                    ),
                    sanim_data::sqlite::Request {
                        result: &ty,
                        sql: &sql.text,
                        tables: &args["tables"],
                        parameters: Some(&sql.parameters),
                        limits: self.limits.data,
                    },
                    span,
                )
            };
        }
        if self.duration_only && builtin.schedule {
            return Ok(Value::Unit);
        }
        if matches!(name, "assert" | "check") {
            let condition = args
                .remove("condition")
                .ok_or_else(|| self.error("S305", "missing assertion condition", span))?;
            let message = args
                .get("message")
                .map(Value::string)
                .transpose()
                .map_err(|e| self.error("S305", e, span))?
                .map(str::to_string);
            match condition {
                Value::Bool(value) => {
                    let verification = Verification {
                        evidence: if value {
                            verify::Evidence::Proved
                        } else {
                            verify::Evidence::Refuted
                        },
                        message: message.unwrap_or_else(|| "boolean assertion must be true".into()),
                        time: Some(self.cursor),
                    };
                    if !value && name == "assert" {
                        return Err(self.error("S350", verification.message, span));
                    }
                    self.checks.push_back(verification);
                }
                Value::Predicate(predicate) => self.assertions.push_back(Assertion {
                    predicate: predicate.as_ref().clone(),
                    time: self.cursor,
                    required: name == "assert",
                    module: self.module.clone(),
                    span,
                    message,
                }),
                _ => {
                    return Err(self.error(
                        "S305",
                        "assert/check requires a boolean or relationship",
                        span,
                    ));
                }
            }
            return Ok(Value::Unit);
        }
        let (prelude::Execution::Native(evaluate)
        | prelude::Execution::Source(evaluate)
        | prelude::Execution::Access(evaluate)) = builtin.execute
        else {
            unreachable!("registered host operation was handled above")
        };
        if name == "Text" {
            let resource = format!("{}:{}", self.module, span.line);
            self.progress
                .emit(Phase::Shaping, None, None, Some(&resource));
        }
        let native_started = Instant::now();
        let mut list_edit = None;
        let mut created = Vec::new();
        let mut native_work = 0;
        let result = evaluate(
            name,
            &args,
            crate::native::Context {
                module: &self.module,
                identity: &self.identity,
                span,
                animation: &mut self.animation,
                cursor: &mut self.cursor,
                limits: self.limits,
                retained_bytes: &mut self.retained_bytes,
                duration,
                easing,
                list_edit: &mut list_edit,
                created: &mut created,
                native_work: &mut native_work,
                remaining_work: self
                    .limits
                    .max_operations
                    .saturating_sub(self.logical_operations),
            },
        )
        .map_err(|e| self.error("S305", e, span));
        self.charge_work(native_work, span)?;
        if result.is_ok() {
            for value in created {
                self.retain_binding_value(&value, span)?;
                self.record_element_source(&value, span)?;
            }
            if builtin.schedule {
                self.check_elements(span)?;
            }
            self.list_edit = list_edit.map(|delta| crate::lists::EditReceipt {
                module: self.module.clone(),
                span,
                delta,
            });
        }
        self.statistics.native_micros = self.statistics.native_micros.saturating_add(
            native_started
                .elapsed()
                .as_micros()
                .try_into()
                .unwrap_or(u64::MAX),
        );
        if name == "Text" && result.is_ok() {
            self.report_evaluation(Some(span));
        }
        result
    }
}
