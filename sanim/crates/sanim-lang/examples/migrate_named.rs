use sanim_lang::{ast::{Expr,ExprKind,Statement,StatementKind},parser};
use std::{collections::BTreeSet,env,fs};
fn mark(expr:&Expr,text:&str,offsets:&mut BTreeSet<usize>) {
    let mut value=|value:&Expr|{
        let prefix=text[..value.span.start].trim_end();
        if prefix.ends_with(':') { offsets.insert(prefix.len()-1); }
    };
    match &expr.kind {
        ExprKind::Call{args,..} => {for (name,expr) in args {if name.is_some(){value(expr);}}}
        ExprKind::Record(fields) => {for (_,expr) in fields {value(expr);}}
        _=>{}
    }
}
fn statement(stmt:&Statement,text:&str,offsets:&mut BTreeSet<usize>) {
    if let StatementKind::Function{parameters,body,..}=&stmt.kind {
        for (_,_,value) in parameters {if let Some(value)=value {value.visit(&mut|expr|mark(expr,text,offsets));}}
        for stmt in body {statement(stmt,text,offsets);}
    } else {stmt.visit_expressions(&mut|expr|mark(expr,text,offsets));}
}
fn main() {
    for path in env::args().skip(1) {
        let text=fs::read_to_string(&path).unwrap();
        let module=parser::parse(&path,&text,10_000_000).unwrap();
        let mut offsets=BTreeSet::new();
        for stmt in &module.statements {statement(stmt,&text,&mut offsets);}
        let count=offsets.len();let mut bytes=text.into_bytes();
        for offset in offsets {bytes[offset]=b'=';}
        fs::write(&path,bytes).unwrap();
        println!("{path}: {count} value separators migrated");
    }
}
