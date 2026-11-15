use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;
fn run(source: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}
#[test]
fn immutable_captures_and_bounded_local_scratch_are_pure() {
    let prepared=run("let offset=2\nfn sum(n:Number):Number{mut total=offset;repeat n {total=total+1};return total}\nlet answer=sum(40)\nassert(answer==42)").unwrap();
    assert!(
        prepared.symbols["sum"].signatures()[0]
            .capabilities
            .is_pure()
    );
}
#[test]
fn capabilities_are_opt_in_for_mut_sig_ref_and_state_effects() {
    for source in [
        "fn bad(mut x:Number):Number{return x}",
        "fn bad(sig x:Number):Number{return x}",
        "fn bad():sig Number{return 1}",
        "mut x=1\nfn bad():Number{return x}",
        "mut x=1\nfn bad():Unit{x=2}",
        "fn bad():Unit{check(true)}",
        "fn bad():String{return read<String>(\"absent.txt\")}",
        "let shape=Box()\nfn bad():Length{return shape.width}",
    ] {
        let error = run(source).unwrap_err();
        assert_eq!(error.code, "S133", "{source}: {error}");
    }
    run("mut x=1\nfn<ref> read_x():Number{return x}\nfn<mut,ref> bump():Number{x=x+1;return x}\nfn<sig> double(sig value:Number):sig Number{return value*2}\nlet live=double(x)\nassert(read_x()==1)\nlet y=bump()\nassert(y==2)\nassert(live==4)").unwrap();
}
#[test]
fn impure_contracts_do_not_leak_through_aliases_or_pure_callers() {
    for source in [
        "fn<mut> effect():Number{return 1}\nfn bad():Number{return effect()}",
        "fn<mut> effect():Number{return 1}\nlet alias=effect\nfn bad():Number{return alias()}",
        "fn identity<T>(value:T):T{return value}\nfn<mut> effect():Number{return 1}\nlet alias=identity(effect)\nfn bad():Number{return alias()}",
        "fn bad():List<Number>{return [1].map(fn<mut>(x:Number):Number{return x})}",
    ] {
        let error = run(source).unwrap_err();
        assert_eq!(error.code, "S133", "{source}: {error}");
    }
    run("fn pure(value:Number):Number{return value*2}\nfn<mut> caller():Number{wait(1ms);return pure(21)}\nassert(caller()==42)").unwrap();
}
#[test]
fn callable_types_preserve_the_capability_subset_rule() {
    for source in [
        "let value:fn():Number=fn<mut>():Number{return 1}",
        "fn apply(callback:fn():Number):Number{return callback()}\nlet value=apply(fn<mut>():Number{return 1})",
        "fn factory():fn():Number{return fn<ref>():Number{return 1}}",
    ] {
        let error = run(source).unwrap_err();
        assert_eq!(error.code, "S106", "{source}: {error}");
    }
    run("let value:fn<mut,ref>():Number=fn():Number{return 42}\nassert(value()==42)\nfn<mut> apply(callback:fn<mut>():Number):Number{return callback()}\nassert(apply(fn():Number{return 42})==42)").unwrap();
}
#[test]
fn imported_immutable_values_are_pure_and_mutable_exports_need_ref() {
    for (binding, capabilities, expected) in [
        ("let x=42", "", true),
        ("mut x=42", "", false),
        ("mut x=42", "<ref>", true),
    ] {
        let mut files = MemorySources::default();
        files.insert("lib.sanim", binding);
        files.insert("main.sanim",format!("import \"lib\" as lib\nfn{capabilities} read_x():Number{{return lib.x}}\nlet answer=read_x()\nAnimation()"));
        let result = Engine::default().prepare("main.sanim", &files, &BTreeMap::new());
        if expected {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert_eq!(result.unwrap_err().code, "S133");
        }
    }
}
#[test]
fn capability_prefixes_format_and_generics_still_parse() {
    let source = "fn<mut,ref> effect<T>(mut value:T):T{return value}\nlet generic=fn<ref><T>(value:T):T{return value}\nassert(generic(42)==42)";
    let formatted =
        sanim_lang::formatter::format_source("main.sanim", source, Default::default(), 1_000_000)
            .unwrap();
    run(&formatted).unwrap();
}

#[test]
fn repeated_pure_calls_reuse_data_but_fresh_mutable_closures_remain_independent() {
    let prepared=run("let scale=2\nfn sum(n:Number):Number{mut total=0;repeat 100 {total=total+n*scale};return total}\nlet first=sum(3)\nlet second=sum(3)\nassert(first==600 and second==600)\nfn counter():fn<mut,ref>():Number{mut count=0;return fn<mut,ref>():Number{count=count+1;return count}}\nlet a=counter()\nlet b=counter()\nassert(a()==1)\nassert(a()==2)\nassert(b()==1)").unwrap();
    assert!(prepared.statistics.reused_expressions > 0);
}

#[test]
fn memoized_captures_and_transitive_function_versions_invalidate_after_edits() {
    let mut engine = Engine::default();
    let mut files = MemorySources::default();
    let source = |factor, offset| {
        format!(
            "let factor={factor}\nfn helper(n:Number):Number{{return n+{offset}}}\nfn total(n:Number):Number{{mut sum=0;repeat 100{{sum=sum+helper(n)*factor}};return sum}}\nlet first=total(3)\nlet second=total(3)\nAnimation()\n"
        )
    };
    for (factor, offset, expected) in [(2, 0, 600), (3, 0, 900), (3, 1, 1200), (2, 0, 600)] {
        files.insert("main.sanim", source(factor, offset));
        let prepared = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(
            prepared.bindings["second"].number().unwrap().value,
            Fraction::from(expected)
        );
    }
}

#[test]
fn cached_higher_order_calls_track_function_arguments_and_each_closure_instance() {
    let mut engine = Engine::default();
    let mut files = MemorySources::default();
    let source = |increment| {
        format!(
            "fn make(offset:Number):fn():Number{{return fn():Number{{return offset}}}}\nlet a=make({increment})\nlet b=make(2)\nfn combine():Number{{mut result=0;repeat 100{{result=result+a()+b()}};return result}}\nlet first=combine()\nlet second=combine()\nfn<ref> unused():Unit{{let ignored=1}}\nfn apply(callback:fn(value:Number):Number,value:Number):Number{{mut total=0;repeat 100{{total=total+callback(value)}};return total}}\nfn plus(value:Number):Number{{return value+{increment}}}\nlet third=apply(plus,3)\nlet fourth=apply(plus,3)\nAnimation()"
        )
    };
    for increment in [1, 3, 1] {
        files.insert("main.sanim", source(increment));
        let p = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(
            p.bindings["second"].number().unwrap().value,
            Fraction::from((increment + 2) * 100)
        );
        assert_eq!(
            p.bindings["fourth"].number().unwrap().value,
            Fraction::from((3 + increment) * 100)
        );
    }
}

#[test]
#[ignore = "performance measurement"]
fn pure_call_benchmark() {
    use std::time::Instant;
    let mut reports = Vec::new();
    for permissions in ["<ref>", ""] {
        let source = format!(
            "let samples=range(5000)\nfn{permissions} expensive(n:Number):Number{{mut sum=len(samples);repeat 300{{sum=sum+n}};return sum}}\nmut answer=0\nrepeat 30{{answer=answer+expensive(2)}}\n"
        );
        let start = Instant::now();
        let prepared = run(&source).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(
            prepared.bindings["answer"].number().unwrap().value,
            Fraction::from(168000)
        );
        println!(
            "permissions={permissions:?} elapsed_us={} operations={} checkpoint_bytes={} value_hits={}",
            elapsed.as_micros(),
            prepared.statistics.operations,
            prepared.statistics.checkpoint_bytes,
            prepared.statistics.reused_expressions
        );
        reports.push((elapsed, prepared.statistics.operations));
    }
    assert!(
        reports[1].1 * 5 < reports[0].1,
        "memoized calls must avoid body work"
    );
}

#[test]
fn mutable_export_interface_changes_invalidate_cached_pure_consumers() {
    let mut engine = Engine::default();
    let mut files = MemorySources::default();
    files.insert("lib.sanim", "let value=42");
    files.insert(
        "main.sanim",
        "import \"lib\" as lib\nfn read_value():Number{return lib.value}\nlet answer=read_value()",
    );
    assert!(engine.lint("main.sanim", &files).is_ok());
    files.insert("lib.sanim", "mut value=42");
    assert_eq!(engine.lint("main.sanim", &files).unwrap_err().code, "S133");
}

#[test]
fn outer_let_signal_handles_still_require_an_explicit_reference_capability() {
    let source = "mut count=1\nfn<sig> live(sig value:Number):sig Number{return value}\nlet observed=live(count)\nfn read_observed():Number{return observed}";
    assert_eq!(run(source).unwrap_err().code, "S133");
    let mut files = MemorySources::default();
    files.insert("lib.sanim", "mut count=1\nfn<sig> live(sig value:Number):sig Number{return value}\nlet observed=live(count)");
    files.insert(
        "main.sanim",
        "import \"lib\" as lib\nfn read_observed():Number{return lib.observed}",
    );
    assert_eq!(
        Engine::default()
            .lint("main.sanim", &files)
            .unwrap_err()
            .code,
        "S133"
    );
}
