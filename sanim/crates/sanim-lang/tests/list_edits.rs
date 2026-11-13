use sanim_core::{Fraction, Type, Unit, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn prepare(source: &str) -> sanim_lang::Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap()
}

#[test]
fn list_edits_keep_input_snapshots_and_element_order() {
    let prepared = prepare(
        "let original=[1,2,3]\nlet appended=append(original,4)\nlet inserted=insert(appended,1,9)\nlet removed=remove_at(inserted,2)\nlet replaced=replace_at(removed,0,7)\nlet moved=move_item(replaced,3,0)\nassert(original==[1,2,3])\nassert(moved==[4,7,9,3])\n",
    );
    assert_eq!(prepared.bindings["original"].list().unwrap().len(), 3);
    assert_eq!(prepared.bindings["moved"].list().unwrap().len(), 4);
}

#[test]
fn empty_results_keep_schema_and_empty_input_infers_from_inserted_value() {
    let prepared = prepare(
        "let first=append([],3)\nlet empty=remove_at(first,0)\nlet restored=insert(empty,0,4)\nassert(len(empty)==0)\nassert(restored==[4])\n",
    );
    let expected = Type::List(Box::new(Type::Numeric(Unit::Number)));
    assert_eq!(prepared.bindings["first"].ty(), expected);
    assert_eq!(prepared.bindings["empty"].ty(), expected);
    assert_eq!(prepared.bindings["restored"].ty(), expected);
}

#[test]
fn invalid_list_edits_have_specific_type_and_bounds_messages() {
    for (source, expected) in [
        (
            "let empty:List<Number>=[]\nlet bad=remove_at(empty,0)",
            "the list is empty",
        ),
        (
            "let bad=insert([1],2,3)",
            "insertion index must be between 0 and 1 inclusive",
        ),
        (
            "let bad=replace_at([1],-1,3)",
            "index must be a nonnegative integer",
        ),
        (
            "let bad=remove_at([1],0.5)",
            "index must be a nonnegative integer",
        ),
        (
            "let bad=move_item([1,2],0,2)",
            "to_index 2 is out of bounds",
        ),
        ("let bad=append([1],\"wrong\")", "list element type Number"),
        (
            "let bad=replace_at([1],0,Color())",
            "list element type Number",
        ),
        ("let bad=append(3,4)", "items must be List<T>"),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn list_edits_consume_the_shared_optional_timeline_duration() {
    let prepared = prepare(
        "mut items:List<Number>=[]\nitems=append(items,1,duration=400ms)\nitems=insert(items,0,2,duration=100ms)\nitems=remove_at(items,1,duration=500ms)\nassert(items==[2])\n",
    );
    assert_eq!(prepared.animation.duration, Fraction::from(1));
}

#[test]
fn no_op_edits_share_a_large_list_payload() {
    let prepared = prepare(
        "let original=range(1000)\nlet replaced=replace_at(original,500,500)\nlet moved=move_item(original,500,500)\n",
    );
    let Value::List(original) = &prepared.bindings["original"] else {
        panic!("list")
    };
    for name in ["replaced", "moved"] {
        let Value::List(edited) = &prepared.bindings[name] else {
            panic!("list")
        };
        assert!(original.values.ptr_eq(&edited.values));
    }
}

#[test]
fn repeated_edits_charge_shared_storage_instead_of_the_full_list_each_time() {
    let prepared = prepare(
        "mut items=range(20000)\nrepeat 100 {items=append(items,-1)}\nassert(len(items)==20100)\n",
    );
    assert_eq!(prepared.bindings["items"].list().unwrap().len(), 20_100);
    assert_eq!(prepared.statistics.mutable_writes, 100);
}

#[test]
fn nested_argument_writes_do_not_confuse_the_native_edit_receipt() {
    let prepared = prepare(
        "mut items=[1,2]\nfn update():Number{items=append(items,3);return 9}\nitems=append(items,update())\nassert(items==[1,2,9])\nlet saved=items\nfn helper():List<Number>{return append(items,10)}\nitems=helper()\nassert(saved==[1,2,9])\nassert(items==[1,2,9,10])\n",
    );
    assert_eq!(prepared.bindings["items"].list().unwrap().len(), 4);
}
