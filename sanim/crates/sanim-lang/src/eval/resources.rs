//! One source-read boundary for data imports and watched native element assets.
use super::*;

#[derive(serde::Serialize)]
enum ResourceKind<'a> {
    Data(&'a Type),
    Media,
}

impl Runtime<'_, '_> {
    pub(super) fn read_resource(
        &mut self,
        path: &str,
        ty: &Type,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        self.load_resource(path, ResourceKind::Data(ty), span)
    }
    pub(super) fn read_media_resource(
        &mut self,
        path: &str,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        self.load_resource(path, ResourceKind::Media, span)
    }
    fn load_resource(
        &mut self,
        path: &str,
        kind: ResourceKind<'_>,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let path = if matches!(kind, ResourceKind::Media) && path.contains("://") {
            shrimply_asset::remote::canonical_url(path)
        } else {
            source::resolve(&self.module, path)
        }
        .map_err(|e| self.error("S201", e, span))?;
        self.progress
            .emit(Phase::Reading, Some(0), None, Some(&path));
        let bytes = self
            .snapshot
            .read(&path)
            .map_err(|e| self.error("S201", e, span))?;
        self.progress.emit(
            Phase::Reading,
            Some(bytes.len() as u64),
            Some(bytes.len() as u64),
            Some(&path),
        );
        let version = self.snapshot.fingerprint(&path);
        self.used_resources
            .entry(path.clone())
            .or_insert((version.clone(), self.root_statement));
        let key = digest(
            &serde_json::to_vec(&(SEMANTICS_VERSION, &path, &kind, self.limits.data, version))
                .map_err(|e| self.error("S301", e.to_string(), span))?,
        );
        if let Some(value) = self.assets.get(&key) {
            self.statistics.reused_assets += 1;
            return Ok(value);
        }
        self.progress.emit(Phase::Decoding, None, None, Some(&path));
        let value = match kind {
            ResourceKind::Data(ty) => sanim_data::decode(&path, &bytes, ty, self.limits.data),
            ResourceKind::Media => sanim_data::media::decode(bytes, self.limits.data)
                .and_then(|asset| crate::media::resource(&path, asset)),
        }
        .map_err(|e| self.error("S306", format!("reading '{path}': {e}"), span))?;
        self.statistics.decoded_assets += 1;
        self.assets.insert(key, value.clone());
        self.report_evaluation(Some(span));
        Ok(value)
    }
}
