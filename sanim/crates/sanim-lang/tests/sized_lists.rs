use sanim_core::{Fraction, ListLength, Type, Unit};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn files(source: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files
}
fn lint(source: &str) -> Result<sanim_lang::LintReport, sanim_core::Diagnostic> {
    Engine::default().lint("main.sanim", &files(source))
}

#[test]
fn list_literals_and_generic_identity_retain_exact_static_lengths() {
    let report=lint("fn identity<T>(value:T):T{return value}\nlet values=[1,2,3]\nlet copy=identity(values)\nlet empty:List<Number,0>=[]\nlet explicit:List<Number,3>=values\nlet open:List<Number,_>=values").unwrap();
    let exports = &report.modules["main.sanim"].checked.exports;
    let exact = Type::List(Box::new(Type::Numeric(Unit::Number)), ListLength::Exact(3));
    assert_eq!(exports["values"], exact);
    assert_eq!(exports["copy"], exact);
    assert_eq!(exports["explicit"], exact);
    assert!(matches!(
        exports["open"],
        Type::List(_, ListLength::Unknown)
    ));
    assert!(matches!(
        exports["empty"],
        Type::List(_, ListLength::Exact(0))
    ));
}

#[test]
fn nested_literals_join_only_lengths_that_vary() {
    let report =
        lint("let rows=[[1,2],[3]]\nlet with_empty=[[],[1]]\nlet uniform=[[1,2],[3,4]]\nlet combined=[[1,2]]+[[3]]\nlet uniform_combined=[[1,2]]+[[3,4]]\nlet choices:List<\"a\"|\"b\",1>=[\"a\"]\nlet joined_choices=choices+[\"b\"]").unwrap();
    let exports = &report.modules["main.sanim"].checked.exports;
    for name in ["rows", "with_empty", "combined"] {
        assert!(
            matches!(&exports[name],Type::List(inner,ListLength::Exact(2)) if matches!(inner.as_ref(),Type::List(_,ListLength::Unknown)))
        );
    }
    assert!(
        matches!(&exports["uniform"],Type::List(inner,ListLength::Exact(2)) if matches!(inner.as_ref(),Type::List(_,ListLength::Exact(2))))
    );
    assert!(
        matches!(&exports["uniform_combined"],Type::List(inner,ListLength::Exact(2)) if matches!(inner.as_ref(),Type::List(_,ListLength::Exact(2))))
    );
    assert!(lint("let rows:List<List<Number,2>>=[[1,2],[3]]").is_err());
}

#[test]
fn exact_return_and_argument_contracts_fail_at_the_correct_boundary() {
    for source in [
        "let values:List<Number,2>=[1]",
        "fn pair():List<Number,2>{return [1]}",
        "fn pair(values:List<Number,2>):Number{return len(values)}\nlet count=pair([1])",
        "let invalid:List<Number,N>=[1]",
        "let invalid:List<Number,1.5>=[1]",
        "let invalid:List<Number,1s>=[1]",
    ] {
        assert!(lint(source).is_err(), "{source}");
    }
    lint("type Pair<T>=List<T,2>\nlet values:Pair<String>=[\"a\",\"b\"]").unwrap();
}

#[test]
fn unknown_length_reads_validate_exact_annotations_at_runtime() {
    let mut files = files("let values=read<List<Number,2>>(\"values.json\")");
    files.insert("values.json", "[1,2]");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(matches!(
        prepared.bindings["values"].ty(),
        Type::List(_, ListLength::Exact(2))
    ));
    files.insert("values.json", "[1]");
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("2") && error.message.contains("1 items"),
        "{error}"
    );
}

#[test]
fn csv_and_sqlite_validate_fixed_row_counts_without_guessing_sql() {
    let mut sources = files("type Row={value:Number}\nlet rows=read<List<Row,2>>(\"values.csv\")");
    sources.insert("values.csv", "value\n1\n2\n");
    Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    sources.insert("values.csv", "value\n1\n");
    assert!(
        Engine::default()
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .is_err()
    );
    let sources = files("let rows:List<Number,1>=query<Number>(\"SELECT 2\")");
    let prepared = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert!(matches!(
        prepared.bindings["rows"].ty(),
        Type::List(_, ListLength::Exact(1))
    ));
    let sources = files("let rows:List<Number,2>=query<Number>(\"SELECT 2\")");
    let error = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("SQL result") && error.message.contains("1 items"),
        "{error}"
    );
}

#[test]
fn mutable_and_reflected_inference_keep_editable_cardinality() {
    let report=lint("mut values=[1,2]\nvalues=[3]\nlet settings=reflect([1,2])\nlet fixed=reflect([1,2],key=\"fixed\")").unwrap();
    let checked = &report.modules["main.sanim"].checked;
    assert!(matches!(
        checked.exports["values"],
        Type::List(_, ListLength::Unknown)
    ));
    assert!(
        checked
            .reflected
            .values()
            .all(|input| matches!(input.ty.as_ref(), Type::List(_, ListLength::Unknown)))
    );
    assert!(lint("mut values:List<Number,2>=[1,2]\nvalues=[3]").is_err());
}

#[test]
fn list_lengths_are_disjoint_guards_and_sig_body_facts_are_live_safe() {
    let source = "fn count(values:List<Number,0>):Number{return 0}\nfn count(values:List<Number,1>):Number{return 1}\nlet a=count([])\nlet b=count([8])";
    let prepared = Engine::default()
        .prepare("main.sanim", &files(source), &BTreeMap::new())
        .unwrap();
    assert_eq!(
        prepared.bindings["b"].number().unwrap().value,
        Fraction::from(1)
    );
    assert!(
        lint("fn retain(sig values:List<Number,1>):sig List<Number,1>{return values}").is_err()
    );
    assert!(lint("fn retain(sig value:Number >= 0):sig Number >= 0{return value}").is_err());
    lint("fn retain(sig values:List<Number,1>):sig List<Number>{return values}").unwrap();
}

#[test]
fn numeric_set_distribution_discards_only_empty_arms() {
    lint("type Ends=Number < 0 | Number > 1\ntype Negative=Ends & Number < 0\nlet negative_sample:Negative=-1\ntype Positive=Ends - (Number < 0)\nlet positive_sample:Positive=2").unwrap();
    assert!(
        lint("type Ends=Number < 0 | Number > 1\ntype Empty=Ends & (Number >= 0 <= 1)").is_err()
    );
}
