use sanim_core::{Fraction, Type, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn run(source: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files.insert("samples.csv", "run,n,x,y\n");
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}

#[test]
fn record_intersections_resolve_to_one_flat_structural_schema() {
    let p=run("type RunInfo={run:String,n:Number}\ntype Measurements={x:Number,y:Number}\ntype Sample=RunInfo&Measurements\ntype Pair=RunInfo&{left:Sample,right:Sample}\nlet sample:Sample={run=\"r\",n=1,x=2,y=3}\nlet pair:Pair={run=\"p\",n=2,left=sample,right=sample}\n").unwrap();
    let Type::Record(sample) = &p.types["Sample"] else {
        panic!("intersection must resolve to a record")
    };
    assert_eq!(sample.len(), 4);
    let Type::Record(pair) = &p.types["Pair"] else {
        panic!("composition must not create a runtime hierarchy")
    };
    assert_eq!(pair.len(), 4);
    assert_eq!(pair["left"], p.types["Sample"]);
    assert_eq!(p.bindings["pair"].ty(), p.types["Pair"]);
    let Value::Record(value) = &p.bindings["sample"] else {
        panic!("flat record expected")
    };
    assert_eq!(value["x"].number().unwrap().value, Fraction::from(2));
}

#[test]
fn identical_overlapping_fields_merge_once_and_conflicting_types_fail() {
    let p=run("type Left={x:Number,shared:String}\ntype Right={x:Number,y:Number}\ntype Both=Left&Right\nlet value:Both={x=1,shared=\"ok\",y=2}\n").unwrap();
    assert!(matches!(&p.types["Both"],Type::Record(fields) if fields.len()==3));
    let error =
        run("type Left={x:Number}\ntype Right={x:String}\ntype Bad=Left&Right\n").unwrap_err();
    assert!(error.message.contains("x"), "{error}");
    assert!(
        error.message.contains("Number") && error.message.contains("String"),
        "{error}"
    );
    assert_eq!(error.source, "main.sanim");
}

#[test]
fn generic_record_composition_and_parentheses_keep_exact_field_sets() {
    let p=run("type Tagged<T> = T&{tag:String}\ntype Reading={value:Number}\ntype Sample=Tagged<Reading>\nlet sample:Sample={value=3,tag=\"ok\"}\ntype Grouped=({a:Number}&{b:Number})&({b:Number}&{c:Number})\nlet grouped:Grouped={a=1,b=2,c=3}\n").unwrap();
    assert!(matches!(&p.types["Sample"],Type::Record(fields) if fields.len()==2));
    assert!(matches!(&p.types["Grouped"],Type::Record(fields) if fields.len()==3));
    assert_eq!(p.bindings["sample"].ty(), p.types["Sample"]);
}

#[test]
fn subtraction_removes_named_fields_and_keeps_a_flat_schema() {
    let p=run("type Measurements={x:Number,y:Number,error:Number}\ntype Sample=Measurements-{error:Number}\nlet sample:Sample={x=1,y=2}\ntype Empty=Sample-Sample\nlet empty:Empty=Empty()\n").unwrap();
    assert!(
        matches!(&p.types["Sample"],Type::Record(fields) if fields.len()==2&&!fields.contains_key("error"))
    );
    assert!(matches!(&p.types["Empty"],Type::Record(fields) if fields.is_empty()));
    assert_eq!(p.bindings["sample"].ty(), p.types["Sample"]);
}

#[test]
fn parenthesized_type_difference_composes_with_intersections() {
    let p=run("type Base={x:Number,y:Number}\ntype WithoutY=(Base-{y:Number})&{label:String}\ntype Grouped=Base-({x:Number}&{y:Number})\nlet row:WithoutY={x=1,label=\"ok\"}\n").unwrap();
    assert!(
        matches!(&p.types["WithoutY"],Type::Record(fields) if fields.len()==2&&fields.contains_key("x")&&fields.contains_key("label"))
    );
    assert!(matches!(&p.types["Grouped"],Type::Record(fields) if fields.is_empty()));
    let error = run("type Base={x:Number}\ntype Bad=Base-{missing:Number}\n").unwrap_err();
    assert!(error.message.contains("missing"), "{error}");
    assert!(error.message.contains("field"), "{error}");
}

#[test]
fn empty_csv_and_sql_lists_preserve_the_full_composed_row_schema() {
    let p=run("type RunInfo={run:String,n:Number}\ntype Measurements={x:Number,y:Number}\ntype Sample=RunInfo&Measurements\nlet rows=read<List<Sample>>(\"samples.csv\")\nlet selected=query<Sample>(\"SELECT run,n,x,y FROM rows WHERE 0\",tables={rows=rows})\nassert(len(rows)==0)\nassert(len(selected)==0)\n").unwrap();
    let expected = Type::List(
        Box::new(p.types["Sample"].clone()),
        sanim_core::ListLength::Exact(0),
    );
    assert_eq!(p.bindings["rows"].ty(), expected);
    assert_eq!(p.bindings["selected"].ty(), expected);
    assert!(p.bindings["selected"].list().unwrap().is_empty());
    let error=run("type Sample={run:String,n:Number,x:Number,y:Number}\nlet rows=read<List<Sample>>(\"samples.csv\")\nlet selected=query<Sample>(\"SELECT run,n,x FROM rows WHERE 0\",tables={rows=rows})\n").unwrap_err();
    assert!(error.message.contains("y"), "{error}");
    assert!(error.message.contains("column"), "{error}");
}

#[test]
fn type_operators_use_left_associative_difference_and_lower_precedence_unions() {
    let p=run("type Base={x:Number,y:Number}\ntype Composed=Base-{y:Number}&{label:String}\nlet row:Composed={x=1,label=\"ok\"}\ntype Choice=Number|{x:Number}&{y:Number}\nlet number_value:Choice=7\nlet record_value:Choice={x=1,y=2}\n").unwrap();
    assert!(
        matches!(&p.types["Composed"],Type::Record(fields) if fields.len()==2&&fields.contains_key("x")&&fields.contains_key("label"))
    );
    assert_eq!(
        p.bindings["number_value"].number().unwrap().value,
        Fraction::from(7)
    );
    assert!(matches!(&p.bindings["record_value"],Value::Record(fields) if fields.len()==2));
}
