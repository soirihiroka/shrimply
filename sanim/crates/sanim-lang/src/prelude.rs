pub use crate::timing::Timing;
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
    &imbl::OrdMap<String, sanim_core::Value>,
    native::Context<'a>,
) -> Result<sanim_core::Value, String>;
pub(crate) type TypeRule = fn(&Builtin, &[Type]) -> Result<Type, String>;
#[derive(Clone, Copy, Debug)]
pub(crate) enum Execution {
    Native(Evaluator),
    /// Watched asset reads must execute even when their argument values are unchanged.
    Source(Evaluator),
    /// Cheap accessors must not serialize/hash their entire input just to cache a lookup.
    Access(Evaluator),
    Read,
    Decode,
    Query,
    Assertion,
}
#[derive(Clone, Copy, Debug, Default)]
pub enum MarkupContent {
    #[default]
    Standard,
    Literal {
        parameter: &'static str,
        dedent: bool,
    },
}
#[derive(Clone, Debug)]
pub struct Builtin {
    pub name: &'static str,
    pub parameters: Vec<Parameter>,
    pub result: Type,
    /// Scene/assertion effects remain impure even when they consume zero timeline time.
    pub schedule: bool,
    pub timing: Timing,
    pub content: MarkupContent,
    pub(crate) execute: Execution,
    pub(crate) check: TypeRule,
}
impl Builtin {
    pub(crate) fn uses_font_environment(&self) -> bool {
        self.result == Type::Font
            || self
                .parameters
                .iter()
                .any(|parameter| parameter.ty == Type::Font)
    }
}

pub fn builtins() -> &'static [Builtin] {
    static BUILTINS: std::sync::OnceLock<Vec<Builtin>> = std::sync::OnceLock::new();
    BUILTINS.get_or_init(build_builtins)
}

/// Shared element layout and visual inputs. Specialized arguments declared later
/// override a shared name (for example Line.width remains stroke thickness).
pub(crate) fn element_parameters() -> Vec<Parameter> {
    use Type::*;
    let p = |name, ty| Parameter {
        name,
        ty,
        required: false,
    };
    let overflow = sanim_core::Overflow::ty();
    vec![
        p("id", String),
        p("position", Vector(2, None)),
        p("x_align", sanim_core::Align::ty()),
        p("y_align", sanim_core::Align::ty()),
        p("fill", sanim_core::Paint::ty()),
        p("opacity", Numeric(Dimension::Number)),
        p("rotation", Numeric(Dimension::Angle)),
        p("scale", Numeric(Dimension::Number)),
        p("width", sanim_core::Layout::dimension_type()),
        p("height", sanim_core::Layout::dimension_type()),
        p("padding", Numeric(Dimension::Length)),
        p("gap", Numeric(Dimension::Length)),
        p("justify", sanim_core::Justify::ty()),
        p("align", sanim_core::Align::ty()),
        p("wrap", Bool),
        p("white_space", sanim_core::text::WhiteSpace::ty()),
        p("overflow_wrap", sanim_core::text::OverflowWrap::ty()),
        p("flex_direction", String),
        p("flex_grow", Numeric(Dimension::Number)),
        p("flex_shrink", Numeric(Dimension::Number)),
        p("flex_basis", sanim_core::Layout::dimension_type()),
        p("border_color", Color),
        p("border_width", Numeric(Dimension::Length)),
        p("overflow", overflow.clone()),
        p("overflow_x", overflow.clone()),
        p("overflow_y", overflow),
        p(
            "scroll",
            Type::union([
                Vector(2, Some(Dimension::Length)),
                Vector(2, Some(Dimension::Percent)),
            ])
            .expect("scroll units"),
        ),
    ]
}

fn build_builtins() -> Vec<Builtin> {
    use Type::*;
    let num = Numeric(Dimension::Number);
    let length = Numeric(Dimension::Length);
    let duration = Numeric(Dimension::Duration);
    let p = |name, ty, required| Parameter { name, ty, required };
    let mut out = vec![];
    crate::dataframe::register(&mut out);
    crate::templates::register(&mut out);
    let mut add = |name, parameters, result, schedule, execute, check| {
        out.push(Builtin {
            name,
            parameters,
            result,
            schedule,
            timing: Default::default(),
            content: Default::default(),
            execute,
            check,
        })
    };
    for operation in [
        sanim_core::code::values::Native::Color,
        sanim_core::code::values::Native::Font,
    ] {
        let (parameters, result) = operation.signature();
        add(
            operation.name(),
            parameters
                .into_iter()
                .map(|(name, ty, required)| p(name, ty, required))
                .collect(),
            result,
            false,
            Execution::Native(native::configuration),
            native_types::fixed,
        );
    }
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
    let common = element_parameters;
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
            p(
                "radius",
                Type::union([length.clone(), String]).expect("corner radius"),
                false,
            ),
            p("children", List(Box::new(Element)), false),
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
    let mut params = common();
    params.extend([
        p("children", List(Box::new(Element)), false),
        p(
            "radius",
            Type::union([length.clone(), String]).expect("corner radius"),
            false,
        ),
    ]);
    add(
        "Box",
        params,
        Element,
        false,
        Execution::Native(native::element),
        native_types::fixed,
    );
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
    let mut params = vec![p("text", String, true), p("font", Font, false)];
    params.extend(common());
    params.push(p("font_size", length.clone(), false));
    for name in ["Text", "Label"] {
        add(
            name,
            params.clone(),
            Element,
            false,
            Execution::Native(native::element),
            native_types::fixed,
        );
    }
    add(
        "wait",
        vec![p("duration", duration, true)],
        Unit,
        false,
        Execution::Native(native::unit),
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
        "bg",
        vec![p("color", Color, true)],
        Unit,
        true,
        Execution::Native(native::bg),
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
            p(
                "sql",
                Type::union([Template, Sql]).expect("distinct query inputs"),
                true,
            ),
            p("tables", Unknown, false),
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
        Execution::Access(native::keys),
        native_types::keys,
    );
    add(
        "len",
        vec![p("items", Unknown, true)],
        num.clone(),
        false,
        Execution::Access(native::len),
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
        Execution::Access(native::get),
        native_types::get,
    );
    add(
        "Some",
        vec![p("value", Unknown, true)],
        Unknown,
        false,
        Execution::Access(native::some),
        native_types::some,
    );
    add(
        "unwrap",
        vec![p("option", Unknown, true), p("default", Unknown, true)],
        Unknown,
        false,
        Execution::Access(native::unwrap),
        native_types::unwrap,
    );
    add(
        "str",
        vec![
            p("value", Unknown, true),
            p("format", String, false),
            p("conversion", String, false),
        ],
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
    out.extend(crate::charts::builtins(common()));
    crate::code_view::register(&mut out, common());
    crate::media::register(&mut out);
    out.extend(crate::utilities::builtins());
    crate::lists::register(&mut out);
    crate::animations::register(&mut out);
    for builtin in &mut out {
        finalize(builtin);
    }
    out
}

pub(crate) fn finalize(builtin: &mut Builtin) {
    let mut names = std::collections::BTreeSet::new();
    builtin.parameters.reverse();
    builtin
        .parameters
        .retain(|parameter| names.insert(parameter.name));
    builtin.parameters.reverse();

    if !builtin.parameters.iter().any(|p| p.name == "duration") {
        builtin.parameters.push(Parameter {
            name: "duration",
            ty: Type::Numeric(Dimension::Duration),
            required: false,
        });
    }
    if !builtin.parameters.iter().any(|p| p.name == "ease") {
        builtin.parameters.push(Parameter {
            name: "ease",
            ty: Type::String,
            required: false,
        });
    }
    builtin.timing.parameter = builtin
        .parameters
        .iter()
        .position(|p| p.name == "duration")
        .expect("registered duration parameter");
}

pub fn builtin(name: &str) -> Option<&'static Builtin> {
    builtins()
        .iter()
        .find(|v| v.name == name)
        .or_else(|| crate::dataframe::builtin(name))
        .or_else(|| crate::code_view::builtin(name))
        .or_else(|| crate::scrolling::builtin(name))
}

pub fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    crate::dataframe::methods_for(receiver)
        .chain(crate::code_view::methods_for(receiver))
        .chain(crate::scrolling::methods_for(receiver))
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
            ("Template", Type::Template),
            ("Sql", Type::Sql),
            ("SqlIdent", Type::SqlIdent),
            ("Color", Type::Color),
            ("Align", sanim_core::Align::ty()),
            ("Justify", sanim_core::Justify::ty()),
            ("Fraction", Type::Fraction),
            ("Matrix4", Type::Matrix4),
            ("Element", Type::Element),
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
        // Record-valued constructors also provide usable structural type names.
        // Keep these aliases derived from the constructor registry.
        for builtin in builtins().iter().filter(|builtin| {
            builtin.name.starts_with(char::is_uppercase)
                && matches!(builtin.result, Type::Record(_))
        }) {
            out.insert(builtin.name, Plain(builtin.result.clone()));
        }
        for builtin in builtins() {
            if let Type::Vector(n, _) = builtin.result {
                out.insert(builtin.name, Vector(n));
            }
        }
        out.insert("List", Container(Type::List));
        out.insert(
            "DataFrame",
            Container(|row| Type::DataFrame {
                row,
                groups: Vec::new(),
            }),
        );
        out.insert("Series", Container(Type::Series));
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
