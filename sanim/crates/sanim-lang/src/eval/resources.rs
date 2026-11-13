//! One source-read boundary for data imports and watched native element assets.
use super::*;

impl Runtime<'_, '_> {
    pub(super) fn read_resource(
        &mut self,
        path: &str,
        ty: &Type,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let path = source::resolve(&self.module, path).map_err(|e| self.error("S201", e, span))?;
        let bytes = self
            .snapshot
            .read(&path)
            .map_err(|e| self.error("S201", e, span))?;
        let version = self.snapshot.fingerprint(&path);
        self.used_resources
            .entry(path.clone())
            .or_insert((version.clone(), self.root_statement));
        let key = digest(
            &serde_json::to_vec(&(SEMANTICS_VERSION, &path, ty, self.limits.data, version))
                .map_err(|e| self.error("S301", e.to_string(), span))?,
        );
        if let Some(value) = self.assets.get(&key) {
            self.statistics.reused_assets += 1;
            return Ok(value);
        }
        self.progress.emit(Phase::Decoding, None, None, Some(&path));
        let value = sanim_data::decode(&path, &bytes, ty, self.limits.data)
            .map_err(|e| self.error("S306", format!("reading '{path}' as {ty:?}: {e}"), span))?;
        self.statistics.decoded_assets += 1;
        self.assets.insert(key, value.clone());
        self.report_evaluation(Some(span));
        Ok(value)
    }
}
