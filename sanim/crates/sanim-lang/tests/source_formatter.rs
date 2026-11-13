use sanim_lang::{
    formatter::{FormatOptions, format_source},
    parser,
};

fn format(source: &str) -> String {
    format_source("main.sanim", source, FormatOptions::default(), 1_000_000).unwrap()
}
fn normalized(source: &str) -> serde_json::Value {
    fn strip(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                object.remove("span");
                object.remove("comments");
                object.remove("name_span");
                object.remove("name_span");
                for value in object.values_mut() {
                    strip(value);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    strip(value)
                }
            }
            _ => {}
        }
    }
    let mut value =
        serde_json::to_value(parser::parse("main.sanim", source, 1_000_000).unwrap()).unwrap();
    strip(&mut value);
    value
}
fn parity(source: &str) -> String {
    let formatted = format(source);
    assert_eq!(normalized(source), normalized(&formatted), "{formatted}");
    assert_eq!(
        format(&formatted),
        formatted,
        "formatter must be idempotent"
    );
    formatted
}

#[test]
fn blocks_generics_operators_and_small_containers_have_stable_layout() {
    let source = "type Row={x:Number,y:Number}\nfn choose<T:Numeric=Number>(a:T,b:T):T{if a<b{return -a+b**2}else{return a}}\nlet rows=read<List<Row>>(\"rows.json\")\nlet d={\"key\": [1,2,3]}\nlet v=d[\"key\"]\n";
    let formatted = parity(source);
    assert!(formatted.contains("if a < b {"), "{formatted}");
    assert!(
        formatted.contains("read<List<Row>>(\"rows.json\")"),
        "{formatted}"
    );
    assert!(formatted.contains("d[\"key\"]"), "{formatted}");
    assert!(formatted.contains("-a + b ** 2"), "{formatted}");
}

#[test]
fn comments_raw_strings_and_formatted_literals_are_preserved_verbatim() {
    let source = "# leading  comment\nlet data=r\"{ \\\"key\\\": 10 }\" # trailing let fn\nlet label=\"value {round(2.5):>08.2f}; literal {{x}}\"\nlet tint=Color(\"#3584e4\")\ntype Pair={ # type comment\nx:Number, # field comment\ny:Number}\nfn show( # argument comment\na:Number):Number{ # body comment\nreturn a # result comment\n}\n";
    let formatted = parity(source);
    for text in [
        "# leading  comment",
        "# trailing let fn",
        "# type comment",
        "# field comment",
        "# argument comment",
        "# body comment",
        "# result comment",
        "r\"{ \\\"key\\\": 10 }\"",
        "\"value {round(2.5):>08.2f}; literal {{x}}\"",
        "#3584e4",
    ] {
        assert!(formatted.contains(text), "missing {text} in {formatted}");
    }
}

#[test]
fn multiline_sql_and_raw_templates_keep_their_contents_and_indentation() {
    let source = r####"let sql="""
  SELECT "value", 'quoted'
  FROM samples
  WHERE value > {2:.1f}
"""
let literal=r'''{ literal }
  # still inside string
'''
"####;
    let formatted = parity(source);
    assert!(formatted.contains("\n  SELECT \"value\", 'quoted'\n  FROM samples\n"));
    assert!(formatted.contains("r'''{ literal }\n  # still inside string\n'''"));
}

#[test]
fn generic_lookahead_rollback_and_nested_literal_expressions_do_not_duplicate_source() {
    for source in [
        "let a=1\nlet b=2\nlet yes=a<b+1\n",
        "let label=\"nested {str(1,format=\"04d\")}\"\n",
        "let yes=not(false or true)\n",
        "let empty={}\nlet items=[]\n",
        "let d={ # comment in an empty literal\n}\n",
        "let duration=4m1s+0.5s\nwait(duration)\n",
    ] {
        parity(source);
    }
}

#[test]
fn wrapping_is_only_inserted_where_the_parser_accepts_continuations() {
    let source = "let values=[123456789,234567890,345678901,456789012]\nlet point=Vec2(123456789pt,234567890pt)\n";
    let options = FormatOptions {
        line_width: 20,
        ..Default::default()
    };
    let formatted = format_source("main.sanim", source, options, 1_000_000).unwrap();
    assert_eq!(normalized(source), normalized(&formatted));
    assert_eq!(
        format_source("main.sanim", &formatted, options, 1_000_000).unwrap(),
        formatted
    );
    assert!(formatted.contains('\n'));
}

#[test]
fn invalid_source_fails_before_any_replacement_can_be_published() {
    let error = format_source(
        "broken.sanim",
        "let broken=(",
        Default::default(),
        1_000_000,
    )
    .unwrap_err();
    assert_eq!(error.source, "broken.sanim");
    assert_eq!(error.code, "S002");
    assert!(
        format_source(
            "main.sanim",
            "wait(1s)",
            FormatOptions {
                indent_width: 0,
                ..Default::default()
            },
            1_000_000
        )
        .is_err()
    );
}

#[test]
fn all_repository_examples_preserve_the_canonical_ast() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("sanim") {
            parity(&std::fs::read_to_string(path).unwrap());
        }
    }
}

#[test]
#[ignore = "measured by make source-format-bench"]
fn source_formatter_benchmark() {
    let mut source = String::new();
    for index in 0..2000 {
        source.push_str(&format!(
            "let value_{index}=Vec2(12pt,-34pt) # retain this comment\n"
        ));
    }
    let started = std::time::Instant::now();
    let formatted = format(&source);
    eprintln!(
        "source formatter: {} bytes, 2000 declarations, {} us",
        source.len(),
        started.elapsed().as_micros()
    );
    assert_eq!(normalized(&source), normalized(&formatted));
}

#[test]
fn named_arguments_use_equals_and_markup_text_is_preserved() {
    let formatted = parity(
        "let marker=Circle(radius=12pt,fill=Color(\"blue3\"))\nlet label=<Label> literal  # text {round(2.5):.1f} </Label>\n",
    );
    assert!(formatted.contains("radius = 12pt"), "{formatted}");
    assert!(formatted.contains("<Label> literal  # text {round(2.5):.1f} </Label>"));
}

#[test]
fn colon_arguments_are_rejected_while_type_and_dictionary_colons_remain_valid() {
    for source in [
        "let marker=Circle(radius:12pt)",
        "fn helper(value:Number):Number{return value}\nhelper(value:1)",
    ] {
        let error = format_source("main.sanim", source, Default::default(), 100_000).unwrap_err();
        assert_eq!(error.code, "S002");
        assert!(error.message.contains("named arguments use '='"), "{error}");
    }
    parity(
        "type Pair={x:Number,y:Number}\nfn identity(value:Pair):Pair{return value}\nlet data={\"x\":1}\nlet pair=identity(value={x=1,y=2})\n",
    );
}

#[test]
fn hash_comments_have_no_hex_exception_and_strings_keep_hashes() {
    let source =
        "#abcdef\n#12345678\nlet text=\"#abcdef // literal\" # cafe\nlet tint=Color(\"#3584e4\")\n";
    let formatted = parity(source);
    assert!(formatted.starts_with("#abcdef\n#12345678\n"));
    assert_eq!(
        parity("# keep trailing spaces  "),
        "# keep trailing spaces  \n"
    );
    let module = parser::parse("main.sanim", &formatted, 100_000).unwrap();
    assert_eq!(module.statements.len(), 2);
    assert!(parser::parse("main.sanim", "// comment\nwait(1s)", 100_000).is_err());
    assert!(parser::parse("main.sanim", "/* comment */\nwait(1s)", 100_000).is_err());
}

#[test]
fn all_control_flow_and_type_operations_preserve_the_canonical_ast() {
    for source in [
        "enum Choice{Empty,Value(Number)}\nfn extract(choice:Choice):Number{match choice{case Choice.Empty{return 0} case Choice.Value(value){return value}}}\n",
        "import \"shared.sanim\" as shared\nmut phase=0deg\nsig size=sin(phase)\nbackground{animate(phase,to=360deg,duration=1s)}\nrepeat 3{wait(1ms)}\ntimer 4 every 500ms{wait(100ms)}\nfor value in [1,2,3]{assert(value>0)}\n",
        "type XY={x:Number,y:Number}\ntype Combined=XY & {z:Number}\ntype Smaller=Combined - {x:Number}\ntype Choice={x:Number}|{y:Number}\n",
        "fn transform<T:Numeric=Number>(input:T,scale:Number=2):T{return input*scale}\nlet size=reflect(64pt,label=\"Base Size\",key=\"shapes.size\")\n",
    ] {
        parity(source);
    }
}
