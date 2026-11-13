//! Estimate canonical AST payloads without a second syntax walker or a serialized buffer.
use crate::ast::Module;
use std::io::{self, Write};

pub(crate) fn retained_bytes(
    module: &Module,
    source_bytes: usize,
    key_bytes: usize,
    limit: usize,
) -> Option<usize> {
    // Serialized nodes do not include Rust enum discriminants or spare Vec capacity.
    // Keep a conservative accounting margin; this is an estimate, not allocator telemetry.
    const AST_METADATA_SLACK: usize = 4;
    struct Count {
        bytes: usize,
        remaining: usize,
    }
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let charged = bytes
                .len()
                .checked_mul(AST_METADATA_SLACK)
                .ok_or_else(|| io::Error::other("parsed cache estimate overflow"))?;
            self.remaining = self
                .remaining
                .checked_sub(charged)
                .ok_or_else(|| io::Error::other("parsed cache byte budget reached"))?;
            self.bytes += charged;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
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
    let mut count = Count {
        bytes: roots,
        remaining: limit.checked_sub(roots)?,
    };
    serde_json::to_writer(&mut count, module).ok()?;
    Some(count.bytes)
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
