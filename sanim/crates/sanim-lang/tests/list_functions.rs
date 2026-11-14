use sanim_core::{Fraction, Type, Unit, Value};
use sanim_lang::{Engine, MemorySources, Preparation};

fn files(source: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files
}
fn prepare(source: &str) -> sanim_lang::Prepared {
    Engine::default()
        .prepare("main.sanim", &files(source), &Default::default())
        .unwrap()
}

#[test]
fn map_filter_and_flat_map_keep_generic_schemas_and_empty_results() {
    let prepared = prepare(
        r#"fn same<T>(value:T):T {return value}
fn pair(value:Number):List<Number> {return [value,value+1]}
fn positive(value:Number):Bool {return value>0}
fn row(value:Number):{n:Number,label:String} {return {n=value,label="v{value}"}}
let items=[-1,0,2]
let mapped=items.map(same)
let records=items.map(row)
let filtered=items.filter(positive)
let expanded=filtered.flat_map(pair)
let names=["a","b"].map(same)
let empty:List<Number>=[]
let empty_mapped=empty.map(row)
let empty_filtered=empty.filter(positive)
assert(mapped==items)
assert(filtered==[2])
assert(expanded==[2,3])
assert(names==["a","b"])
assert(len(empty_mapped)==0)
assert(len(empty_filtered)==0)
wait(1s)"#,
    );
    assert_eq!(
        prepared.bindings["empty_filtered"].ty(),
        Type::List(
            Box::new(Type::Numeric(Unit::Number)),
            sanim_core::ListLength::Exact(0)
        )
    );
    let (
        Type::List(empty, sanim_core::ListLength::Exact(0)),
        Type::List(records, sanim_core::ListLength::Exact(3)),
    ) = (
        prepared.bindings["empty_mapped"].ty(),
        prepared.bindings["records"].ty(),
    )
    else {
        panic!("typed empty result and records")
    };
    assert_eq!(empty, records);
    let (Value::List(mapped), Value::List(items)) =
        (&prepared.bindings["mapped"], &prepared.bindings["items"])
    else {
        panic!("typed lists")
    };
    assert!(mapped.shares_values(items));
}

#[test]
fn reduce_fold_scan_and_empty_contracts_are_explicit() {
    let prepared = prepare(
        r#"fn sum_values(total:Number,value:Number):Number{return total+value}
let items=[1,2,3]
let reduced=items.reduce(sum_values)
let folded=items.fold(10,sum_values)
let scanned=items.scan(0,sum_values)
let empty:List<Number>=[]
let zero=empty.fold(7,sum_values)
let no_steps=empty.scan(0,sum_values)
let singleton=[5].reduce(sum_values)
assert(reduced==6)
assert(folded==16)
assert(scanned==[1,3,6])
assert(zero==7)
assert(singleton==5)
assert(len(no_steps)==0)
wait(1s)"#,
    );
    assert_eq!(
        prepared.bindings["no_steps"].ty(),
        Type::List(
            Box::new(Type::Numeric(Unit::Number)),
            sanim_core::ListLength::Exact(0)
        )
    );
    let error=Engine::default().prepare("main.sanim",&files("fn sum_values(a:Number,b:Number):Number{return a+b}\nlet empty:List<Number>=[]\nlet bad=empty.reduce(sum_values)"),&Default::default()).unwrap_err();
    assert!(
        error.message.contains("empty list") && error.message.contains("fold"),
        "{error}"
    );
}

#[test]
fn accumulator_clauses_close_over_reachable_state_union() {
    for preparation in [Preparation::Full, Preparation::Demand] {
        let mut engine = Engine::default();
        engine.preparation = preparation;
        let prepared = engine
            .prepare(
                "main.sanim",
                &files(
                    r#"
fn step(state:"empty",item:Number):"full" {return "full"}
fn step(state:"full",item:Number):"empty" {return "empty"}
fn step(state:"unused",item:Number):"invalid" {return "invalid"}
fn reduce_step(state:"empty",item:"empty"|"full"):"full" {return "full"}
fn reduce_step(state:"full",item:"empty"|"full"):"empty" {return "empty"}
let folded=[1,2].fold("empty",step)
let scanned=[1,2,3].scan("empty",step)
let states:List<"empty"|"full">=["empty","full","empty"]
let reduced=states.reduce(reduce_step)
assert(folded=="empty")
assert(scanned==["full","empty","full"])
assert(reduced=="empty")
wait(1s)
"#,
                ),
                &Default::default(),
            )
            .unwrap();
        assert_eq!(prepared.animation.duration, Fraction::from(1));
    }
}

#[test]
fn callbacks_require_closed_accumulators_and_complete_argument_coverage() {
    for (source, reason) in [
        (
            r#"fn step(state:"empty",item:Number):"full" {return "full"}
let result=[1,2].fold("empty",step)"#,
            "not closed",
        ),
        (
            r#"fn step(state:"empty",item:Number):"full" {return "full"}
fn step(state:"full",item:Number):"missing" {return "missing"}
let result=[1,2].scan("empty",step)"#,
            "not closed",
        ),
        (
            r#"fn kind(value:"a"):Number {return 1}
fn kind(value:"b"):Number {return 2}
let items:List<"a"|"b"|"c">=["c"]
let result=items.map(kind)"#,
            "do not cover every combination",
        ),
        (
            r#"fn step(state:"empty",item:"a"):"full" {return "full"}
fn step(state:"full",item:"b"):"empty" {return "empty"}
let items:List<"a"|"b">=["a","b"]
let result=items.fold("empty",step)"#,
            "do not cover every combination",
        ),
    ] {
        let error = Engine::default()
            .prepare("main.sanim", &files(source), &Default::default())
            .unwrap_err();
        assert!(error.message.contains(reason), "{source}: {error}");
    }
    prepare(
        r#"fn kind(value:"a"):Number {return 1}
fn kind(value:"b"):Number {return 2}
let items:List<"a"|"b">=["a","b"]
assert(items.map(kind)==[1,2])
fn lower(value:Number<0):Number{return value}
fn lower(value:Number>=0):Number{return value}
assert([-2,0,3].map(lower)==[-2,0,3])
wait(1s)"#,
    );
}

#[test]
fn callbacks_are_invoked_once_and_early_exit_preserves_mutations() {
    for preparation in [Preparation::Full, Preparation::Demand] {
        let mut engine = Engine::default();
        engine.preparation = preparation;
        let prepared = engine
            .prepare(
                "main.sanim",
                &files(
                    r#"mut calls=0
fn counted(value:Number):Number {calls=calls+1;return value*2}
fn matches(value:Number):Bool {calls=calls+1;return value==2}
let mapped=[1,2,3].map(counted)
assert(calls==3)
let found=[1,2,3].find(matches)
assert(calls==5)
let position=[1,2,3].position(matches)
assert(calls==7)
let any_match=[1,2,3].any(matches)
assert(calls==9)
let all_match=[1,2,3].all(matches)
assert(calls==10)
assert(mapped==[2,4,6])
assert(found==Some(2))
assert(position==Some(1))
assert(any_match)
assert(not all_match)
wait(1s)"#,
                ),
                &Default::default(),
            )
            .unwrap();
        assert_eq!(prepared.animation.duration, 1.into());
    }
}

#[test]
fn timed_callbacks_use_the_canonical_timeline_and_bounded_outer_envelopes() {
    let source = r#"fn double(value:Number):Number {wait(100ms);return value*2}
let mapped=[1,2,3].map(double)
assert(mapped==[2,4,6])
wait(1s)"#;
    let prepared = prepare(source);
    assert_eq!(prepared.animation.duration, Fraction::new(13u64, 10u64));
    let duration = Engine::default()
        .duration("main.sanim", &files(source), &Default::default())
        .unwrap();
    assert_eq!(duration.duration, prepared.animation.duration);
    let padded = prepare(
        r#"fn step(value:Number):Number {wait(100ms);return value}
let result=[1,2].map(step,duration=500ms)
wait(1s)"#,
    );
    assert_eq!(padded.animation.duration, Fraction::new(3u64, 2u64));
    let error = Engine::default()
        .prepare(
            "main.sanim",
            &files(
                r#"fn step(value:Number):Number {wait(100ms);return value}
let result=[1,2].map(step,duration=100ms)"#,
            ),
            &Default::default(),
        )
        .unwrap_err();
    assert!(error.message.contains("exceeding requested"), "{error}");
}

#[test]
fn timed_reductions_scans_and_early_exit_predicates_match_duration_resolution() {
    let source = r#"fn accumulate(total:Number,value:Number):Number{wait(10ms);return total+value}
fn is_two(value:Number):Bool{wait(10ms);return value==2}
let reduced=[1,2,3].reduce(accumulate)
let folded=[1,2,3].fold(0,accumulate)
let scanned=[1,2,3].scan(0,accumulate)
let any_hit=[1,2,3].any(is_two)
let all_hit=[1,2,3].all(is_two)
let position=[1,2,3].position(is_two)
let found=[1,2,3].find(is_two)
assert(reduced==6 and folded==6 and scanned==[1,3,6])
assert(any_hit and not all_hit and position==Some(1) and found==Some(2))
wait(1s)"#;
    let prepared = prepare(source);
    assert_eq!(prepared.animation.duration, Fraction::new(23u64, 20u64));
    let report = Engine::default()
        .duration("main.sanim", &files(source), &Default::default())
        .unwrap();
    assert_eq!(report.duration, prepared.animation.duration);
}

#[test]
fn structural_methods_share_slices_and_keep_source_snapshots() {
    let prepared = prepare(
        r#"let items=[1,2,3,4]
let first=items.take(2)
let last=items.drop(2)
let middle=items.slice(start=1,end=3)
let empty=items.slice(start=3,end=1)
let unchanged=items.slice()
let reversed=items.reverse()
let indexed=items.enumerate()
let zipped=items.zip(["a","b"])
assert(first==[1,2])
assert(last==[3,4])
assert(middle==[2,3])
assert(len(empty)==0)
assert(reversed==[4,3,2,1])
assert(items==[1,2,3,4])
assert(indexed==[{index=0,value=1},{index=1,value=2},{index=2,value=3},{index=3,value=4}])
assert(zipped==[{left=1,right="a"},{left=2,right="b"}])
wait(1s)"#,
    );
    let (Value::List(unchanged), Value::List(items)) =
        (&prepared.bindings["unchanged"], &prepared.bindings["items"])
    else {
        panic!("typed lists")
    };
    assert!(unchanged.shares_values(items));
}

#[test]
fn sorting_and_unique_use_stable_keys_evaluated_once_in_input_order() {
    prepare(
        r#"mut calls=0
type Row={key:Number,label:String}
fn key(row:Row):Number {calls=calls+1;return row.key}
let items=[{key=2,label="first"},{key=1,label="second"},{key=2,label="third"}]
let sorted=items.sort(key=key)
assert(calls==3)
assert(sorted==[{key=1,label="second"},{key=2,label="first"},{key=2,label="third"}])
let unique=items.unique(key=key)
assert(calls==6)
assert(unique==[{key=2,label="first"},{key=1,label="second"}])
let descending=[3,1,2].sort(reverse=true)
let words=["b","a","b"].unique()
assert(descending==[3,2,1])
assert(words==["b","a"])
wait(1s)"#,
    );
}

#[test]
fn bisect_uses_ascending_sorted_contract_and_handles_duplicates_empty_and_windows() {
    prepare(
        r#"let items=[1,2,2,2,4]
assert(items.bisect_left(2)==1)
assert(items.bisect_right(2)==4)
assert(items.bisect_left(3)==4)
assert(items.bisect_left(0)==0)
assert(items.bisect_right(8)==5)
assert(items.bisect_left(2,lo=2,hi=4)==2)
let empty:List<Number>=[]
assert(empty.bisect_left(3)==0)
assert(empty.bisect_right(3)==0)
wait(1s)"#,
    );
}

#[test]
fn bisect_work_grows_with_tree_probes_without_rescanning_the_list() {
    let mut operations = Vec::new();
    for length in [1024, 32768] {
        let mut files = files(&format!(
            "let items=read<List<Number>>(\"sorted.json\")\nlet position=items.bisect_left({})\nassert(position=={})\nwait(1s)",
            length / 2,
            length / 2
        ));
        files.insert(
            "sorted.json",
            format!(
                "[{}]",
                (0..length)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        );
        let mut engine = Engine::default();
        engine.limits.max_operations = 64;
        let prepared = engine
            .prepare("main.sanim", &files, &Default::default())
            .unwrap();
        operations.push(prepared.statistics.operations);
    }
    assert!(
        operations[1] <= operations[0] + 6,
        "32× larger input should add at most five binary-search probes: {operations:?}"
    );
}

#[test]
fn callback_type_errors_and_bounds_fail_with_readable_context() {
    for (source, expected) in [
        (
            "fn wrong(value:String):String{return value}\nlet result=[1].map(wrong)",
            "callback",
        ),
        (
            "fn wrong(value:Number):Number{return value}\nlet result=[1].filter(wrong)",
            "Bool",
        ),
        (
            "fn wrong(value:Number):Number{return value}\nlet result=[1].flat_map(wrong)",
            "List<U>",
        ),
        ("let result=[1,2].take(-1)", "nonnegative"),
        ("let result=[1,2].bisect_left(1,lo=2,hi=1)", "lo <= hi"),
        ("let result=[{x=1}].sort()", "key="),
        (
            "type Row={x:Number}\nlet result=[1].map(Row)",
            "declared fn",
        ),
        (
            "mut rows:List<List<Number,2>>=[[1,2]]\nrows=append(rows,[3])",
            "type mismatch",
        ),
    ] {
        let error = Engine::default()
            .prepare("main.sanim", &files(source), &Default::default())
            .unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn overloaded_nested_generic_and_defaulted_callbacks_use_shared_dispatch() {
    prepare(
        r#"fn classify(value:Int < 0):String{return "negative"}
fn classify(value:Int >= 0):String{return "nonnegative"}
fn retain(value:Int < 0):Bool{return false}
fn retain(value:Int >= 0):Bool{return true}
fn accumulate(total:Number,value:Int < 0):Number{return total+value}
fn accumulate(total:Number,value:Int >= 0):Number{return total+value}
fn expand(value:Int < 0):List<String>{return ["negative"]}
fn expand(value:Int >= 0):List<Number>{return [value]}
fn size<T>(value:List<T>):Number{return len(value)}
fn shifted(value:Number,offset:Number=2):Number{return value+offset}
let integers:List<Int>=[-2,0,3]
let labels=integers.map(classify)
let kept=integers.filter(retain)
let total=integers.fold(0,accumulate)
let lengths=[[1,2],[3]].map(size)
let shifted_values=[1,2].map(shifted)
let mixed_items:List<Int>=[-1,0]
let mixed=mixed_items.flat_map(expand)
assert(labels==["negative","nonnegative","nonnegative"])
assert(kept==[0,3])
assert(total==1)
assert(lengths==[2,1])
assert(shifted_values==[3,4])
assert(len(mixed)==2)
assert(mixed[0]==Some("negative"))
assert(mixed[1]==Some(0))
wait(1s)"#,
    );
}

#[test]
fn callback_created_elements_receive_distinct_stable_iteration_identities() {
    let source = "fn make_dot(radius:Number):Element{return Circle(radius=radius*1pt)}\nlet shapes=[1,2,3].map(make_dot)\nwait(1s)";
    let first = prepare(source);
    let second = prepare(source);
    let ids = |prepared: &sanim_lang::Prepared| {
        prepared.bindings["shapes"]
            .list()
            .unwrap()
            .iter()
            .map(|value| {
                if let Value::Element(element) = value {
                    element.id.clone()
                } else {
                    panic!("Element output")
                }
            })
            .collect::<Vec<_>>()
    };
    let first = ids(&first);
    assert_eq!(first, ids(&second));
    assert_eq!(
        first
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}

#[test]
fn chained_call_result_and_generic_schemas_use_the_complete_call_span() {
    prepare(
        r#"fn same<T>(value:T):T{return value}
fn stringify<T>(value:T):String{return str(value)}
fn is_one(value:String):Bool{return value=="1"}
let words=[1,20].map(same).map(stringify).map(same)
let flags=[1,20].map(stringify).map(is_one)
assert(words==["1","20"])
assert(flags==[true,false])
wait(1s)"#,
    );
}

#[test]
fn duration_only_discards_map_outputs_without_forcing_nested_inputs_or_return_assets() {
    for flat in [false, true] {
        let (output, read) = if flat {
            ("List<Number>", "read<List<Number>>(\"absent.json\")")
        } else {
            ("Number", "read<Number>(\"absent.json\")")
        };
        let method = if flat { "flat_map" } else { "map" };
        let mut sources = files(&format!(
            "fn delayed(value:Number):{output} {{wait(value*1s);return {read}}}\nlet inputs=read<List<Number>>(\"inputs.json\")\nlet unused=inputs.{method}(delayed)\nwait(1s)"
        ));
        sources.insert("inputs.json", "[1,2]");
        let report = Engine::default()
            .duration("main.sanim", &sources, &Default::default())
            .unwrap();
        assert_eq!(report.duration, 4.into());
        assert!(!report.dependencies.contains_key("absent.json"));
        assert_eq!(report.statistics.decoded_assets, 1);
        assert_eq!(report.statistics.checkpoint_count, 0);
    }
    let source = r#"fn double(value:Number):Number{return value*2}
fn delayed(value:Number):Number{wait(value*1s);return read<Number>("absent.json")}
let unused=[1,2].map(double).map(delayed)
wait(1s)"#;
    let report = Engine::default()
        .duration("main.sanim", &files(source), &Default::default())
        .unwrap();
    assert_eq!(
        report.duration,
        7.into(),
        "inner map produces needed input even when the outer result is discarded"
    );
    assert!(!report.dependencies.contains_key("absent.json"));
}

#[test]
fn demand_keeps_asset_backed_callback_mutation_and_timing_in_source_order() {
    let mut sources = files(
        r#"mut calls=0
fn observe(value:Int < 0):Number{calls=calls+1;return value}
fn observe(value:Int >= 0):Number{calls=calls+1;return value}
fn delayed(value:Number):Number{wait(10ms);return value}
let unused_mutation=read<List<Int>>("items.json").map(observe)
assert(calls==3)
let unused_timing=read<List<Number>>("items.json").map(delayed)
wait(1s)"#,
    );
    sources.insert("items.json", "[-1,0,2]");
    let mut engine = Engine::default();
    engine.preparation = Preparation::Demand;
    let prepared = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    assert_eq!(prepared.animation.duration, Fraction::new(103u64, 100u64));
    assert!(prepared.bindings.contains_key("unused_mutation"));
    assert!(prepared.bindings.contains_key("unused_timing"));
}

#[test]
fn demand_callbacks_preserve_read_only_captures_at_the_let_declaration() {
    let mut sources = files(
        r#"mut factor=1
fn scaled(value:Number):Number{return value*factor}
let mapped=read<List<Number>>("items.json").map(scaled)
factor=2
assert(mapped==[1,2,3])
assert(factor==2)
wait(1s)"#,
    );
    sources.insert("items.json", "[1,2,3]");
    let mut engine = Engine::default();
    engine.preparation = Preparation::Demand;
    engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
}

#[test]
fn callback_effects_cannot_hide_inside_reflect_or_signal_captures() {
    for source in [
        "mut calls=0\nfn observe(value:Number):Number{calls=calls+1;return value}\nlet value=reflect([1].map(observe))",
        "mut calls=0\nfn observe(value:Number):Number{calls=calls+1;return value}\nsig value=[1].map(observe)",
        "fn delayed(value:Number):Number{wait(1s);return value}\nsig value=[1].map(delayed)",
        "mut calls=0\nfn key(value:Number):Number{calls=calls+1;return value}\nlet value=reflect([1].sort(key=key))",
    ] {
        let error = Engine::default()
            .prepare("main.sanim", &files(source), &Default::default())
            .unwrap_err();
        assert!(
            error.message.contains("pure") || error.message.contains("must not mutate"),
            "{source}: {error}"
        );
    }
}

#[test]
fn callback_iteration_limits_count_visits_without_restricting_indexed_queries() {
    let run = |source: &str| {
        let mut engine = Engine::default();
        engine.limits.max_iterations = 2;
        engine.prepare("main.sanim", &files(source), &Default::default())
    };
    let error = run("fn same(value:Number):Number{return value}\nlet result=[1,2,3].map(same)")
        .unwrap_err();
    assert!(
        error.message.contains("iteration limit") && error.message.contains("item index 2"),
        "{error}"
    );
    run("fn accumulate(a:Number,b:Number):Number{return a+b}\nlet result=[1,2,3].reduce(accumulate)\nassert(result==6)").unwrap();
    run("fn yes(value:Number):Bool{return true}\nlet found=[1,2,3,4,5].any(yes)\nassert(found)\nassert([1,2,3,4,5].bisect_left(4)==3)").unwrap();
    run("fn yes(value:Number):Bool{wait(1ms);return true}\nlet found=[1,2,3,4,5].any(yes)\nassert(found)").unwrap();
}

#[test]
fn sized_list_cardinalities_propagate_through_functional_and_standard_operations() {
    use sanim_core::ListLength::{Exact, Unknown};
    let prepared = prepare(
        r#"fn same(value:Number):Number{return value}
fn twins(value:Number):List<Number,2>{return [value,value]}
fn positive(value:Number):Bool{return value>0}
fn three(items:List<Number,3>):List<Number,3>{return items}
fn extend_values(items:List<Number>,value:Number):List<Number>{return append(items,value)}
let items=[1,2,3]
let mapped=items.map(same)
let checked=three(mapped)
let expanded=items.flat_map(twins)
let filtered=items.filter(positive)
let indexed=items.enumerate()
let zipped=items.zip(["a","b"])
let taken=items.take(1+1)
let dropped=items.drop(1)
let sliced=items.slice(start=1,end=9)
let reversed=items.reverse()
let sorted=items.sort()
let unique=items.unique()
let appended=append(items,4)
let inserted=insert(items,1,9)
let popped=pop(items)
let shifted=popleft(items)
let replaced=replace_at(items,1,9)
let moved=move_item(items,0,2)
let folded=items.fold([],extend_values)
let history=items.scan([],extend_values)
let empty:List<Number,0>=[]
let empty_mapped=empty.map(same)
let empty_filtered=empty.filter(positive)
let empty_unique=empty.unique()
let ranged=range(4)
let stepped=range(8,start=2,step=2)
let ragged=append([[1,2]],[3])
let ragged_inserted=insert([[1,2]],0,[3])
assert(folded==[1,2,3])
assert(len(history)==3)
assert(ragged==[[1,2],[3]])
assert(ragged_inserted==[[3],[1,2]])
wait(1s)"#,
    );
    for (name, length) in [
        ("items", Exact(3)),
        ("mapped", Exact(3)),
        ("checked", Exact(3)),
        ("expanded", Exact(6)),
        ("filtered", Unknown),
        ("indexed", Exact(3)),
        ("zipped", Exact(2)),
        ("taken", Exact(2)),
        ("dropped", Exact(2)),
        ("sliced", Exact(2)),
        ("reversed", Exact(3)),
        ("sorted", Exact(3)),
        ("unique", Unknown),
        ("appended", Exact(4)),
        ("inserted", Exact(4)),
        ("popped", Exact(2)),
        ("shifted", Exact(2)),
        ("replaced", Exact(3)),
        ("moved", Exact(3)),
        ("folded", Unknown),
        ("history", Exact(3)),
        ("empty_mapped", Exact(0)),
        ("empty_filtered", Exact(0)),
        ("empty_unique", Exact(0)),
        ("ranged", Exact(4)),
        ("stepped", Exact(3)),
    ] {
        let Type::List(_, actual) = &prepared.symbols[name] else {
            panic!("{name} must be List")
        };
        assert_eq!(*actual, length, "{name}: {}", prepared.symbols[name]);
    }
}

#[test]
#[ignore = "run make list-functions-bench"]
fn bisect_benchmark() {
    for length in [1024, 32768] {
        let mut files = files("let items=read<List<Number>>(\"sorted.json\")\nwait(0s)");
        files.insert(
            "sorted.json",
            format!(
                "[{}]",
                (0..length)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        );
        let mut engine = Engine::default();
        engine
            .prepare("main.sanim", &files, &Default::default())
            .unwrap();
        let mut samples = Vec::new();
        for sample in 0..9 {
            files.insert("main.sanim",format!("let items=read<List<Number>>(\"sorted.json\")\nlet revision={sample}\nrepeat 5000 {{ let position=items.bisect_left({}) }}\nwait(0s)",length/2));
            let started = std::time::Instant::now();
            let prepared = engine
                .prepare("main.sanim", &files, &Default::default())
                .unwrap();
            if sample > 0 {
                samples.push(started.elapsed());
            }
            assert!(prepared.statistics.operations < 150_000);
        }
        samples.sort();
        eprintln!("5000 indexed bisects, n={length}, median={:?}", samples[4]);
    }
}
