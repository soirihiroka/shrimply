use sanim_lang::{ast::*, parser};

#[test]
fn recovery_retains_canonical_declarations_and_nested_call_context() {
    let text = "fn helper(value:Number):Number {return value}\nlet amount=1\nlet dot=Circle(radius=helper(";
    let recovered = parser::parse_recovering("main.sanim", text, 100_000);
    assert!(recovered.diagnostic.is_some());
    assert_eq!(recovered.module.statements.len(), 2);
    assert_eq!(recovered.pending_calls.len(), 2);
    let inner = recovered.pending_calls.last().unwrap();
    assert!(matches!(&inner.callee.kind,ExprKind::Name(name) if name=="helper"));
    assert_eq!(inner.argument, 0);
    assert_eq!(
        recovered.pending_calls[0].argument_name.as_deref(),
        Some("radius")
    );
    assert!(parser::parse("main.sanim", &text[..recovered.prefix_end], 100_000).is_ok());
}

#[test]
fn successful_recovery_has_no_pending_calls_and_keeps_parameter_positions() {
    let text = "fn helper(first:Number, second:Number=2):Number {return first+second}\nlet value=helper(1)";
    let recovered = parser::parse_recovering("main.sanim", text, 100_000);
    assert!(recovered.diagnostic.is_none());
    assert!(recovered.pending_calls.is_empty());
    let function = &recovered.module.statements[0];
    let StatementKind::Function { parameters, .. } = &function.kind else {
        panic!("function")
    };
    let names = parameters
        .iter()
        .map(|parameter| &text[parameter.name_span.start..parameter.name_span.end])
        .collect::<Vec<_>>();
    assert_eq!(names, ["first", "second"]);
}
