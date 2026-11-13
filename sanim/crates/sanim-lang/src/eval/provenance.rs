//! Constructor evidence is separate from geometry, so source edits do not evict draw caches.
use super::*;

impl Runtime<'_, '_> {
    pub(super) fn restore_element_sources(
        &mut self,
        entry: &crate::cache::CachedValue,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if entry.sources.is_empty() || self.duration_only {
            return Ok(());
        }
        let mut bytes = 0usize;
        for (id, source) in &entry.sources {
            if !self.element_sources.contains_key(id) {
                // Pure-expression caches may be reused from a different helper call location.
                let source = if source.call_sites == self.source_calls {
                    source.clone()
                } else {
                    Arc::new(crate::ElementSource {
                        creation: source.creation.clone(),
                        call_sites: self.source_calls.clone(),
                    })
                };
                bytes = bytes.saturating_add(
                    id.len()
                        + std::mem::size_of::<String>()
                        + std::mem::size_of::<Arc<crate::ElementSource>>()
                        + source.retained_bytes(),
                );
                self.element_sources.insert(id.clone(), source);
            }
        }
        if self.element_sources.len() > self.limits.max_elements {
            return Err(self.error(
                "S303",
                "cached element source metadata exceeds the element budget",
                span,
            ));
        }
        self.retained_bytes = self.retained_bytes.checked_sub(bytes).ok_or_else(|| {
            self.error(
                "S303",
                "cached element source metadata exceeds the preparation byte budget",
                span,
            )
        })?;
        Ok(())
    }

    pub(super) fn source_location(&mut self, span: Span) -> crate::SourceLocation {
        crate::SourceLocation {
            source: self
                .source_ids
                .entry(self.module.clone())
                .or_insert_with(|| Arc::from(self.module.as_str()))
                .clone(),
            span,
        }
    }

    pub(super) fn record_element_source(
        &mut self,
        value: &Value,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let Value::Element(element) = value else {
            return Ok(());
        };
        if self.duration_only || self.element_sources.contains_key(&element.id) {
            return Ok(());
        }
        let source = Arc::new(crate::ElementSource {
            creation: self.source_location(span),
            call_sites: self.source_calls.clone(),
        });
        let mut bytes = source.retained_bytes();
        let mut pending: Vec<&Element> = vec![element.as_ref()];
        while let Some(element) = pending.pop() {
            // Existing child objects keep their constructor location when grouped or plotted.
            if self.element_sources.contains_key(&element.id) {
                continue;
            }
            bytes = bytes.saturating_add(
                element.id.len()
                    + std::mem::size_of::<String>()
                    + std::mem::size_of::<Arc<crate::ElementSource>>(),
            );
            self.element_sources
                .insert(element.id.clone(), source.clone());
            if self.element_sources.len() > self.limits.max_elements {
                return Err(self.error(
                    "S303",
                    "element source metadata exceeds the element budget",
                    span,
                ));
            }
            if let Some(children) = element.geometry.children() {
                pending.extend(children);
            }
        }
        self.retained_bytes = self.retained_bytes.checked_sub(bytes).ok_or_else(|| {
            self.error(
                "S303",
                "element source metadata exceeds the preparation byte budget",
                span,
            )
        })?;
        Ok(())
    }
}
