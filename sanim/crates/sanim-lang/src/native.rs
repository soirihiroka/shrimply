use crate::Limits;
use sanim_core::{
    verify::{Predicate, Relation},
    *,
};
use std::collections::BTreeMap;

pub(crate) struct Context<'a> {
    pub module: &'a str,
    pub identity: &'a [String],
    pub span: Span,
    pub animation: &'a mut Animation,
    pub cursor: &'a mut Fraction,
    pub limits: &'a Limits,
}

pub(crate) fn color(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    Ok(Value::Color(Color::parse(
        args.get("value")
            .map(Value::string)
            .transpose()?
            .unwrap_or("red3"),
    )?))
}

pub(crate) fn fraction(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let numerator = number(args, "numerator", Fraction::from(0))?;
    let denominator = number(args, "denominator", Fraction::from(1))?;
    if numerator.fract() != Fraction::from(0) || denominator.fract() != Fraction::from(0) {
        return Err("Fraction numerator and denominator must be integers".into());
    }
    Ok(Value::Fraction(math::arithmetic(
        "/",
        numerator,
        denominator,
    )?))
}

pub(crate) fn vector(
    name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let n = match name {
        "Vec2" => 2,
        "Vec3" => 3,
        _ => 4,
    };
    let names = ["x", "y", "z", "w"];
    let unit = names[..n]
        .iter()
        .find_map(|name| args.get(*name))
        .map(Value::number)
        .transpose()?
        .map_or(Unit::Number, |n| n.unit);
    let values = names[..n]
        .iter()
        .map(|name| number(args, name, Fraction::from(0)).and_then(math::to_f64))
        .collect::<Result<Vec<_>, _>>()?;
    Value::from_vector(values, unit)
}

pub(crate) fn matrix(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let Some(rows) = args.get("rows") else {
        return Ok(Value::Matrix4(Box::new(math::linear::identity())));
    };
    let rows = rows.list()?;
    if rows.len() != 4 {
        return Err("Matrix4 requires exactly four rows of four Number values".into());
    }
    let mut matrix = [[0.0; 4]; 4];
    for (i, row) in rows.iter().enumerate() {
        let row = row.list()?;
        if row.len() != 4 {
            return Err(format!("Matrix4 row {i} requires four Number values"));
        }
        for (j, v) in row.iter().enumerate() {
            matrix[i][j] = math::to_f64(v.number()?.value)?;
        }
    }
    Ok(Value::Matrix4(Box::new(matrix)))
}

pub(crate) fn dot(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let (a, _) = required("a")?.vector()?;
    let (b, _) = required("b")?.vector()?;
    Ok(Value::Number(Number {
        value: math::from_f64(math::linear::dot(a, b)?)?,
        unit: Unit::Number,
    }))
}

pub(crate) fn matrix_operation(
    name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let Value::Matrix4(matrix) = required("matrix")? else {
        return Err("expected Matrix4".into());
    };
    Ok(Value::Matrix4(Box::new(if name == "inverse" {
        math::linear::inverse(matrix)?
    } else {
        math::linear::transpose(matrix)
    })))
}

pub(crate) fn len(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let size = match required("items")? {
        Value::Dict(items) => items.len(),
        Value::Data(serde_json::Value::Object(items)) => items.len(),
        Value::Data(serde_json::Value::Array(items)) => items.len(),
        Value::Data(_) => return Err("len requires object or array Data".into()),
        value => value.list()?.len(),
    };
    Ok(Value::Number(Number {
        value: Fraction::from(size as u64),
        unit: Unit::Number,
    }))
}

pub(crate) fn range(
    _name: &str,
    args: &BTreeMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    let Context { limits, .. } = context;
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let end = required("end")?.number()?.usize()?;
    let start = args
        .get("start")
        .map(Value::number)
        .transpose()?
        .map(Number::usize)
        .transpose()?
        .unwrap_or(0);
    let step = args
        .get("step")
        .map(Value::number)
        .transpose()?
        .map(Number::usize)
        .transpose()?
        .unwrap_or(1);
    if step == 0 {
        return Err("range step must be positive".into());
    }
    let count = end.saturating_sub(start).div_ceil(step);
    if count > limits.max_iterations {
        return Err(format!(
            "range would create {count} items; iteration limit is {}",
            limits.max_iterations
        ));
    }
    let values = (start..end)
        .step_by(step)
        .map(|v| {
            Value::Number(Number {
                value: Fraction::from(v as u64),
                unit: Unit::Number,
            })
        })
        .collect();
    Ok(Value::List(values))
}

pub(crate) fn get(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    required("items")?.lookup(required("index")?)
}

pub(crate) fn data(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    Ok(Value::Data(sanim_data::to_json(required("value")?)?))
}

pub(crate) fn keys(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let keys = match required("value")? {
        Value::Dict(values) => values.keys().cloned().collect::<Vec<_>>(),
        Value::Data(serde_json::Value::Object(values)) => values.keys().cloned().collect(),
        _ => return Err("keys requires Dict or object Data".into()),
    };
    Ok(Value::List(keys.into_iter().map(Value::String).collect()))
}

pub(crate) fn some(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    Ok(Value::Some(Box::new(required("value")?.clone())))
}

pub(crate) fn unwrap(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    match required("option")? {
        Value::Some(value) => Ok(*value.clone()),
        Value::None => Ok(required("default")?.clone()),
        _ => Err("expected Option".into()),
    }
}

pub(crate) fn scalar(
    name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let first = if matches!(name, "min" | "max") {
        required("a")?
    } else {
        required("value")?
    }
    .number()?;
    let value = match name {
        "abs" => {
            if first.value < Fraction::from(0) {
                -first.value
            } else {
                first.value
            }
        }
        "min" => first.value.min(required("b")?.number()?.value),
        "max" => first.value.max(required("b")?.number()?.value),
        _ => {
            let low = required("low")?.number()?.value;
            let high = required("high")?.number()?.value;
            if low > high {
                return Err("clamp low must not exceed high".into());
            }
            first.value.max(low).min(high)
        }
    };
    let first_value = args.get(if matches!(name, "min" | "max") {
        "a"
    } else {
        "value"
    });
    Ok(if matches!(first_value, Some(Value::Fraction(_))) {
        Value::Fraction(value)
    } else {
        Value::Number(Number {
            value,
            unit: first.unit,
        })
    })
}

pub(crate) fn str(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    Ok(Value::String(match required("value")? {
        Value::String(v) => v.clone(),
        Value::Number(n) => format!("{}", n.value),
        Value::Bool(v) => v.to_string(),
        other => serde_json::to_string(other).map_err(|e| e.to_string())?,
    }))
}

pub(crate) fn wait(
    _name: &str,
    args: &BTreeMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    let Context {
        animation, cursor, ..
    } = context;
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let duration = required("duration")?.number()?;
    if duration.value < Fraction::from(0) {
        return Err("wait duration must be nonnegative".into());
    }
    *cursor = math::timeline_end(*cursor, duration.value)?;
    animation.duration = animation.duration.max(*cursor);
    Ok(Value::Unit)
}

pub(crate) fn background(
    _name: &str,
    args: &BTreeMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    let Some(Value::Color(color)) = args.get("color") else {
        return Err("background requires a Color".into());
    };
    context.animation.background = color.clone();
    Ok(Value::Unit)
}

pub(crate) fn add(
    _name: &str,
    args: &BTreeMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    let Context {
        animation,
        cursor,
        limits,
        ..
    } = context;
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let Value::Element(element) = required("element")? else {
        return Err("add requires Element".into());
    };
    animation.add(*element.clone(), *cursor)?;
    if animation.element_count() > limits.max_elements {
        return Err(format!(
            "element count exceeds limit {}",
            limits.max_elements
        ));
    }
    Ok(Value::Unit)
}

pub(crate) fn always(
    _name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let Value::Predicate(mut predicate) = required("condition")?.clone() else {
        return Err("always requires a visual relationship".into());
    };
    predicate.continuous = true;
    Ok(Value::Predicate(predicate))
}

pub(crate) fn math_function(
    name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let function = math::functions::function(name).expect("registered math function");
    let values = function
        .parameters
        .iter()
        .filter_map(|(name, _)| args.get(*name))
        .map(Value::number)
        .collect::<Result<Vec<_>, _>>()?;
    let result = math::functions::evaluate(name, &values)?;
    Ok(
        if function.domain == math::functions::Domain::Numeric
            && name != "sign"
            && matches!(args.get(function.parameters[0].0), Some(Value::Fraction(_)))
        {
            Value::Fraction(result.value)
        } else {
            Value::Number(result)
        },
    )
}

pub(crate) fn motion(
    name: &str,
    args: &BTreeMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    let Context {
        animation, cursor, ..
    } = context;
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let Value::Element(element) = required("element")? else {
        return Err("motion target must be Element".into());
    };
    let (property, to, from) = match name {
        "MoveTo" => (Property::Position, required("to")?.clone(), None),
        "Shift" => {
            let offset = required("to")?.vec2()?;
            let mut current = animation.sample_element(element, *cursor)?.position;
            if current.values.iter().all(|v| *v == 0.0) {
                current.unit = offset.unit;
            }
            if offset.unit != current.unit {
                return Err(
                    "Shift and current position must both use pt or both use percentages".into(),
                );
            }
            (
                Property::Position,
                Value::Vec2(Vec2 {
                    values: [
                        math::float_arithmetic("+", current.values[0], offset.values[0])?,
                        math::float_arithmetic("+", current.values[1], offset.values[1])?,
                    ],
                    unit: current.unit,
                }),
                None,
            )
        }
        "FadeIn" => (
            Property::Opacity,
            Value::Number(Number::scalar(1)),
            Some(Value::Number(Number::scalar(0))),
        ),
        "FadeOut" => (Property::Opacity, Value::Number(Number::scalar(0)), None),
        "Create" | "Write" => (
            Property::Reveal,
            Value::Number(Number::scalar(1)),
            Some(Value::Number(Number::scalar(0))),
        ),
        "Uncreate" | "Unwrite" => (Property::Reveal, Value::Number(Number::scalar(0)), None),
        "GrowFromCenter" | "ShrinkToCenter" => {
            if element.layout.x_align != Align::Center || element.layout.y_align != Align::Center {
                return Err(format!(
                    "{name} requires x_align: Align.Center and y_align: Align.Center; use ScaleTo for an edge-anchored element"
                ));
            }
            if name == "GrowFromCenter" {
                (
                    Property::Scale,
                    Value::Number(Number {
                        value: element.scale,
                        unit: Unit::Number,
                    }),
                    Some(Value::Number(Number::scalar(0))),
                )
            } else {
                (Property::Scale, Value::Number(Number::scalar(0)), None)
            }
        }
        "RotateTo" => (Property::Rotation, required("to")?.clone(), None),
        "ScaleTo" => {
            if required("to")?.number()?.value <= Fraction::from(0) {
                return Err("scale must be positive".into());
            }
            (Property::Scale, required("to")?.clone(), None)
        }
        _ => (Property::Fill, required("to")?.clone(), None),
    };
    Ok(Value::Motion(Box::new(Motion {
        element: *element.clone(),
        property,
        to,
        from,
        reveal: match name {
            "Create" | "Uncreate" => Some(RevealKind::Create),
            "Write" | "Unwrite" => Some(RevealKind::Write),
            _ => None,
        },
    })))
}

pub(crate) fn relationship(
    name: &str,
    args: &BTreeMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let left = if name == "inside_canvas" {
        required("element")?
    } else {
        required("a")?
    };
    let Value::Element(left) = left else {
        return Err("relationship requires Element".into());
    };
    let right = if name == "inside_canvas" {
        None
    } else {
        let Value::Element(right) = required("b")? else {
            return Err("relationship requires Element".into());
        };
        Some(*right.clone())
    };
    let relation = match name {
        "above" => Relation::Above,
        "below" => Relation::Below,
        "left_of" => Relation::LeftOf,
        "right_of" => Relation::RightOf,
        "overlaps" => Relation::Overlaps,
        "disjoint" => Relation::Disjoint,
        "contains" => Relation::Contains,
        "touches" => Relation::Touches,
        _ => Relation::InsideCanvas,
    };
    let gap = number(args, "gap", Fraction::from(0))?;
    if gap < Fraction::from(0) {
        return Err("relationship gap must be nonnegative".into());
    }
    Ok(Value::Predicate(Box::new(Predicate {
        left: *left.clone(),
        right,
        relation,
        gap,
        continuous: false,
    })))
}

pub(crate) fn element(
    name: &str,
    args: &BTreeMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    let Context {
        module,
        identity,
        span,
        ..
    } = context;
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let geometry = match name {
        "Circle" => Geometry::Circle {
            radius: positive(args, "radius", Fraction::from(32))?,
        },
        "Rectangle" | "Rect" => Geometry::Rectangle {
            width: positive(args, "width", Fraction::from(64))?,
            height: positive(args, "height", Fraction::from(64))?,
        },
        "Group" | "Row" | "Column" => {
            let children = required("children")?
                .list()?
                .iter()
                .map(|v| {
                    if let Value::Element(e) = v {
                        Ok(*e.clone())
                    } else {
                        Err("Group children must be Elements")
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Geometry::Group { children }
        }
        "Line" => {
            let end = required("end")?.vec2()?;
            if end.unit != sanim_core::Unit::Length {
                return Err("Line end must be a parent-local pt offset".into());
            }
            Geometry::Line {
                end,
                width: positive(args, "width", Fraction::from(1))?,
            }
        }
        "Polygon" => {
            let points = required("points")?
                .list()?
                .iter()
                .map(Value::vec2)
                .collect::<Result<Vec<_>, _>>()?;
            if points.len() < 3 {
                return Err("Polygon requires at least three points".into());
            }
            if points.iter().any(|p| p.unit != sanim_core::Unit::Length) {
                return Err("Polygon vertices must be local pt offsets".into());
            }
            Geometry::Polygon { points }
        }
        "Text" => {
            let Value::Font(font) = required("font")? else {
                return Err("Text requires read<Font>(...)".into());
            };
            let text = required("text")?.string()?;
            let size = positive(args, "font_size", Fraction::from(24))?;
            let (bounds, outline) = sanim_data::font::shape(font, text, size)?;
            Geometry::Text {
                text: text.into(),
                size,
                outline,
                bounds,
            }
        }
        _ => return Err(format!("native '{name}' has no implementation")),
    };
    let id = args
        .get("id")
        .map(Value::string)
        .transpose()?
        .map(str::to_string)
        .unwrap_or_else(|| format!("{module}:{}:{}", span.start, identity.join("/")));
    if id.is_empty() {
        return Err("element id must be nonempty".into());
    }
    let group = matches!(geometry, Geometry::Group { .. });
    let position = args
        .get("position")
        .map(Value::vec2)
        .transpose()?
        .unwrap_or(Vec2::zero(Unit::Length));
    if !matches!(position.unit, Unit::Length | Unit::Percent) {
        return Err("position Vec2 components require pt or % units".into());
    }
    let fill = match args.get("fill") {
        Some(Value::Color(color)) => color.clone(),
        None => Color::parse("light1")?,
        _ => return Err("fill must be Color".into()),
    };
    let opacity = number(args, "opacity", Fraction::from(1))?;
    if !(Fraction::from(0)..=Fraction::from(1)).contains(&opacity) {
        return Err("opacity must be in [0, 1]".into());
    }
    let alignment = |name: &str| -> Result<Align, String> {
        let Some(value) = args.get(name) else {
            return Ok(if group { Align::Start } else { Align::Center });
        };
        let Value::Variant { tag, .. } = value else {
            return Err("alignment requires Align.Start, Align.Center or Align.End".into());
        };
        Align::from_name(tag)
    };
    let justify = match args.get("justify") {
        Some(Value::Variant { tag, .. }) => Justify::from_name(tag)?,
        None => Justify::Start,
        _ => return Err("justify requires a Justify enum case".into()),
    };
    let align = match args.get("align") {
        Some(Value::Variant { tag, .. }) => Align::from_name(tag)?,
        None => Align::Start,
        _ => return Err("align requires an Align enum case".into()),
    };
    let layout = Layout {
        x_align: alignment("x_align")?,
        y_align: alignment("y_align")?,
        direction: match name {
            "Row" => Some(Direction::Row),
            "Column" => Some(Direction::Column),
            _ => None,
        },
        gap: number(args, "gap", Fraction::from(0))?,
        padding: number(args, "padding", Fraction::from(0))?,
        justify,
        align,
        wrap: args
            .get("wrap")
            .map(Value::boolean)
            .transpose()?
            .unwrap_or(name == "Row"),
        width: args
            .get("width")
            .map(Value::number)
            .transpose()?
            .map(|n| n.value),
        height: args
            .get("height")
            .map(Value::number)
            .transpose()?
            .map(|n| n.value),
    };
    if layout
        .width
        .into_iter()
        .chain(layout.height)
        .any(|v| v <= Fraction::from(0))
    {
        return Err("layout width and height must be positive".into());
    }
    if layout.gap < Fraction::from(0) || layout.padding < Fraction::from(0) {
        return Err("layout gap and padding must be nonnegative".into());
    }
    Ok(Value::Element(Box::new(Element {
        id,
        geometry,
        position,
        rotation: number(args, "rotation", Fraction::from(0))?,
        scale: positive(args, "scale", Fraction::from(1))?,
        opacity,
        fill,
        reveal: Reveal::default(),
        layout,
    })))
}

fn number(
    args: &BTreeMap<String, Value>,
    name: &str,
    default: Fraction,
) -> Result<Fraction, String> {
    args.get(name)
        .map(Value::number)
        .transpose()
        .map(|n| n.map_or(default, |v| v.value))
}
fn positive(
    args: &BTreeMap<String, Value>,
    name: &str,
    default: Fraction,
) -> Result<Fraction, String> {
    let value = number(args, name, default)?;
    if value <= Fraction::from(0) {
        Err(format!("{name} must be positive"))
    } else {
        Ok(value)
    }
}
