use sanim_core::Diagnostic;
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn prepare(source: &str) -> Result<Prepared, Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}

#[test]
fn sized_dispatch_uses_constant_work_and_bounds_unproven_element_checks() {
    let mut work = Vec::new();
    for count in [64, 8192] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("fn size(items:List<Number,{count}>):Number{{return len(items)}}\nfn size(items:List<Number,1>):Number{{return len(items)}}\nlet values=read<List<Number,{count}>>(\"values.json\")\nrepeat 20{{let result=size(values)}}\nAnimation()"));
        files.insert("values.json", format!("[{}]", vec!["1"; count].join(",")));
        let mut engine = Engine::default();
        engine.limits.max_operations = 1000;
        work.push(
            engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap()
                .statistics
                .operations,
        );
    }
    assert_eq!(
        work[0], work[1],
        "dispatch work must depend on clauses, not payload size"
    );

    let mut files = MemorySources::default();
    files.insert("main.sanim", "fn size(items:List<\"a\",1024>):Number{return len(items)}\nfn size(items:List<\"b\",1024>):Number{return len(items)}\nlet values=read<List<String,1024>>(\"values.json\")\nlet result=size(values)\nAnimation()");
    files.insert(
        "values.json",
        format!("[{}]", vec!["\"a\""; 1024].join(",")),
    );
    let mut engine = Engine::default();
    engine.limits.max_operations = 100;
    let error = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S303", "{error}");
    assert!(error.message.contains("clause validation"), "{error}");
}

#[test]
fn string_and_numeric_clauses_dispatch_actual_values_with_named_arguments() {
    prepare(
        r#"
fn describe(mode:"before"):String{return "old"}
fn describe(mode:"after"):String{return "new"}
fn classify(value:Int<1):String{return "low"}
fn classify(value:Int>=1):String{return "high"}
assert(describe(mode="before")=="old")
assert(describe("after")=="new")
mut count=0
assert(classify(count)=="low")
count=2
assert(classify(value=count)=="high")
"#,
    )
    .unwrap();
}

#[test]
fn clause_defaults_and_generics_keep_canonical_call_binding() {
    prepare(
        r#"
fn select<T>(mode:"before",value:T,suffix:String="!"):T{return value}
fn select<T>(mode:"after",value:T,suffix:String="?"):T{return value}
assert(select("before",7)==7)
assert(select(value="yes",mode="after")=="yes")
"#,
    )
    .unwrap();
}

#[test]
fn signal_handles_survive_dispatch_and_mut_parameters_remain_local() {
    prepare(
        r#"
fn<sig> scale(sig value:Number,mode:"double"):sig Number{return value*2}
fn<sig> scale(sig value:Number,mode:"triple"):sig Number{return value*3}
fn<mut> increment(mut value:Number,mode:"one"):Number{value=value+1;return value}
fn<mut> increment(mut value:Number,mode:"two"):Number{value=value+2;return value}
mut count=2
let output=scale(count,"triple")
assert(output==6)
count=4
assert(output==12)
assert(increment(count,"two")==6)
assert(count==4)
"#,
    )
    .unwrap();
}

#[test]
fn imported_groups_use_each_clauses_checked_signature() {
    let mut files = MemorySources::default();
    files.insert(
        "library.sanim",
        r#"
fn title(mode:"before"):String{return "first"}
fn title(mode:"after"):String{return "second"}
"#,
    );
    files.insert(
        "main.sanim",
        "import \"./library.sanim\" as library\nassert(library.title(\"before\")==\"first\")\nassert(library.title(mode=\"after\")==\"second\")\n\nAnimation()",
    );
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
}

fn call_chain(defaults: bool) -> MemorySources {
    let mut files = MemorySources::default();
    for index in 0..5 {
        let body = if index == 4 {
            if defaults {
                "fn step(value:Number=0):Number{return value}"
            } else {
                "fn step():Number{return 0}"
            }
            .to_owned()
        } else {
            let declaration = if defaults {
                "fn step(value:Number=next.step()):Number{return value}"
            } else if index == 0 {
                "fn step(n:Int<1):Number{return next.step()}\nfn step(n:Int>=1):Number{return next.step()}"
            } else {
                "fn step():Number{return next.step()}"
            };
            format!(
                "import \"./module{}.sanim\" as next\n{declaration}",
                index + 1
            )
        };
        files.insert(format!("module{index}.sanim"), body);
    }
    files.insert(
        "main.sanim",
        format!(
            "{}\nAnimation()",
            if defaults {
                "import \"./module0.sanim\" as api\nlet result=api.step()"
            } else {
                "import \"./module0.sanim\" as api\nlet result=api.step(2)"
            }
        ),
    );
    files
}
#[test]
fn guarded_calls_preserve_the_runtime_call_depth_limit() {
    let mut engine = Engine::default();
    engine.limits.max_call_depth = 4;
    let error = engine
        .prepare("main.sanim", &call_chain(false), &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S303", "{error:?}");
    assert!(error.message.contains("call depth"));
}
#[test]
fn defaults_are_also_inside_the_bounded_call_lifetime() {
    let mut engine = Engine::default();
    engine.limits.max_call_depth = 4;
    let error = engine
        .prepare("main.sanim", &call_chain(true), &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S303", "{error:?}");
    assert!(error.message.contains("call depth"));
}

#[test]
fn signal_clause_selection_occurs_at_invocation_and_keeps_the_live_argument() {
    prepare(
        r#"
fn<sig> guarded(sig value:Number<0):sig Number{return value*2}
fn<sig> guarded(sig value:Number>=0):sig Number{return value*3}
mut source=-2
let original=guarded(source)
assert(original == -4)
source=2
assert(original == 4)
let later=guarded(source)
assert(later == 6)
"#,
    )
    .unwrap();
}

#[test]
#[ignore = "manual guarded call overhead measurement"]
fn guarded_call_benchmark() {
    use std::{hint::black_box, time::Instant};
    let mut measurements = Vec::new();
    for guarded in [false, true] {
        let declaration = if guarded {
            "fn category(value:Number<0):Number{return 1}\nfn category(value:Number>=0):Number{return 1}"
        } else {
            "fn category(value:Number):Number{return 1}"
        };
        let source = format!(
            "{declaration}\nmut total=0\nrepeat 1000 {{total=total+category(1)}}\nassert(total==1000)"
        );
        let mut times = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            let prepared = prepare(&source).unwrap();
            black_box(&prepared);
            times.push(start.elapsed());
        }
        times.sort_unstable();
        measurements.push(times[times.len() / 2]);
    }
    eprintln!(
        "1000 actual helper invocations: plain={:?}, two guarded clauses={:?}",
        measurements[0], measurements[1]
    );
}
