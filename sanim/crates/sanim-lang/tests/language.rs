use sanim_core::{Fraction, Number, Type, Unit, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn prepare(text: &str) -> Prepared {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", text);
    Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_or_else(|e| panic!("{e}\n{text}"))
}
fn number(prepared: &Prepared, name: &str) -> Fraction {
    prepared.bindings[name].number().unwrap().value
}

#[test]
fn arithmetic_precedence_and_exact_time() {
    let p = prepare("let result = 2 + 3 * 4\nwait(0.1s + 200ms)\n");
    assert_eq!(number(&p, "result"), Fraction::from(14));
    assert_eq!(p.animation.duration, Fraction::new(3u64, 10u64));
}
#[test]
fn braces_ignore_indentation() {
    let p = prepare("fn twice(n: Number): Number {\nreturn n * 2\n}\nlet answer = twice(21)\n");
    assert_eq!(number(&p, "answer"), Fraction::from(42));
}
#[test]
fn semicolons_work() {
    assert_eq!(
        number(&prepare("let one = 1; let two = one + 1;"), "two"),
        Fraction::from(2)
    );
}
#[test]
fn typed_defaults_and_named_arguments() {
    let p = prepare(
        "fn greet(prefix: String, suffix: String = \"!\"): String { return prefix + suffix }\nlet a = greet(\"hi\")\nlet b = greet(suffix: \"?\", prefix: \"hey\")\n",
    );
    assert_eq!(p.bindings["a"], Value::String("hi!".into()));
    assert_eq!(p.bindings["b"], Value::String("hey?".into()));
}
#[test]
fn generic_identity_infers_and_accepts_explicit_types() {
    let p = prepare(
        "fn identity<T>(value: T): T { return value }\nlet a = identity(42)\nlet b = identity<String>(\"hello\")\n",
    );
    assert_eq!(number(&p, "a"), Fraction::from(42));
    assert_eq!(p.bindings["b"], Value::String("hello".into()));
}
#[test]
fn nested_generic_arguments() {
    let p = prepare(
        "fn identity<T>(value: T): T { return value }\nlet rows = identity<List<List<Number>>>([[1, 2], [3]])\n",
    );
    assert!(matches!(p.bindings["rows"], Value::List(_)));
}
#[test]
fn structural_records_and_constructor() {
    let p = prepare(
        "type Style = { fill: Color, size: Length }\nfn tile(style: Style): Length { return style.size }\nlet size = tile({ fill: Color(\"red\"), size: 12pt })\nlet style = Style(size: 16pt, fill: #00FF00)\n",
    );
    assert_eq!(number(&p, "size"), Fraction::from(12));
    assert_eq!(p.types["Style"], p.bindings["style"].ty());
}
#[test]
fn generic_record_alias() {
    let p = prepare("type Boxed<T> = { value: T }\nlet item: Boxed<Number> = { value: 7 }\n");
    assert!(matches!(p.bindings["item"], Value::Record(_)));
}
#[test]
fn exhaustive_enum_match() {
    let p = prepare(
        "enum Choice { Empty, Value(Number), }\nfn extract(choice: Choice): Number {\nmatch choice {\ncase Choice.Empty { return 0 }\ncase Choice.Value(value) { return value }\n}\n}\nlet result = extract(Choice.Value(12))\n",
    );
    assert_eq!(number(&p, "result"), Fraction::from(12));
}
#[test]
fn if_else_returns() {
    let p = prepare(
        "fn choose(value: Bool): String {\nif value { return \"yes\" } else { return \"no\" }\n}\nlet result = choose(false)\n",
    );
    assert_eq!(p.bindings["result"], Value::String("no".into()));
}
#[test]
fn options_make_indexing_total() {
    let p = prepare(
        "let values = [10, 20]\nlet missing = values[9]\nlet result = unwrap(missing, 42)\nlet present = unwrap(get(values, 1), 0)\n",
    );
    assert_eq!(number(&p, "result"), Fraction::from(42));
    assert_eq!(number(&p, "present"), Fraction::from(20));
}
#[test]
fn finite_for_schedules_in_order() {
    let p = prepare("for i in range(4) { wait(100ms) }\n");
    assert_eq!(p.animation.duration, Fraction::new(2u64, 5u64));
}
#[test]
fn plain_repeat_runs_sequentially() {
    let p = prepare("repeat 4 { wait(100ms) }\n");
    assert_eq!(p.animation.duration, Fraction::new(2u64, 5u64));
}
#[test]
fn timer_pads_each_iteration() {
    let p = prepare("timer 4 every 500ms { wait(100ms) }\n");
    assert_eq!(p.animation.duration, Fraction::from(2));
}
#[test]
fn scheduling_helpers_work() {
    let p = prepare(
        "fn pause(duration: Duration = 200ms): Unit { wait(duration) }\nrepeat 3 { pause() }\n",
    );
    assert_eq!(p.animation.duration, Fraction::new(3u64, 5u64));
}
#[test]
fn reflected_values_are_known_loop_bounds() {
    let p = prepare("reflect count: Number = 4\nrepeat count { wait(100ms) }\n");
    assert_eq!(p.animation.duration, Fraction::new(2u64, 5u64));
    assert_eq!(p.reflected[0].name, "count");
}
#[test]
fn reflection_describes_nested_types() {
    let p = prepare(
        "type Style = { fill: Color, width: Length, outline: Option<Color> }\nreflect style: Style = { fill: Color(\"red\"), width: 16pt, outline: None }\n",
    );
    assert!(matches!(p.reflected[0].ty, Type::Record(_)));
    assert_eq!(p.reflected[0].default, p.reflected[0].value);
}
#[test]
fn reflection_overrides_validate_before_use() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "reflect count: Number = 2\nrepeat count { wait(1s) }\n",
    );
    let overrides = BTreeMap::from([("count".into(), Value::Number(Number::scalar(3)))]);
    let p = Engine::default()
        .prepare("main.sanim", &sources, &overrides)
        .unwrap();
    assert_eq!(p.animation.duration, Fraction::from(3));
}
#[test]
fn coordinate_units_and_percentages() {
    let p = prepare("let length = 96px\nlet center = Vec2(50%, 25%)\n");
    assert_eq!(number(&p, "length"), Fraction::from(72));
    let Value::Vec2(point) = p.bindings["center"] else {
        panic!()
    };
    assert!(point.unit == Unit::Percent);
    assert_eq!(point.values[0], 0.5);
}
#[test]
fn short_circuit_avoids_division_by_zero() {
    let p = prepare("let value = false and (1 / 0 == 0)\nlet other = true or (1 / 0 == 0)\n");
    assert_eq!(p.bindings["value"], Value::Bool(false));
    assert_eq!(p.bindings["other"], Value::Bool(true));
}
#[test]
fn utf8_strings_and_comments() {
    let p =
        prepare("# comment\nlet label = \"日本語\\nhello\" // comment\nlet color = #12345678\n");
    assert_eq!(p.bindings["label"], Value::String("日本語\nhello".into()));
}
#[test]
fn pure_numeric_helpers() {
    let p = prepare("let a = clamp(12pt, 2pt, 8pt)\nlet b = abs(-3)\nlet c = max(1s, 500ms)\n");
    assert_eq!(number(&p, "a"), Fraction::from(8));
    assert_eq!(number(&p, "b"), Fraction::from(3));
    assert_eq!(p.bindings["c"].number().unwrap().unit, Unit::Duration);
}

macro_rules! reject {
    ($name:ident,$text:expr,$code:expr) => {
        #[test]
        fn $name() {
            let mut sources = MemorySources::default();
            sources.insert("main.sanim", $text);
            let error = Engine::default()
                .prepare("main.sanim", &sources, &BTreeMap::new())
                .unwrap_err();
            assert_eq!(error.code, $code, "{error}");
            assert!(!error.message.is_empty());
        }
    };
}
reject!(protected_constructor, "let Color = 1\n", "S101");
reject!(
    protected_parameter,
    "fn f(Color: String): String { return Color }\n",
    "S101"
);
reject!(protected_type, "type Circle = { radius: Number }\n", "S101");
reject!(protected_alias, "import \"lib\" as Color\n", "S101");
reject!(
    no_shadowing,
    "let item = 1\nif true { let item = 2 }\n",
    "S102"
);
reject!(no_mutation, "let item = 1\nitem = 2\n", "S002");
reject!(wrong_units, "let value = 1pt + 1s\n", "S106");
reject!(
    wrong_field_type,
    "type Style = { fill: Color }\nlet style: Style = { fill: \"red\" }\n",
    "S106"
);
reject!(
    misspelled_field,
    "type Style = { width: Length }\nlet style: Style = { widht: 1pt }\n",
    "S106"
);
reject!(
    unknown_argument,
    "let marker = Circle(raduis: 4pt)\n",
    "S124"
);
reject!(
    duplicate_argument,
    "let marker = Circle(radius: 1pt, radius: 2pt)\n",
    "S124"
);
reject!(unknown_name, "let marker = Circel()\n", "S120");
reject!(division_zero, "let bad = 1 / 0\n", "S305");
reject!(
    direct_recursion,
    "fn f(value: Number): Number { return f(value) }\n",
    "S111"
);
reject!(
    mutual_recursion,
    "fn f(value: Number): Number { return g(value) }\nfn g(value: Number): Number { return f(value) }\n",
    "S111"
);
reject!(
    missing_return,
    "fn f(value: Bool): Number { if value { return 1 } }\n",
    "S110"
);
reject!(
    unreachable_code,
    "fn f(): Number { return 1; let unused = 2 }\n",
    "S112"
);
reject!(
    nonexhaustive_match,
    "enum Choice { A, B }\nfn f(value: Choice): Number { match value { case Choice.A { return 1 } } }\n",
    "S117"
);
reject!(
    reflect_in_branch,
    "if true { reflect amount = 1 }\n",
    "S113"
);
reject!(read_needs_type, "let data = read(\"data.csv\")\n", "S125");
#[test]
fn unused_read_helper_does_not_read_its_asset() {
    let p = prepare("fn load(): String { return read<String>(\"absent.json\") }");
    assert!(!p.dependencies.contains_key("absent.json"));
}
reject!(negative_repeat, "repeat -1 { wait(1s) }\n", "S303");
reject!(fractional_repeat, "repeat 1.5 { wait(1s) }\n", "S303");
reject!(
    timer_overrun,
    "timer 2 every 100ms { wait(200ms) }\n",
    "S304"
);
reject!(zero_timer_period, "timer 2 every 0s { wait(1s) }\n", "S304");
reject!(negative_wait, "wait(-1s)\n", "S304");
reject!(
    invalid_color,
    "let color = Color(\"not-a-color\")\n",
    "S305"
);
reject!(invalid_opacity, "let marker = Circle(opacity: 2)\n", "S305");
reject!(
    unsupported_unbounded_loop,
    "while true { wait(1s) }\n",
    "S002"
);
reject!(heterogeneous_list, "let values = [1, \"two\"]\n", "S106");
reject!(unknown_generic, "let value: List<Missing> = []\n", "S105");
reject!(
    generic_arity,
    "let value: List<Number, String> = []\n",
    "S105"
);
reject!(
    required_after_default,
    "fn f(a: Number = 1, b: Number): Number { return b }\n",
    "S124"
);
reject!(
    invalid_default,
    "fn f(a: Number = \"bad\"): Number { return a }\n",
    "S106"
);
reject!(malformed_string, "let text = \"unterminated\n", "S002");
reject!(missing_brace, "repeat 2 { wait(1s)\n", "S002");

#[test]
fn operation_budget_stops_nested_bounded_work() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "repeat 100 { repeat 100 { wait(1ms) } }\n");
    let mut engine = Engine::default();
    engine.limits.max_operations = 100;
    assert_eq!(
        engine
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S303"
    );
}
#[test]
fn reflection_rejects_unknown_override() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "reflect count = 1\n");
    let input = BTreeMap::from([("missing".into(), Value::Number(Number::scalar(2)))]);
    assert_eq!(
        Engine::default()
            .prepare("main.sanim", &sources, &input)
            .unwrap_err()
            .code,
        "S302"
    );
}
#[test]
fn reflection_checks_every_list_item() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "reflect values: List<Number> = [1]\n");
    let input = BTreeMap::from([(
        "values".into(),
        Value::List(vec![
            Value::Number(Number::scalar(1)),
            Value::String("bad".into()),
        ]),
    )]);
    assert_eq!(
        Engine::default()
            .prepare("main.sanim", &sources, &input)
            .unwrap_err()
            .code,
        "S302"
    );
}

#[test]
fn powers_are_right_associative_and_integral_powers_exact() {
    let p = prepare(
        "let a=2**3**2\nlet b=-2**2\nlet c=2**-3\nlet d=(-2)**3\nlet e=pow(Fraction(2,3),3)\n",
    );
    assert_eq!(number(&p, "a"), Fraction::from(512));
    assert_eq!(number(&p, "b"), -Fraction::from(4));
    assert_eq!(number(&p, "c"), Fraction::new(1u64, 8u64));
    assert_eq!(number(&p, "d"), -Fraction::from(8));
    assert_eq!(number(&p, "e"), Fraction::new(8u64, 27u64));
}
#[test]
fn common_math_functions_have_clear_units_and_constants() {
    let p = prepare(
        "let a=log(8,base:2)\nlet b=sin(90deg)\nlet c=cos(PI)\nlet d=sqrt(9)\nlet e=floor(1.9pt)\nlet f=hypot(3pt,4pt)\nlet g=abs(sin(PI)) < 0.000001\nlet h=TAU > PI and E > 2\n",
    );
    for (name, value) in [("a", 3), ("b", 1), ("c", -1), ("d", 3), ("e", 1), ("f", 5)] {
        assert_eq!(number(&p, name), Fraction::from(value));
    }
    assert_eq!(p.bindings["e"].number().unwrap().unit, Unit::Length);
    assert_eq!(p.bindings["f"].number().unwrap().unit, Unit::Length);
    assert_eq!(p.bindings["g"], Value::Bool(true));
    assert_eq!(p.bindings["h"], Value::Bool(true));
}
#[test]
fn math_errors_do_not_publish_nan_or_implicitly_drop_units() {
    for source in [
        "let x=log(0)",
        "let x=log(8,base:1)",
        "let x=sqrt(-1)",
        "let x=asin(2)",
        "let x=exp(1000)",
        "let x=0**-1",
        "let x=(-2)**0.5",
        "let x=2pt**2",
        "let x=log(2s)",
        "let PI=3",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        assert!(
            Engine::default()
                .prepare("main.sanim", &files, &BTreeMap::new())
                .is_err(),
            "{source}"
        );
    }
}
#[test]
fn incomplete_types_cannot_escape_into_bindings() {
    for source in ["let x=[]", "let x=None", "let x:Any=1", "let x:Unknown=1"] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        assert!(
            Engine::default()
                .prepare("main.sanim", &files, &BTreeMap::new())
                .is_err(),
            "{source}"
        );
    }
    prepare("let values:List<Number> = []\nlet absent:Option<Number> = None\n");
}

#[test]
fn math_helpers_preserve_angle_units_and_fraction_values() {
    let p = prepare(
        "let angle_length=hypot(3deg,4deg)\nlet exact=abs(Fraction(-2,3))\nlet rounded=floor(Fraction(5,3))",
    );
    assert_eq!(
        p.bindings["angle_length"].number().unwrap().value,
        Fraction::from(5)
    );
    assert_eq!(
        p.bindings["angle_length"].number().unwrap().unit,
        Unit::Angle
    );
    assert_eq!(
        p.bindings["exact"],
        Value::Fraction(Fraction::new(2u64, 3u64))
    );
    assert_eq!(p.bindings["rounded"], Value::Fraction(Fraction::from(1)));
}

#[test]
fn adwaita_colors_work_in_reflection_and_typed_data_loading() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "reflect tint:Color=Color(\"BLUE3\")\nlet loaded=read<Color>(\"color.json\")\nlet default_color=Color()\nadd(Circle(fill:tint))");
    files.insert("color.json", "\"orange3\"");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        prepared.bindings["tint"],
        Value::Color(sanim_core::Color::parse("#3584e4").unwrap())
    );
    assert_eq!(
        prepared.bindings["loaded"],
        Value::Color(sanim_core::Color::parse("#ff7800").unwrap())
    );
    assert_eq!(
        prepared.bindings["default_color"],
        Value::Color(sanim_core::Color::parse("red3").unwrap())
    );
}

#[test]
fn equals_names_arguments_and_record_values_while_colons_declare_types() {
    let p = prepare(
        "type Style={radius:Length,color:Color}\nfn marker(style:Style,offset:Vec2<Length>=Vec2(0pt,0pt)):Element{return Circle(radius=style.radius,fill=style.color,position=offset)}\nlet style:Style={radius=12pt,color=Color(\"blue3\")}\nlet created=marker(style=style,offset=Vec2(10pt,20pt))\nadd(created)",
    );
    let element = &p.animation.layers[0].element;
    assert_eq!(element.position.values, [10.0, 20.0]);
    assert_eq!(element.fill, sanim_core::Color::parse("blue3").unwrap());
}

#[test]
fn equals_named_arguments_do_not_consume_comparison_operators() {
    let p = prepare(
        "fn choose(flag:Bool,count:Number=3):Number{if flag{return count};return 0}\nlet a=choose(1==1,count=7)\nlet b=choose(flag=2>=1,count=9)\nlet c={same=1==1,greater=2>=1}\nassert(c.same and c.greater)",
    );
    assert_eq!(number(&p, "a"), Fraction::from(7));
    assert_eq!(number(&p, "b"), Fraction::from(9));
}

#[test]
fn dictionary_entries_keep_python_colons_with_equals_call_arguments() {
    let p = prepare(
        "let d:Dict={\"paint\":\"blue3\",\"nested\":{\"count\":2}}\nlet count=decode<Number>(d[\"nested\"][\"count\"])\nlet marker=Circle(fill=decode<Color>(d[\"paint\"]))\nadd(marker)",
    );
    assert_eq!(number(&p, "count"), Fraction::from(2));
    assert!(
        Engine::default()
            .prepare(
                "main.sanim",
                &{
                    let mut files = MemorySources::default();
                    files.insert("main.sanim", "let d={\"key\"=1}");
                    files
                },
                &BTreeMap::new()
            )
            .is_err()
    );
}
