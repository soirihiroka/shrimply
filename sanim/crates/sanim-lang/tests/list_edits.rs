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
        "let original=[1,2,3]\nlet appended=append(original,4)\nlet inserted=insert(appended,1,9)\nlet removed=pop(inserted,2)\nlet replaced=replace_at(removed,0,7)\nlet moved=move_item(replaced,3,0)\nassert(original==[1,2,3])\nassert(moved==[4,7,9,3])\n",
    );
    assert_eq!(prepared.bindings["original"].list().unwrap().len(), 3);
    assert_eq!(prepared.bindings["moved"].list().unwrap().len(), 4);
}

#[test]
fn empty_results_keep_schema_and_empty_input_infers_from_inserted_value() {
    let prepared = prepare(
        "let first=append([],3)\nlet empty=pop(first,0)\nlet restored=insert(empty,0,4)\nassert(len(empty)==0)\nassert(restored==[4])\n",
    );
    let expected = Type::List(Box::new(Type::Numeric(Unit::Number)));
    assert_eq!(prepared.bindings["first"].ty(), expected);
    assert_eq!(prepared.bindings["empty"].ty(), expected);
    assert_eq!(prepared.bindings["restored"].ty(), expected);
}

#[test]
fn pop_and_popleft_remove_opposite_ends_without_changing_the_input() {
    let prepared = prepare(
        "let original=[1,2,3]\nlet stack=pop(original)\nlet queue=popleft(original)\nlet middle=pop(original,index=1)\nassert(original==[1,2,3])\nassert(stack==[1,2])\nassert(queue==[2,3])\nassert(middle==[1,3])\nlet empty=popleft([1])\nassert(len(empty)==0)\n",
    );
    assert_eq!(prepared.bindings["original"].list().unwrap().len(), 3);
    assert_eq!(
        prepared.bindings["empty"].ty(),
        Type::List(Box::new(Type::Numeric(Unit::Number)))
    );
}

#[test]
fn invalid_list_edits_have_specific_type_and_bounds_messages() {
    for (source, expected) in [
        (
            "let empty:List<Number>=[]\nlet bad=pop(empty,0)",
            "the list is empty",
        ),
        (
            "let empty:List<Number>=[]\nlet bad=pop(empty)",
            "pop: cannot remove an item; the list is empty (length 0)",
        ),
        (
            "let empty:List<Number>=[]\nlet bad=popleft(empty)",
            "popleft: cannot remove an item; the list is empty (length 0)",
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
            "let bad=pop([1],0.5)",
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
        ("let bad=pop(3)", "items must be List<T>"),
        ("let bad=popleft(3)", "items must be List<T>"),
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
        "mut items:List<Number>=[]\nitems=append(items,1,duration=400ms)\nitems=insert(items,0,2,duration=100ms)\nitems=pop(items,1,duration=500ms)\nassert(items==[2])\n",
    );
    assert_eq!(prepared.animation.duration, Fraction::from(1));
}

#[test]
fn stack_and_queue_pops_share_timing_and_commit_at_the_end() {
    let prepared = prepare(
        "mut stack=[1,2,3]\nmut queue=[1,2,3]\nbackground{stack=pop(stack,duration=500ms)}\nbackground{queue=popleft(queue,duration=500ms)}\nlet before_stack=stack\nlet before_queue=queue\nwait(500ms)\nassert(before_stack==[1,2,3] and before_queue==[1,2,3])\nassert(stack==[1,2] and queue==[2,3])",
    );
    assert_eq!(prepared.animation.duration, Fraction::new(1u64, 2u64));
}

#[test]
fn repeated_pops_preserve_large_snapshots_without_full_payload_charges() {
    let prepared = prepare(
        "let original=range(20000)\nmut stack=original\nmut queue=original\nrepeat 100{stack=pop(stack);queue=popleft(queue)}\nassert(len(original)==20000)\nassert(len(stack)==19900 and len(queue)==19900)\nassert(get(stack,0)==Some(0) and get(queue,0)==Some(100))",
    );
    assert_eq!(prepared.statistics.mutable_writes, 200);
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

#[test]
#[ignore = "manual performance measurement"]
fn pop_benchmark() {
    for operation in ["pop", "popleft"] {
        for size in [1_000, 20_000] {
            let mut files = MemorySources::default();
            let prefix =
                format!("mut items=range({size})\nrepeat 200{{items={operation}(items)}}\n");
            files.insert("main.sanim", format!("{prefix}wait(1s)"));
            let mut engine = Engine::default();
            let started = std::time::Instant::now();
            let prepared = engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap();
            let elapsed = started.elapsed();
            assert_eq!(prepared.bindings["items"].list().unwrap().len(), size - 200);
            assert_eq!(prepared.statistics.mutable_writes, 200);
            files.insert("main.sanim", format!("{prefix}wait(2s)"));
            let started = std::time::Instant::now();
            let patched = engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap();
            let patch = started.elapsed();
            assert!(patched.statistics.restored_statements >= 2);
            println!(
                "{operation} size={size} edits=200 prepare_us={} native_us={} validation_us={} cache_key_us={} patch_us={} restored={} patch_ops={}",
                elapsed.as_micros(),
                prepared.statistics.native_micros,
                prepared.statistics.value_validation_micros,
                prepared.statistics.cache_key_micros,
                patch.as_micros(),
                patched.statistics.restored_statements,
                patched.statistics.operations
            );
        }
    }
}
