use sanim_core::{FnCapability, Fraction, Type};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn prepare(source: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{source}\nAnimation()"));
    Engine::default().prepare("main.sanim", &sources, &BTreeMap::new())
}
#[test]
fn boolean_assertions_are_pure_immediate_and_leave_no_verification_state() {
    let p = prepare("fn checked(value:Number):Number{assert(value>0,message=\"value must be positive\");return value}\nlet answer=checked(42)\nassert(answer==42)").unwrap();
    let signature = &p.symbols["checked"].signatures()[0];
    assert!(signature.capabilities.is_pure());
    assert!(!signature.mutates);
    assert_eq!(p.animation.duration, Fraction::from(0));
    assert!(p.checks.is_empty());
    assert!(p.contracts.is_empty());
    let builtin = sanim_lang::prelude::builtin("assert").unwrap();
    assert!(builtin.capabilities().is_pure());
    assert!(builtin.capabilities_for_arguments([&Type::Bool]).is_pure());
    assert!(
        builtin
            .capabilities_for_arguments([&Type::Predicate])
            .contains(FnCapability::Mut)
    );
}
#[test]
fn failed_boolean_assertions_stop_the_current_call_and_loop_before_later_work() {
    for source in [
        "fn checked(value:Number):Number{assert(value>0,message=\"value must be positive\");return value}\nlet answer=checked(-1)\nlet absent=read<String>(\"absent.txt\")",
        "repeat 4 { assert(false,message=\"value must be positive\");let absent=read<String>(\"absent.txt\") }",
    ] {
        let error = prepare(source).unwrap_err();
        assert_eq!(error.code, "S306", "{error}");
        assert_eq!(error.message, "value must be positive");
        assert_eq!(error.source, "main.sanim");
        assert!(error.span.end > error.span.start);
    }
}
#[test]
fn boolean_bounded_generics_remain_pure() {
    prepare("fn require_true<T:Bool>(value:T):Unit{assert(value)}\nrequire_true(true)").unwrap();
    assert_eq!(
        prepare("fn require_true<T:Bool>(value:T):Unit{assert(value)}\nrequire_true(false)")
            .unwrap_err()
            .code,
        "S306"
    );
}
#[test]
fn visual_predicates_and_recorded_checks_still_require_mut() {
    for source in [
        "fn validate(a:Element,b:Element):Unit{assert(a.left_of(b))}",
        "fn validate():Unit{check(true)}",
    ] {
        assert_eq!(prepare(source).unwrap_err().code, "S133");
    }
    let p=prepare("fn<mut> validate(a:Element,b:Element):Unit{assert(a.left_of(b))}\nlet a=Box(width=10pt,height=10pt,position=[10pt, 10pt])\nlet b=Box(width=10pt,height=10pt,position=[50pt, 10pt])\nAnimation(children=[a,b])\nvalidate(a,b)").unwrap();
    assert_eq!(p.contracts.len(), 1);
    assert_eq!(p.checks.len(), 1);
}
#[test]
fn cached_success_never_hides_a_changed_immutable_capture_or_argument_failure() {
    let source = "let limit=reflect(3,key=\"limit\")\nfn checked(value:Number):Number{assert(value<limit,message=\"above limit\");mut total=0;repeat 100 {total=total+value};return total}\nlet a=checked(1)\nlet b=checked(1)\nAnimation()\n";
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    let mut engine = Engine::default();
    let successful = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert!(successful.statistics.reused_expressions > 0);
    assert!(successful.checks.is_empty() && successful.contracts.is_empty());
    let error = engine
        .prepare(
            "main.sanim",
            &sources,
            &BTreeMap::from([(
                "limit".into(),
                sanim_core::Value::Number(sanim_core::Number::scalar(0)),
            )]),
        )
        .unwrap_err();
    assert_eq!(error.code, "S306");
    assert_eq!(error.message, "above limit");
    sources.insert(
        "main.sanim",
        source.replace("let b=checked(1)", "let b=checked(4)"),
    );
    assert_eq!(
        engine
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S306"
    );
}
#[test]
fn validation_rechecks_new_signal_values_and_never_disappears_in_native_lowering() {
    let error=prepare("fn checked(value:Number):Number{assert(value>=0);return value}\nmut value=1\nsig result=checked(value)\nlet before=result\nvalue=-1\nlet after=result").unwrap_err();
    assert_eq!(error.code, "S306", "{error}");
    let error=prepare("fn checked(value:Length):Length{assert(value>=0pt);return value}\nmut size=1pt\nsig width=checked(size)\nlet shape=Box(width=width,height=1pt)\nAnimation(children=[shape,animate(size,to=-1pt,duration=1s)])").unwrap_err();
    assert_eq!(error.code, "S140", "{error}");
    assert!(
        error.message.contains("retained animated helpers"),
        "{error}"
    );
}
#[test]
fn plan_measurement_and_full_preparation_both_validate_boolean_assertions() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "fn checked():Number{assert(false);return 1}\nlet ignored=checked()\nwait(1s)",
    );
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &sources, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S306"
    );
    assert_eq!(
        Engine::default()
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S306"
    );
}

#[test]
fn visual_relationships_have_explicit_typed_sample_times() {
    let source = "let a=Box(width=10pt,height=10pt,position=[-20pt,0pt])\nlet b=Box(width=10pt,height=10pt,position=[20pt,0pt])\nassert(a.left_of(b),at=0s)\nreturn Animation(children=[a,b,a.move_to([40pt,0pt],duration=1s,ease=\"linear\")])";
    prepare(source).unwrap();
    let failed = prepare(&source.replace("at=0s", "at=1s")).unwrap_err();
    assert_eq!(failed.code, "S350", "{failed}");
    let failed = prepare("assert(true,at=1s)").unwrap_err();
    assert!(
        failed
            .message
            .contains("boolean assertions validate immediately"),
        "{failed}"
    );
}
