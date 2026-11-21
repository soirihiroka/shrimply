use sanim_core::{Fraction, ListLength, Type, Unit, Value};
use sanim_lang::{Engine, MemorySources};
fn run(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{source}\nAnimation()\n"));
    Engine::default().prepare("main.sanim", &files, &Default::default())
}
#[test]
fn numeric_list_math_and_fixed_indices_have_one_canonical_value_type() {
    let prepared=run("let a=[1,2,3]\nlet b=[4,5,6]\nlet sum=a+b\nlet component=sum[1]\nlet product=dot(a,b)\nlet magnitude=norm([3,4])\nlet normal=cross([1,0,0],[0,1,0])\nlet joined=concat(a,b)\nassert(component==7)\nassert(product==32)\nassert(magnitude==5)\nassert(normal==[0,0,1])\nassert(joined==[1,2,3,4,5,6])").unwrap();
    assert_eq!(
        prepared.bindings["sum"].ty(),
        Type::List(Box::new(Type::Numeric(Unit::Number)), ListLength::Exact(3))
    );
    assert_eq!(
        prepared.bindings["component"],
        Value::Number(sanim_core::Number::scalar(7))
    );
}
#[test]
fn unknown_lengths_check_dimensions_at_runtime_and_unknown_indices_remain_optional() {
    let error=run("fn add_values(a:List<Number>,b:List<Number>):List<Number>{return a+b}\nlet result=add_values([1,2],[3])").unwrap_err();
    assert!(error.message.contains("matching lengths"), "{error}");
    let prepared = run(
        "mut index=1\nlet values=[10,20]\nlet maybe=values[index]\nassert(unwrap(maybe,0)==20)",
    )
    .unwrap();
    assert!(matches!(prepared.bindings["maybe"], Value::Some(_)));
    assert!(run("let invalid=[1,2][2]").is_err());
}
#[test]
fn literal_list_signals_retain_dimension_proofs_and_component_dependencies() {
    let prepared=run("mut coordinate=3\nsig axes=[coordinate,4]\nsig magnitude=norm(axes)\nsig position=[coordinate*1pt,2pt]\nsig x=position[0]\nlet shape=Box(position=position,width=magnitude*1pt,height=10pt)\ncoordinate=6\nlet result=x\nassert(result==6pt)").unwrap();
    assert_eq!(
        prepared.bindings["result"].number().unwrap().value,
        Fraction::from(6)
    );
}
#[test]
fn old_vector_names_are_absent_in_types_and_values() {
    for name in ["Vec2", "Vec3", "Vec4"] {
        assert!(run(&format!("let value={name}(1,2)")).is_err());
        assert!(run(&format!("let value:{name}=[1,2]")).is_err());
        assert!(!sanim_lang::prelude::protected(name));
    }
}
