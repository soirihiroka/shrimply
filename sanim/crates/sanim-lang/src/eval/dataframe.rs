use super::*;

impl Runtime<'_, '_> {
    pub(super) fn checked_native_method(&self, span: Span) -> Option<&crate::NativeMethod> {
        self.loaded
            .iter()
            .find(|module| module.id == self.module)?
            .checked
            .native_methods
            .get(&(span.start, span.end))
    }
    pub(super) fn frame_index(
        &self,
        frame: &sanim_core::dataframe::Frame,
        index: Value,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        if !frame.groups.is_empty() {
            return Err(self.error("S309", sanim_core::dataframe::GROUPED_ROWS_ERROR, span));
        }
        let result = match index {
            Value::String(name) => frame
                .column(&name)
                .map(|series| Value::Series(std::sync::Arc::new(series))),
            Value::Series(mask) => frame
                .filter(&mask)
                .map(|frame| Value::DataFrame(std::sync::Arc::new(frame))),
            Value::List(names) => names
                .iter()
                .map(|name| name.string().map(str::to_owned))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|names| frame.select(&names))
                .map(|frame| Value::DataFrame(std::sync::Arc::new(frame))),
            value => Err(format!(
                "DataFrame indexing expects a column name, column list, or Series<Bool> mask; found {}",
                value.ty()
            )),
        };
        result.map_err(|message| self.error("S309", message, span))
    }
    pub(super) fn materialize_frame(
        &mut self,
        request: crate::dataframe::Materialization,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let key = digest(
            &serde_json::to_vec(&(
                "dataframe",
                SEMANTICS_VERSION,
                &request.frame.fingerprint,
                request.series.as_ref().map(|series| &series.expression),
                self.limits.data,
            ))
            .map_err(|error| self.error("S301", error.to_string(), span))?,
        );
        let value = if let Some(value) = self.assets.get(&key) {
            self.statistics.reused_queries += 1;
            value.clone()
        } else {
            let query = sanim_data::dataframe::compile(
                &request.frame,
                request.series.as_ref(),
                self.limits.data,
            )
            .map_err(|message| self.error("S309", message, span))?;
            self.query_cached(
                key,
                query.input_fingerprint,
                sanim_data::sqlite::Request {
                    result: &query.result,
                    sql: &query.sql,
                    tables: &query.tables,
                    parameters: Some(&query.parameters),
                    limits: self.limits.data,
                },
                span,
            )?
        };
        if request.scalar {
            value
                .list()
                .map_err(|message| self.error("S309", message, span))?
                .front()
                .cloned()
                .ok_or_else(|| {
                    self.error("S309", "SQLite reduction returned no scalar result", span)
                })
        } else {
            Ok(value)
        }
    }
    pub(super) fn query_cached(
        &mut self,
        key: String,
        input_key: String,
        request: sanim_data::sqlite::Request<'_>,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        if let Some(value) = self.assets.get(&key) {
            self.statistics.reused_queries += 1;
            return Ok(value.clone());
        }
        let resource = format!("{}:{}", self.module, span.line);
        self.progress
            .emit(Phase::Querying, None, None, Some(&resource));
        let started = std::time::Instant::now();
        let execution = self
            .databases
            .query(input_key, request)
            .map_err(|e| self.error("S309", e, span))?;
        self.charge_work(execution.instructions, span)?;
        self.statistics.sql_instructions = self
            .statistics
            .sql_instructions
            .saturating_add(execution.instructions);
        let value = execution.value;
        let elapsed = started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
        self.statistics.query_micros = self.statistics.query_micros.saturating_add(elapsed);
        if elapsed > self.limits.warn_prepare_micros {
            self.performance.push(PerformanceWarning{code:"P003".into(),message:format!("SQLite query took {elapsed}us (warning budget {}us), returning {} rows. Filter before joining or grouping, reduce input tables, or reuse a prepared result outside loops.",self.limits.warn_prepare_micros,value.list().map_or(0,|rows|rows.len())),source:self.module.clone(),span,operations:execution.instructions});
        }
        self.statistics.executed_queries += 1;
        self.statistics.reused_databases += u64::from(execution.reused_database);
        self.assets.insert(key, value.clone());
        self.report_evaluation(Some(span));
        Ok(value)
    }
}
