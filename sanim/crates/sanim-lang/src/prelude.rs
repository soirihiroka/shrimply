use crate::{native, native_types};
use sanim_core::{NativeEnum, Type, Unit as Dimension};

#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: &'static str,
    pub ty: Type,
    pub required: bool,
}
pub(crate) type Evaluator = for<'a> fn(
    &str,
    &std::collections::BTreeMap<String, sanim_core::Value>,
    native::Context<'a>,
) -> Result<sanim_core::Value, String>;
pub(crate) type TypeRule = fn(&Builtin, &[Type]) -> Result<Type, String>;
#[derive(Clone, Copy, Debug)]
pub(crate) enum Execution {
    Native(Evaluator),
    Wait(Evaluator),
    Read,
    Decode,
    Query,
    Play,
    Assertion,
}
#[derive(Clone, Debug)]
pub struct Builtin {
    pub name: &'static str,
    pub parameters: Vec<Parameter>,
    pub result: Type,
    pub schedule: bool,
    pub(crate) execute: Execution,
    pub(crate) check: TypeRule,
}

pub fn builtins() -> &'static [Builtin] {
    static BUILTINS: std::sync::OnceLock<Vec<Builtin>> = std::sync::OnceLock::new();
    BUILTINS.get_or_init(build_builtins)
}

fn build_builtins() -> Vec<Builtin> {
    use Type::*;
    let num = Numeric(Dimension::Number);
    let length = Numeric(Dimension::Length);
    let duration = Numeric(Dimension::Duration);
    let angle = Numeric(Dimension::Angle);
    let p = |name, ty, required| Parameter { name, ty, required };
    let mut out = vec![];
    let mut add = |name, parameters, result, schedule, execute, check| {
        out.push(Builtin {
            name,
            parameters,
            result,
            schedule,
            execute,
            check,
        })
    };
    add(
        "Color",
        vec![p("value", String, false)],
        Color,
        false,
        Execution::Native(native::color),
        native_types::fixed,
    );
    add(
        "Fraction",
        vec![
            p("numerator", num.clone(), false),
            p("denominator", num.clone(), false),
        ],
        Fraction,
        false,
        Execution::Native(native::fraction),
        native_types::fixed,
    );
    for (name, n) in [("Vec2", 2), ("Vec3", 3), ("Vec4", 4)] {
        let names = ["x", "y", "z", "w"];
        add(
            name,
            names[..n]
                .iter()
                .map(|name| p(name, Unknown, false))
                .collect(),
            Vector(n, None),
            false,
            Execution::Native(native::vector),
            native_types::vector,
        );
    }
    add(
        "Matrix4",
        vec![p(
            "rows",
            List(Box::new(List(Box::new(num.clone())))),
            false,
        )],
        Matrix4,
        false,
        Execution::Native(native::matrix),
        native_types::fixed,
    );
    add(
        "dot",
        vec![p("a", Unknown, true), p("b", Unknown, true)],
        num.clone(),
        false,
        Execution::Native(native::dot),
        native_types::dot,
    );
    for name in ["inverse", "transpose"] {
        add(
            name,
            vec![p("matrix", Matrix4, true)],
            Matrix4,
            false,
            Execution::Native(native::matrix_operation),
            native_types::fixed,
        );
    }
    let common = || {
        vec![
            p("id", String, false),
            p("position", Vector(2, None), false),
            p("x_align", sanim_core::Align::ty(), false),
            p("y_align", sanim_core::Align::ty(), false),
            p("fill", Color, false),
            p("opacity", num.clone(), false),
            p("rotation", angle.clone(), false),
            p("scale", num.clone(), false),
        ]
    };
    let mut params = common();
    params.push(p("radius", length.clone(), false));
    add(
        "Circle",
        params,
        Element,
        false,
        Execution::Native(native::element),
        native_types::fixed,
    );
    for name in ["Rectangle", "Rect"] {
        let mut params = common();
        params.extend([
            p("width", length.clone(), false),
            p("height", length.clone(), false),
        ]);
        add(
            name,
            params,
            Element,
            false,
            Execution::Native(native::element),
            native_types::fixed,
        );
    }
    for name in ["Group", "Row", "Column"] {
        let mut params = common();
        params.extend([
            p("children", List(Box::new(Element)), true),
            p("padding", length.clone(), false),
            p("width", length.clone(), false),
            p("height", length.clone(), false),
        ]);
        if name != "Group" {
            params.extend([
                p("gap", length.clone(), false),
                p("justify", sanim_core::Justify::ty(), false),
                p("align", sanim_core::Align::ty(), false),
                p("wrap", Bool, false),
            ]);
        }
        add(
            name,
            params,
            Element,
            false,
            Execution::Native(native::element),
            native_types::fixed,
        );
    }
    let mut params = common();
    params.extend([
        p("end", Vector(2, Some(Dimension::Length)), true),
        p("width", length.clone(), false),
    ]);
    add(
        "Line",
        params,
        Element,
        false,
        Execution::Native(native::element),
        native_types::fixed,
    );
    let mut params = common();
    params.push(p(
        "points",
        List(Box::new(Vector(2, Some(Dimension::Length)))),
        true,
    ));
    add(
        "Polygon",
        params,
        Element,
        false,
        Execution::Native(native::element),
        native_types::fixed,
    );
    let mut params = vec![p("text", String, true), p("font", Font, true)];
    params.extend(common());
    params.push(p("font_size", length.clone(), false));
    add(
        "Text",
        params,
        Element,
        false,
        Execution::Native(native::element),
        native_types::fixed,
    );
    for (name, ty) in [
        ("MoveTo", Vector(2, None)),
        ("Shift", Vector(2, None)),
        ("RotateTo", angle),
        ("ScaleTo", num.clone()),
        ("FillTo", Color),
    ] {
        add(
            name,
            vec![p("element", Element, true), p("to", ty, true)],
            Motion,
            false,
            Execution::Native(native::motion),
            native_types::fixed,
        );
    }
    for name in [
        "FadeIn",
        "FadeOut",
        "Create",
        "Uncreate",
        "Write",
        "Unwrite",
        "GrowFromCenter",
        "ShrinkToCenter",
    ] {
        add(
            name,
            vec![p("element", Element, true)],
            Motion,
            false,
            Execution::Native(native::motion),
            native_types::fixed,
        );
    }
    add(
        "play",
        vec![
            p("duration", duration.clone(), false),
            p("ease", String, false),
        ],
        Unit,
        true,
        Execution::Play,
        native_types::fixed,
    );
    add(
        "wait",
        vec![p("duration", duration, true)],
        Unit,
        true,
        Execution::Wait(native::wait),
        native_types::fixed,
    );
    add(
        "add",
        vec![p("element", Element, true)],
        Unit,
        true,
        Execution::Native(native::add),
        native_types::fixed,
    );
    add(
        "background",
        vec![p("color", Color, true)],
        Unit,
        true,
        Execution::Native(native::background),
        native_types::fixed,
    );
    add(
        "read",
        vec![p("path", String, true)],
        Unknown,
        false,
        Execution::Read,
        native_types::fixed,
    );
    add(
        "Data",
        vec![p("value", Unknown, true)],
        Data,
        false,
        Execution::Native(native::data),
        native_types::fixed,
    );
    add(
        "decode",
        vec![p("value", Unknown, true)],
        Unknown,
        false,
        Execution::Decode,
        native_types::fixed,
    );
    add(
        "query",
        vec![
            p("sql", String, true),
            p("tables", Unknown, true),
            p("params", Unknown, false),
        ],
        Unknown,
        false,
        Execution::Query,
        native_types::fixed,
    );
    add(
        "keys",
        vec![p("value", Unknown, true)],
        List(Box::new(String)),
        false,
        Execution::Native(native::keys),
        native_types::keys,
    );
    add(
        "len",
        vec![p("items", Unknown, true)],
        num.clone(),
        false,
        Execution::Native(native::len),
        native_types::len,
    );
    add(
        "range",
        vec![
            p("end", num.clone(), true),
            p("start", num.clone(), false),
            p("step", num.clone(), false),
        ],
        List(Box::new(num.clone())),
        false,
        Execution::Native(native::range),
        native_types::fixed,
    );
    add(
        "get",
        vec![p("items", Unknown, true), p("index", Unknown, true)],
        Unknown,
        false,
        Execution::Native(native::get),
        native_types::get,
    );
    add(
        "Some",
        vec![p("value", Unknown, true)],
        Unknown,
        false,
        Execution::Native(native::some),
        native_types::some,
    );
    add(
        "unwrap",
        vec![p("option", Unknown, true), p("default", Unknown, true)],
        Unknown,
        false,
        Execution::Native(native::unwrap),
        native_types::unwrap,
    );
    for name in ["min", "max"] {
        add(
            name,
            vec![p("a", Unknown, true), p("b", Unknown, true)],
            Unknown,
            false,
            Execution::Native(native::scalar),
            native_types::scalar,
        );
    }
    add(
        "abs",
        vec![p("value", Unknown, true)],
        Unknown,
        false,
        Execution::Native(native::scalar),
        native_types::scalar,
    );
    add(
        "clamp",
        vec![
            p("value", Unknown, true),
            p("low", Unknown, true),
            p("high", Unknown, true),
        ],
        Unknown,
        false,
        Execution::Native(native::scalar),
        native_types::scalar,
    );
    add(
        "str",
        vec![p("value", Unknown, true)],
        String,
        false,
        Execution::Native(native::str),
        native_types::fixed,
    );
    add(
        "assert",
        vec![p("condition", Unknown, true), p("message", String, false)],
        Unit,
        true,
        Execution::Assertion,
        native_types::assertion,
    );
    add(
        "check",
        vec![p("condition", Unknown, true), p("message", String, false)],
        Unit,
        true,
        Execution::Assertion,
        native_types::assertion,
    );
    for name in [
        "above", "below", "left_of", "right_of", "overlaps", "disjoint", "contains", "touches",
    ] {
        add(
            name,
            vec![
                p("a", Element, true),
                p("b", Element, true),
                p("gap", length.clone(), false),
            ],
            Type::Predicate,
            false,
            Execution::Native(native::relationship),
            native_types::fixed,
        );
    }
    add(
        "always",
        vec![p("condition", Type::Predicate, true)],
        Type::Predicate,
        false,
        Execution::Native(native::always),
        native_types::fixed,
    );
    add(
        "inside_canvas",
        vec![p("element", Element, true)],
        Type::Predicate,
        false,
        Execution::Native(native::relationship),
        native_types::fixed,
    );
    for function in sanim_core::math::functions::functions() {
        add(
            function.name,
            function
                .parameters
                .iter()
                .map(|(name, required)| p(name, Unknown, *required))
                .collect(),
            num.clone(),
            false,
            Execution::Native(native::math_function),
            native_types::math,
        );
    }
    out
}

pub fn builtin(name: &str) -> Option<&'static Builtin> {
    builtins().iter().find(|v| v.name == name)
}

#[derive(Clone)]
pub enum TypeDefinition {
    Plain(Type),
    Container(fn(Box<Type>) -> Type),
    Vector(usize),
    Bound(sanim_core::TypeBound),
}
impl TypeDefinition {
    pub fn instantiate(&self, args: Vec<Type>) -> Result<Type, String> {
        match (self, args.as_slice()) {
            (Self::Plain(ty), []) => Ok(ty.clone()),
            (Self::Container(build), [inner]) => Ok(build(Box::new(inner.clone()))),
            (Self::Vector(n), []) => Ok(Type::Vector(*n, Some(Dimension::Number))),
            (Self::Vector(n), [Type::Numeric(unit)]) => Ok(Type::Vector(*n, Some(*unit))),
            (Self::Plain(_), _) => Err("takes no type arguments".into()),
            (Self::Container(_), _) => Err("requires exactly one type argument".into()),
            (Self::Bound(_), _) => Err("is a generic constraint, not a value type".into()),
            (Self::Vector(_), _) => Err("expects zero arguments or one numeric unit type".into()),
        }
    }
}
pub fn types() -> &'static std::collections::BTreeMap<&'static str, TypeDefinition> {
    static TYPES: std::sync::OnceLock<std::collections::BTreeMap<&'static str, TypeDefinition>> =
        std::sync::OnceLock::new();
    TYPES.get_or_init(|| {
        use TypeDefinition::*;
        let mut out = std::collections::BTreeMap::new();
        for (name, ty) in [
            ("Number", Type::Numeric(Dimension::Number)),
            ("Length", Type::Numeric(Dimension::Length)),
            ("Duration", Type::Numeric(Dimension::Duration)),
            ("Angle", Type::Numeric(Dimension::Angle)),
            ("Rate", Type::Numeric(Dimension::Rate)),
            ("Percent", Type::Numeric(Dimension::Percent)),
            ("Bool", Type::Bool),
            ("String", Type::String),
            ("Color", Type::Color),
            ("Align", sanim_core::Align::ty()),
            ("Justify", sanim_core::Justify::ty()),
            ("Fraction", Type::Fraction),
            ("Matrix4", Type::Matrix4),
            ("Element", Type::Element),
            ("Motion", Type::Motion),
            ("Font", Type::Font),
            ("Data", Type::Data),
            ("Dict", Type::Dict),
            ("Unit", Type::Unit),
            ("Predicate", Type::Predicate),
        ] {
            out.insert(name, Plain(ty));
        }
        for builtin in builtins().iter().filter(|b| b.result == Type::Element) {
            out.insert(builtin.name, Plain(Type::Element));
        }
        for builtin in builtins() {
            if let Type::Vector(n, _) = builtin.result {
                out.insert(builtin.name, Vector(n));
            }
        }
        out.insert("List", Container(Type::List));
        out.insert("Table", Container(Type::Table));
        out.insert("Array", Container(Type::Array));
        out.insert("Option", Container(Type::Option));
        for (name, bound) in [
            ("Numeric", sanim_core::TypeBound::Numeric),
            ("Readable", sanim_core::TypeBound::Readable),
            ("Reflectable", sanim_core::TypeBound::Reflectable),
        ] {
            out.insert(name, Bound(bound));
        }
        out
    })
}
pub fn values() -> &'static std::collections::BTreeMap<&'static str, sanim_core::Value> {
    static VALUES: std::sync::OnceLock<
        std::collections::BTreeMap<&'static str, sanim_core::Value>,
    > = std::sync::OnceLock::new();
    VALUES.get_or_init(|| {
        let mut out = std::collections::BTreeMap::new();
        for (name, definition) in types() {
            if let TypeDefinition::Plain(ty @ Type::Enum { .. }) = definition {
                out.insert(*name, sanim_core::enum_namespace(ty));
            }
        }
        for (name, value) in sanim_core::math::functions::CONSTANTS {
            out.insert(
                *name,
                sanim_core::Value::Number(sanim_core::Number {
                    value: sanim_core::math::from_f64(*value)
                        .expect("native math constant must be representable"),
                    unit: Dimension::Number,
                }),
            );
        }
        out
    })
}
pub fn protected(name: &str) -> bool {
    types().contains_key(name)
        || builtin(name).is_some()
        || values().contains_key(name)
        || crate::parser::Keyword::from_name(name).is_some()
}
