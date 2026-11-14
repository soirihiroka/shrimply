//! Immutable checked modules are reused only with the same complete static inputs.
use crate::checker::Checked;
use std::sync::Arc;

pub(crate) struct Module {
    pub checked: Arc<Checked>,
    /// Replaying this charge preserves cold/warm compiler limit behavior.
    pub type_bytes: usize,
    pub interface_key: String,
}
pub(crate) fn retained_bytes(checked: &Checked, key: &str, limit: usize) -> Option<usize> {
    super::size::retained(
        checked,
        std::mem::size_of::<Module>().checked_add(key.len().checked_mul(3)?)?,
        limit,
    )
}

#[cfg(test)]
mod tests {
    use crate::{Engine, MemorySources};
    use std::{collections::BTreeMap, sync::Arc};

    #[test]
    fn edits_share_unmodified_checks_and_only_interfaces_invalidate_consumers() {
        let mut files = MemorySources::default();
        files.insert("lib.sanim", "fn amount():Number{return 1}");
        files.insert(
            "main.sanim",
            "import \"lib\" as lib\nlet value=lib.amount()+1",
        );
        let mut engine = Engine::default();
        let cold = engine.lint("main.sanim", &files).unwrap();
        assert_eq!(cold.statistics.checked_modules, 2);
        let main = cold.modules["main.sanim"].checked.clone();
        let library = cold.modules["lib.sanim"].checked.clone();
        let warm = engine.lint("main.sanim", &files).unwrap();
        assert_eq!(warm.statistics.reused_checks, 2);
        assert_eq!(warm.statistics.checked_modules, 0);
        assert!(Arc::ptr_eq(&main, &warm.modules["main.sanim"].checked));
        files.insert(
            "main.sanim",
            "import \"lib\" as lib\nlet value=lib.amount()+2",
        );
        let edit = engine.lint("main.sanim", &files).unwrap();
        assert_eq!(
            (
                edit.statistics.checked_modules,
                edit.statistics.reused_checks
            ),
            (1, 1)
        );
        assert!(Arc::ptr_eq(&library, &edit.modules["lib.sanim"].checked));

        files.insert("lib.sanim", "fn amount():Number{return 2}");
        let body = engine.lint("main.sanim", &files).unwrap();
        assert_eq!(
            (
                body.statistics.checked_modules,
                body.statistics.reused_checks
            ),
            (1, 1)
        );
        assert!(Arc::ptr_eq(
            &edit.modules["main.sanim"].checked,
            &body.modules["main.sanim"].checked
        ));
        let first = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(first.bindings["value"].number().unwrap().value, 4.into());
        files.insert("lib.sanim", "fn amount():Number{return 3}");
        let next = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(next.bindings["value"].number().unwrap().value, 5.into());
        files.insert("lib.sanim", "fn amount():String{return \"changed\"}");
        assert_eq!(engine.lint("main.sanim", &files).unwrap_err().code, "S106");
    }

    #[test]
    fn cached_checks_preserve_root_rules_limits_and_aggregate_budget() {
        let mut files = MemorySources::default();
        files.insert("lib.sanim", "wait(1s)");
        files.insert("main.sanim", "import \"lib\" as lib");
        let mut engine = Engine::default();
        engine.lint("lib.sanim", &files).unwrap();
        assert!(engine.lint("main.sanim", &files).is_err());

        files.insert(
            "lib.sanim",
            "fn identity(value:Number):Number{return value}",
        );
        files.insert(
            "main.sanim",
            "import \"lib\" as lib\nlet result=lib.identity(3)",
        );
        engine.lint("main.sanim", &files).unwrap();
        engine.limits.max_type_bytes = 100;
        let warm = engine.lint("main.sanim", &files).unwrap_err();
        let mut fresh = Engine {
            limits: engine.limits.clone(),
            ..Engine::default()
        };
        let cold = fresh.lint("main.sanim", &files).unwrap_err();
        assert_eq!((warm.code, warm.span), (cold.code, cold.span));
        assert_eq!(warm.message, cold.message);
    }

    #[test]
    fn body_only_import_edits_preserve_warm_and_cold_budget_parity() {
        fn cost(
            source: &str,
            text: &str,
            imports: &BTreeMap<String, Arc<crate::CheckedModule>>,
            root: bool,
        ) -> (Arc<crate::CheckedModule>, usize) {
            let limits = crate::Limits::default();
            let module = crate::parser::parse(source, text, limits.max_parse_steps).unwrap();
            let budget = std::cell::Cell::new(limits.max_type_bytes);
            let checked =
                crate::checker::check(source, &module, imports, root, &budget, &limits).unwrap();
            (Arc::new(checked), limits.max_type_bytes - budget.get())
        }
        let old = "fn value():Number{return 1}";
        let new = format!(
            "fn value():Number{{{}return 1}}",
            (0..100)
                .map(|i| format!("let local_{i}={i};"))
                .collect::<String>()
        );
        let main = "import \"lib\" as lib\nlet result=lib.value()";
        let (old_checked, old_library_cost) = cost("lib.sanim", old, &BTreeMap::new(), false);
        let (_, new_library_cost) = cost("lib.sanim", &new, &BTreeMap::new(), false);
        let (_, old_consumer_cost) = cost(
            "main.sanim",
            main,
            &BTreeMap::from([("lib".into(), old_checked)]),
            true,
        );
        assert!(
            new_library_cost > old_library_cost,
            "the edit must change actual library checking work"
        );
        let limits = crate::Limits {
            max_type_bytes: new_library_cost + old_consumer_cost,
            ..crate::Limits::default()
        };
        let mut files = MemorySources::default();
        files.insert("main.sanim", main);
        files.insert("lib.sanim", old);
        let mut warm = Engine {
            limits: limits.clone(),
            ..Engine::default()
        };
        warm.lint("main.sanim", &files).unwrap();
        files.insert("lib.sanim", new);
        let warm = warm.lint("main.sanim", &files);
        let cold = Engine {
            limits,
            ..Engine::default()
        }
        .lint("main.sanim", &files);
        assert_eq!(
            warm.is_ok(),
            cold.is_ok(),
            "body-only edits must not bypass aggregate limits on a cache hit: warm={warm:?}, cold={cold:?}"
        );
        if let (Err(warm), Err(cold)) = (warm, cold) {
            assert_eq!(
                (warm.code, warm.span, warm.message),
                (cold.code, cold.span, cold.message)
            );
        }
    }

    #[test]
    fn edit_versions_and_disabled_caches_release_checked_roots() {
        let mut engine = Engine::default();
        engine.limits.max_cache_entries = 2;
        engine.limits.max_cache_bytes = 64 * 1024;
        let mut files = MemorySources::default();
        files.insert("main.sanim", "let first=1");
        let first = engine.lint("main.sanim", &files).unwrap();
        let weak = Arc::downgrade(&first.modules["main.sanim"].checked);
        drop(first);
        for index in 0..64 {
            files.insert("main.sanim", format!("let value={index}"));
            engine.lint("main.sanim", &files).unwrap();
            assert!(engine.checked.usage().0 <= engine.limits.max_cache_bytes);
            assert!(engine.checked.usage().1 <= engine.limits.max_cache_entries);
        }
        assert!(weak.upgrade().is_none());
        engine.limits.max_cache_entries = 0;
        let disabled = engine.lint("main.sanim", &files).unwrap();
        assert_eq!(disabled.statistics.reused_checks, 0);
        assert_eq!(engine.checked.usage(), (0, 0));
    }
}
