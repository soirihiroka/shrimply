use crate::Limits;
use sanim_core::{
    verify::{Predicate, Relation},
    *,
};
use std::sync::Arc;

pub(crate) struct Context<'a> {
    pub module: &'a str,
    pub identity: &'a [String],
    pub span: Span,
    pub animation: &'a mut Animation,
    pub cursor: &'a mut Fraction,
    pub limits: &'a Limits,
    /// Remaining shared runtime budget for newly retained native metadata.
    pub retained_bytes: &'a mut usize,
    pub duration: Fraction,
    pub easing: Easing,
    pub list_edit: &'a mut Option<crate::lists::EditDelta>,
    /// Native algorithms reserve bounded work before entering their expensive loops.
    pub native_work: &'a mut u64,
    pub remaining_work: u64,
    /// Fresh visual children created by timed native operations, for source provenance.
    pub created: &'a mut Vec<Value>,
}

impl Context<'_> {
    pub(crate) fn charge_work(&mut self, amount: usize) -> Result<(), String> {
        let amount = u64::try_from(amount).map_err(|_| "native operation work count overflow")?;
        *self.native_work = self
            .native_work
            .checked_add(amount)
            .ok_or("native operation work count overflow")?;
        if *self.native_work > self.remaining_work {
            return Err(format!(
                "native computation requires {} work units, but only {} remain in the preparation budget; reduce samples/resolution or increase the host operation limit",
                self.native_work, self.remaining_work
            ));
        }
        Ok(())
    }
}

pub(crate) fn configuration(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    sanim_core::code::values::Native::from_name(name)
        .expect("registered native value constructor")
        .execute(args)
}

pub(crate) fn fraction(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
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
    args: &imbl::OrdMap<String, Value>,
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
    args: &imbl::OrdMap<String, Value>,
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
    args: &imbl::OrdMap<String, Value>,
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
    args: &imbl::OrdMap<String, Value>,
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
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let size = match required("items")? {
        Value::Dict(items) => items.len(),
        Value::Data(items) if items.is_object() => items.as_object().expect("object").len(),
        Value::Data(items) if items.is_array() => items.as_array().expect("array").len(),
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
    args: &imbl::OrdMap<String, Value>,
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
    Ok(Value::List(List::typed(
        Type::Numeric(Unit::Number),
        values,
    )))
}

pub(crate) fn get(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
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
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    Ok(Value::Data(sanim_data::to_json(required("value")?)?.into()))
}

pub(crate) fn keys(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let keys = match required("value")? {
        Value::Dict(values) => values.keys().cloned().collect::<Vec<_>>(),
        Value::Data(values) if values.is_object() => values
            .as_object()
            .expect("object")
            .keys()
            .cloned()
            .collect(),
        _ => return Err("keys requires Dict or object Data".into()),
    };
    Ok(Value::List(List::typed(
        Type::String,
        keys.into_iter()
            .map(|key| Value::String(key.into()))
            .collect(),
    )))
}

pub(crate) fn some(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    Ok(Value::Some(std::sync::Arc::new(required("value")?.clone())))
}

pub(crate) fn unwrap(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    match required("option")? {
        Value::Some(value) => Ok(value.as_ref().clone()),
        Value::None => Ok(required("default")?.clone()),
        _ => Err("expected Option".into()),
    }
}

pub(crate) fn str(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let format = args
        .get("format")
        .map(Value::string)
        .transpose()?
        .unwrap_or("");
    let conversion = args.get("conversion").map(Value::string).transpose()?;
    math::format::value(required("value")?, format, conversion)
        .map(|text| Value::String(text.into()))
}

pub(crate) fn unit(
    _name: &str,
    _args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    Ok(Value::Unit)
}

pub(crate) fn bg(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    let Some(Value::Color(color)) = args.get("color") else {
        return Err("bg requires a Color".into());
    };
    context.animation.background = color.clone();
    Ok(Value::Unit)
}

pub(crate) fn add(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
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
    animation.add(element.as_ref().clone(), *cursor)?;
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
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let Value::Predicate(mut predicate) = required("condition")?.clone() else {
        return Err("always requires a visual relationship".into());
    };
    std::sync::Arc::make_mut(&mut predicate).continuous = true;
    Ok(Value::Predicate(predicate))
}

pub(crate) fn math_function(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
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

pub(crate) fn relationship(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
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
        Some(right.as_ref().clone())
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
    Ok(Value::Predicate(std::sync::Arc::new(Predicate {
        left: left.as_ref().clone(),
        right,
        relation,
        gap,
        continuous: false,
    })))
}

pub(crate) fn element(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    make_element(name, args, context.module, context.identity, context.span)
}

pub(crate) fn make_element(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    module: &str,
    identity: &[String],
    span: Span,
) -> Result<Value, String> {
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let radius = || -> Result<CornerRadius, String> {
        match args.get("radius") {
            Some(Value::String(value)) if value.as_ref() == "full" => Ok(CornerRadius::Pill),
            Some(Value::String(_)) => Err("radius accepts a Length or \"full\"".into()),
            _ => {
                let radius = number(args, "radius", Fraction::from(0))?;
                if !math::finite(&radius) || radius < Fraction::from(0) {
                    return Err("radius must be finite and nonnegative".into());
                }
                Ok(radius.into())
            }
        }
    };
    let geometry = match name {
        "Circle" => Geometry::Circle {
            radius: positive(args, "radius", Fraction::from(32))?,
        },
        "Rectangle" | "Rect" => Geometry::Rectangle {
            width: positive(args, "width", Fraction::from(64))?,
            height: positive(args, "height", Fraction::from(64))?,
            radius: radius()?,
            children: args
                .get("children")
                .map(|value| {
                    value
                        .list()?
                        .iter()
                        .map(|value| {
                            if let Value::Element(element) = value {
                                Ok(element.as_ref().clone())
                            } else {
                                Err("Rect children must be Elements".to_string())
                            }
                        })
                        .collect::<Result<imbl::Vector<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default(),
        },
        "Box" => Geometry::Box {
            children: args
                .get("children")
                .map(|value| {
                    value
                        .list()?
                        .iter()
                        .map(|value| {
                            let Value::Element(child) = value else {
                                return Err("Box children must be Elements".to_string());
                            };
                            Ok(child.as_ref().clone())
                        })
                        .collect::<Result<imbl::Vector<_>, String>>()
                })
                .transpose()?
                .unwrap_or_default(),
            radius: radius()?,
        },
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
        "Polygon" | "Polyline" => {
            let points = required("points")?
                .list()?
                .iter()
                .map(Value::vec2)
                .collect::<Result<Vec<_>, _>>()?;
            if name == "Polyline" && points.len() > math::chart::plot::MAX_POINTS {
                return Err(format!(
                    "Polyline supports at most {} points",
                    math::chart::plot::MAX_POINTS
                ));
            }
            let minimum = if name == "Polyline" { 2 } else { 3 };
            if points.len() < minimum {
                return Err(format!("{name} requires at least {minimum} points"));
            }
            if points.iter().any(|p| p.unit != sanim_core::Unit::Length) {
                return Err(format!("{name} vertices must be local pt offsets"));
            }
            if points
                .iter()
                .flat_map(|point| point.values)
                .any(|value| !value.is_finite())
            {
                return Err(format!("{name} coordinates must be finite"));
            }
            if name == "Polyline" {
                Geometry::Polyline {
                    points: points.into(),
                    width: positive(args, "width", Fraction::from(1))?,
                }
            } else {
                Geometry::Polygon {
                    points: points.into(),
                }
            }
        }
        "Text" | "Label" => {
            let Value::Font(font) = required("font")? else {
                return Err(
                    "Text requires Font(\"Family\", \"sans-serif\") or read<Font>(...)".into(),
                );
            };
            let text = required("text")?.string()?;
            let size = positive(args, "font_size", Fraction::from(24))?;
            let shaped = sanim_data::font::cached_shape(font, text, size, false, false)?;
            Geometry::Text {
                text: text.into(),
                font: font.clone(),
                size,
                outline: shaped.outline.clone(),
                bounds: sanim_data::font::bounds(&shaped),
                pending: false,
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
    let group = matches!(geometry, Geometry::Box { .. });
    let position = args
        .get("position")
        .map(Value::vec2)
        .transpose()?
        .unwrap_or(Vec2::zero(Unit::Length));
    if !matches!(position.unit, Unit::Length | Unit::Percent) {
        return Err("position Vec2 components require pt or % units".into());
    }
    let fill = match args.get("fill") {
        Some(value) => sanim_core::Paint::from_value(value)?,
        None => Color::parse(if name == "Box" {
            "transparent"
        } else {
            "light1"
        })?
        .into(),
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
    let mut layout = Layout {
        x_align: alignment("x_align")?,
        y_align: alignment("y_align")?,
        position_offset: [0.; 2],
        direction: match args.get("flex_direction").map(Value::string).transpose()? {
            Some("row") => Some(Direction::Row),
            Some("column") => Some(Direction::Column),
            Some(other) => {
                return Err(format!(
                    "unknown flex_direction {other:?}; use row or column"
                ));
            }
            None => (name == "Box").then_some(Direction::Column),
        },
        flex: args.contains_key("flex_direction"),
        flex_grow: number(args, "flex_grow", Fraction::from(0))?,
        flex_shrink: number(args, "flex_shrink", Fraction::from(1))?,
        flex_basis: args.get("flex_basis").map(Value::number).transpose()?,
        gap: number(args, "gap", Fraction::from(0))?,
        padding: number(args, "padding", Fraction::from(0))?,
        justify,
        align,
        wrap: args
            .get("wrap")
            .map(Value::boolean)
            .transpose()?
            .unwrap_or(false),
        width: if name == "Line" {
            None
        } else {
            args.get("width").map(Value::number).transpose()?
        },
        height: args.get("height").map(Value::number).transpose()?,
        overflow_x: Overflow::Visible,
        overflow_y: Overflow::Visible,
        scroll: None,
        scroll_range: [0.; 2],
    };
    let overflow = |name| {
        args.get(name)
            .map(Value::string)
            .transpose()?
            .map(Overflow::parse)
            .transpose()
    };
    let default_overflow = overflow("overflow")?.unwrap_or_default();
    let overflow_x = overflow("overflow_x")?.unwrap_or(default_overflow);
    let overflow_y = overflow("overflow_y")?.unwrap_or(default_overflow);
    if (overflow_x.clips() || overflow_y.clips()) && geometry.children().is_none() {
        return Err(format!(
            "{name} has no scrollable children; put it inside a container with overflow"
        ));
    }
    let scroll = args.get("scroll").map(Value::vec2).transpose()?;
    layout.configure_overflow(overflow_x, overflow_y, scroll)?;
    if layout.flex_grow < Fraction::from(0)
        || layout.flex_shrink < Fraction::from(0)
        || !math::finite(&layout.flex_grow)
        || !math::finite(&layout.flex_shrink)
    {
        return Err("flex_grow and flex_shrink must be finite and nonnegative".into());
    }
    if layout.width.into_iter().chain(layout.height).any(|v| {
        !matches!(v.unit, Unit::Length | Unit::Percent)
            || !math::finite(&v.value)
            || v.value < Fraction::from(0)
            || v.value == Fraction::from(0) && name != "Box"
    }) {
        return Err("layout width and height must be positive".into());
    }
    if layout.gap < Fraction::from(0) || layout.padding < Fraction::from(0) {
        return Err("layout gap and padding must be nonnegative".into());
    }
    let border_width = number(
        args,
        "border_width",
        if args.contains_key("border_color") {
            Fraction::from(1)
        } else {
            Fraction::from(0)
        },
    )?;
    if !math::finite(&border_width) || border_width < Fraction::from(0) {
        return Err("border_width must be finite and nonnegative".into());
    }
    let stroke = (border_width > Fraction::from(0))
        .then(|| -> Result<sanim_core::paint::Stroke, String> {
            Ok(sanim_core::paint::Stroke {
                paint: match args.get("border_color") {
                    Some(Value::Color(color)) => color.clone().into(),
                    None => Color::parse("light5")?.into(),
                    _ => return Err("border_color requires Color".into()),
                },
                width: border_width,
            })
        })
        .transpose()?;
    Ok(Value::Element(std::sync::Arc::new(Element {
        id,
        geometry: geometry.into(),
        position,
        rotation: number(args, "rotation", Fraction::from(0))?,
        scale: positive(args, "scale", Fraction::from(1))?,
        opacity,
        fill,
        stroke,
        reveal: Reveal::default(),
        horizontal: HorizontalTransform::default(),
        layout,
    })))
}

/// Native composites share the same Box representation as authored containers.
/// Generated chart coordinates are normalized once into an intrinsic, non-flow box.
pub(crate) fn composition(
    args: &imbl::OrdMap<String, Value>,
    module: &str,
    identity: &[String],
    span: Span,
) -> Result<Value, String> {
    let Value::Element(mut root) = make_element("Box", args, module, identity, span)? else {
        unreachable!()
    };
    let element = Arc::make_mut(&mut root);
    element.layout.direction = None;
    element.layout.flex = false;
    element.fill = Color::parse("transparent")?.into();
    if !args.contains_key("x_align") {
        element.layout.x_align = Align::Center;
    }
    if !args.contains_key("y_align") {
        element.layout.y_align = Align::Center;
    }
    let Geometry::Box { children, .. } = Arc::make_mut(&mut element.geometry) else {
        unreachable!()
    };
    if !children.is_empty() {
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        let mut bounds = Vec::with_capacity(children.len());
        for child in children.iter_mut() {
            sanim_core::layout::resolve(child, 0., 0.)?;
            let b = sanim_core::verify::bounds(child)?;
            for axis in 0..2 {
                low[axis] = low[axis].min(b.min.values[axis]);
                high[axis] = high[axis].max(b.max.values[axis]);
            }
            bounds.push(b);
        }
        for (child, bounds) in children.iter_mut().zip(bounds) {
            child.position = Vec2 {
                values: [bounds.min.values[0] - low[0], bounds.min.values[1] - low[1]],
                unit: Unit::Length,
            };
            child.layout.x_align = Align::Start;
            child.layout.y_align = Align::Start;
        }
        for axis in 0..2 {
            if !args.contains_key(if axis == 0 { "x_align" } else { "y_align" }) {
                element.layout.position_offset[axis] =
                    math::float_arithmetic("+", low[axis] / 2., high[axis] / 2.)?;
            }
        }
        for (axis, field) in [&mut element.layout.width, &mut element.layout.height]
            .into_iter()
            .enumerate()
        {
            if field.is_none() {
                *field = Some(Number {
                    value: math::from_f64(high[axis] - low[axis])?,
                    unit: Unit::Length,
                });
            }
        }
    }
    Ok(Value::Element(root))
}

pub(crate) fn number(
    args: &imbl::OrdMap<String, Value>,
    name: &str,
    default: Fraction,
) -> Result<Fraction, String> {
    args.get(name)
        .map(|value| {
            value
                .number()
                .map_err(|error| format!("{name}: {error}; found {}", value.ty()))
        })
        .transpose()
        .map(|n| n.map_or(default, |v| v.value))
}
fn positive(
    args: &imbl::OrdMap<String, Value>,
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
