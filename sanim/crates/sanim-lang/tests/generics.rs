use sanim_core::{Fraction, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;
fn run(source: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files.insert("value.json", "12");
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}
#[test]
fn defaults_are_fallbacks_after_argument_and_expected_type_inference() {
    let p=run("fn absent<T=Number>():Option<T>{return None}\nlet a=absent()\nlet b:Option<String> =absent()\nfn identity<T=Number>(value:T):T{return value}\nlet c=identity(\"text\")\n").unwrap();
    assert_eq!(p.bindings["a"], Value::None);
    assert_eq!(p.bindings["b"], Value::None);
    assert_eq!(p.bindings["c"], Value::String("text".into()));
}
#[test]
fn constrained_numeric_helpers_preserve_concrete_types() {
    let p=run("fn square<T:Numeric>(value:T):T{return value*value}\nlet a=square(3)\nlet b=square(Fraction(2,3))\n").unwrap();
    assert_eq!(p.bindings["a"].number().unwrap().value, Fraction::from(9));
    assert_eq!(p.bindings["b"], Value::Fraction(Fraction::new(4u64, 9u64)));
    assert!(
        run("fn square<T:Numeric>(value:T):T{return value*value}\nlet a=square(\"wrong\")")
            .is_err()
    );
}
#[test]
fn structural_constraints_expose_only_guaranteed_fields() {
    let p=run("fn amount<T:{value:Number}>(row:T):Number{return row.value}\nlet result=amount({value:7,label:\"extra\"})").unwrap();
    assert_eq!(
        p.bindings["result"].number().unwrap().value,
        Fraction::from(7)
    );
    assert!(run("fn amount<T:{value:Number}>(row:T):Number{return row.other}").is_err());
    assert!(run("fn amount<T:{value:Number}>(row:T):Number{return row.value}\nlet result=amount({other:1})").is_err());
}
#[test]
fn generic_reads_use_call_specific_schemas_and_track_dependencies() {
    let p=run("fn load<T:Readable=Data>(path:String):T{return read<T>(path)}\nlet raw=load(\"value.json\")\nlet number_value=load<Number>(\"value.json\")\n").unwrap();
    assert_eq!(p.bindings["raw"], Value::Data(serde_json::json!(12)));
    assert_eq!(
        p.bindings["number_value"].number().unwrap().value,
        Fraction::from(12)
    );
    assert!(p.dependencies.contains_key("value.json"));
}
#[test]
fn generic_defaults_constraints_and_parameter_order_fail_cleanly() {
    for source in [
        "fn bad<T:Numeric=String>(value:T):T{return value}",
        "fn bad<T=Number,U>(value:U):U{return value}",
        "fn bad<T:Numeric>():T{return 1}",
        "fn bad<T:Numeric,U:Numeric>(a:T,b:U):T{return a+b}",
        "fn bad<T:Missing>(value:T):T{return value}",
    ] {
        assert!(run(source).is_err(), "{source}");
    }
}
#[test]
fn type_aliases_support_defaults_and_constraints() {
    let p=run("type Box<T:Readable=Number> ={value:T}\nlet a:Box={value:3}\nlet b:Box<String> ={value:\"ok\"}").unwrap();
    assert!(matches!(p.bindings["a"], Value::Record(_)));
    assert!(matches!(p.bindings["b"], Value::Record(_)));
}

#[test]
fn dependent_constraints_are_checked_after_substitution() {
    let p=run("fn pick<T,U:T>(a:T,b:U):T{return b}\nlet result=pick(1,2)\ntype Pair<T,U:T=T>={a:T,b:U}\nlet pair:Pair<Number>={a:1,b:2}").unwrap();
    assert_eq!(
        p.bindings["result"].number().unwrap().value,
        Fraction::from(2)
    );
    assert!(run("fn pick<T,U:T>(a:T,b:U):T{return b}\nlet result=pick(1,\"wrong\")").is_err());
}
#[test]
fn readable_constraint_does_not_imply_data_conversion() {
    assert!(run("fn pack<T:Readable>(value:T):Data{return Data(value)}").is_err());
    run("fn pack<T:Reflectable>(value:T):Data{return Data(value)}\nlet result=pack(2)").unwrap();
}
