use crate::Limits;
use sanim_core::{
    verify::{Predicate, Relation},
    *,
};

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
    mut context: Context<'_>,
) -> Result<Value, String> {
    if let Some(design) = args.get("design") {
        context.charge_work(design.string()?.len())?;
    }
    make_element(name, args, context.module, context.identity, context.span)
}

pub(crate) fn make_element(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    module: &str,
    identity: &[String],
    span: Span,
) -> Result<Value, String> {
    if args.contains_key("design") {
        let design = crate::design::resolve(name, args)?;
        let value = make_element_raw(name, &design.args, module, identity, span)?;
        let Value::Element(element) = value else {
            unreachable!("native element")
        };
        let mut element = std::sync::Arc::unwrap_or_clone(element);
        if let Some(direction) = design.direction {
            if element.geometry.children().is_none() {
                return Err(format!(
                    "{name} does not support child layout; use Rect, Row or Column"
                ));
            }
            element.layout.direction = Some(direction);
        }
        if design.pill {
            let Geometry::Rectangle { radius, .. } =
                std::sync::Arc::make_mut(&mut element.geometry)
            else {
                return Err("pill/rounded-full requires Rect or Rectangle".into());
            };
            *radius = CornerRadius::Pill;
        }
        if let Some(border) = design.stroke {
            element.stroke = Some(sanim_core::paint::Stroke {
                paint: border.color.into(),
                width: border.width,
            });
        }
        return Ok(Value::Element(element.into()));
    }
    make_element_raw(name, args, module, identity, span)
}
fn make_element_raw(
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
    let geometry = match name {
        "Circle" => Geometry::Circle {
            radius: positive(args, "radius", Fraction::from(32))?,
        },
        "Rectangle" | "Rect" => {
            let radius = number(args, "radius", Fraction::from(0))?;
            if !math::finite(&radius) || radius < Fraction::from(0) {
                return Err("Rect radius must be finite and nonnegative".into());
            }
            Geometry::Rectangle {
                width: positive(args, "width", Fraction::from(64))?,
                height: positive(args, "height", Fraction::from(64))?,
                radius: radius.into(),
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
            }
        }
        "Group" | "Row" | "Column" | "ScrollView" => {
            let children = required("children")?
                .list()?
                .iter()
                .map(|v| {
                    if let Value::Element(e) = v {
                        Ok(e.as_ref().clone())
                    } else {
                        Err("Group children must be Elements")
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Geometry::Group {
                children: children.into(),
            }
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
                bounds: shaped.bounds,
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
        Some(value) => sanim_core::Paint::from_value(value)?,
        None => Color::parse("light1")?.into(),
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
        width: args.get("width").map(Value::number).transpose()?,
        height: args.get("height").map(Value::number).transpose()?,
        scroll: args
            .get("scroll")
            .map(Value::vec2)
            .transpose()?
            .or_else(|| (name == "ScrollView").then(|| Vec2::zero(Unit::Length))),
    };
    if name == "ScrollView" && (layout.width.is_none() || layout.height.is_none()) {
        return Err("ScrollView requires width and height for its viewport".into());
    }
    if layout.width.into_iter().chain(layout.height).any(|v| {
        !matches!(v.unit, Unit::Length | Unit::Percent)
            || !math::finite(&v.value)
            || v.value <= Fraction::from(0)
    }) {
        return Err("layout width and height must be positive".into());
    }
    if layout.gap < Fraction::from(0) || layout.padding < Fraction::from(0) {
        return Err("layout gap and padding must be nonnegative".into());
    }
    Ok(Value::Element(std::sync::Arc::new(Element {
        id,
        geometry: geometry.into(),
        position,
        rotation: number(args, "rotation", Fraction::from(0))?,
        scale: positive(args, "scale", Fraction::from(1))?,
        opacity,
        fill,
        stroke: None,
        reveal: Reveal::default(),
        horizontal: HorizontalTransform::default(),
        layout,
    })))
}

pub(crate) fn number(
    args: &imbl::OrdMap<String, Value>,
    name: &str,
    default: Fraction,
) -> Result<Fraction, String> {
    args.get(name)
        .map(Value::number)
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
