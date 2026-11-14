//! Estimate canonical AST payloads without a second syntax walker or a serialized buffer.
use crate::ast::Module;

pub(crate) fn retained_bytes(
    module: &Module,
    source_bytes: usize,
    key_bytes: usize,
    limit: usize,
) -> Option<usize> {
    // Source payloads plus serialized structure account for strings, types,
    // generated arguments and nested declarations without allocating a duplicate AST.
    let roots = std::mem::size_of::<Module>()
        .checked_add(
            module
                .statements
                .capacity()
                .checked_mul(std::mem::size_of::<crate::ast::Statement>())?,
        )?
        .checked_add(
            module
                .comments
                .capacity()
                .checked_mul(std::mem::size_of::<sanim_core::Span>())?,
        )?
        .checked_add(source_bytes)?
        .checked_add(key_bytes.checked_mul(2)?)?
        .checked_add(2 * std::mem::size_of::<String>())?;
    super::size::retained(module, roots, limit)
}

#[cfg(test)]
mod tests {
    use crate::{Engine, MemorySources};
    use std::{collections::BTreeMap, sync::Arc};

    #[test]
    fn failed_type_edits_evict_old_parses_under_entry_and_byte_limits() {
        let mut engine = Engine::default();
        engine.limits.max_cache_bytes = 24 * 1024;
        engine.limits.max_cache_entries = 128;
        let mut files = MemorySources::default();
        let mut first = None;
        for index in 0..128 {
            files.insert(
                "main.sanim",
                format!("let value={index}\nlet broken:Color=1"),
            );
            let error = engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap_err();
            assert_eq!(error.code, "S106", "this must exercise post-parse failures");
            let (bytes, entries) = engine.parsed.usage();
            assert!(bytes <= engine.limits.max_cache_bytes);
            assert!(entries <= engine.limits.max_cache_entries);
            if index == 0 {
                let hash = key(&engine, &files);
                let parsed = engine.parsed.get(&hash).unwrap();
                first = Some(Arc::downgrade(&parsed));
            }
        }
        assert!(
            first.unwrap().upgrade().is_none(),
            "failed revisions must release their ASTs without GC"
        );
        let (bytes, entries) = engine.parsed.usage();
        assert!(
            bytes > 0 && entries > 0 && entries < 128,
            "the byte limit must evict admitted ASTs"
        );
        engine.limits.max_cache_entries = 3;
        assert!(
            engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .is_err()
        );
        assert!(engine.parsed.usage().1 <= 3);
        engine.limits.max_cache_entries = 0;
        assert!(
            engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .is_err()
        );
        assert_eq!(engine.parsed.usage(), (0, 0));
    }
    fn key(engine: &Engine, files: &MemorySources) -> String {
        use crate::Sources;
        let (_, hash) = files.read_snapshot("main.sanim").unwrap();
        sanim_core::digest(&serde_json::to_vec(&(hash, engine.limits.max_parse_steps)).unwrap())
    }
}
