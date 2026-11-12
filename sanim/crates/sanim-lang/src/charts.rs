//! Typed chart constructors lower to the same elements used by every other Sanim shape.
use crate::{
    native::{self, Context},
    native_types,
    prelude::{Builtin, Execution, Parameter},
};
use imbl::OrdMap;
use sanim_core::{
    math::{self, chart::LinearScale},
    *,
};
use std::collections::BTreeSet;

const SUMMARY_FIELDS: [&str; 5] = ["low", "q1", "middle", "q3", "high"];
fn record(fields: impl IntoIterator<Item = (&'static str, Type)>) -> Type {
    Type::Record(fields.into_iter().map(|(n, t)| (n.into(), t)).collect())
}
fn scale_type() -> Type {
    record([
        ("domain", Type::Vector(2, Some(Unit::Number))),
        ("range", Type::Vector(2, Some(Unit::Length))),
        ("clamp", Type::Bool),
    ])
}
fn summary_type() -> Type {
    record(SUMMARY_FIELDS.map(|n| (n, Type::Numeric(Unit::Number))))
}
pub(crate) fn builtins(common: Vec<Parameter>) -> Vec<Builtin> {
    let number = Type::Numeric(Unit::Number);
    let length = Type::Numeric(Unit::Length);
    let p = |name, ty, required| Parameter { name, ty, required };
    let list = |ty| Type::List(Box::new(ty));
    let mut out = vec![];
    let mut add = |name, parameters, result| {
        out.push(Builtin {
            name,
            parameters,
            result,
            schedule: false,
            execute: Execution::Native(evaluate),
            check: native_types::fixed,
        })
    };
    add(
        "LinearScale",
        vec![
            p("domain", Type::Vector(2, Some(Unit::Number)), true),
            p("range", Type::Vector(2, Some(Unit::Length)), true),
            p("clamp", Type::Bool, false),
        ],
        scale_type(),
    );
    add(
        "scale_value",
        vec![
            p("scale", scale_type(), true),
            p("value", number.clone(), true),
        ],
        length.clone(),
    );
    add(
        "BoxSummary",
        vec![p("values", list(number.clone()), true)],
        summary_type(),
    );
    for name in ["Axis", "Grid", "Scatter", "BoxPlot"] {
        let mut params: Vec<_> = common
            .iter()
            .filter(|p| p.name != "scale")
            .cloned()
            .collect();
        if name == "Scatter" {
            params.extend([
                p(
                    "data",
                    Type::Table(Box::new(record([
                        ("x", number.clone()),
                        ("y", number.clone()),
                    ]))),
                    true,
                ),
                p("x_scale", scale_type(), true),
                p("y_scale", scale_type(), true),
                p("radius", length.clone(), false),
                p("point_opacity", number.clone(), false),
                p("keys", list(Type::String), false),
            ]);
        } else {
            params.push(p("scale", scale_type(), true));
            if name == "BoxPlot" {
                params.extend([
                    p("summary", summary_type(), true),
                    p("height", length.clone(), false),
                    p("width", length.clone(), false),
                    p("median_color", Type::Color, false),
                ]);
            } else {
                params.extend([
                    p("ticks", list(number.clone()), false),
                    p("tick_count", number.clone(), false),
                    p("vertical", Type::Bool, false),
                    p("width", length.clone(), false),
                ]);
                if name == "Grid" {
                    params.extend([
                        p("spans", list(Type::Vector(2, Some(Unit::Length))), true),
                        p("highlight", list(number.clone()), false),
                        p("grid_opacity", number.clone(), false),
                        p("highlight_opacity", number.clone(), false),
                    ]);
                } else {
                    params.extend([
                        p("font", Type::Font, true),
                        p("show_labels", Type::Bool, false),
                        p("font_size", length.clone(), false),
                        p("labels", list(Type::String), false),
                        p("format", Type::String, false),
                        p("tick_length", length.clone(), false),
                        p("label_gap", length.clone(), false),
                        p("title", Type::String, false),
                    ]);
                }
            }
        }
        add(name, params, Type::Element);
    }
    out
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, String> {
    let Value::Record(fields) = value else {
        return Err("chart configuration must be a typed record".into());
    };
    fields
        .get(name)
        .ok_or_else(|| format!("chart configuration is missing '{name}'"))
}
fn required<'a>(args: &'a OrdMap<String, Value>, name: &str) -> Result<&'a Value, String> {
    args.get(name)
        .ok_or_else(|| format!("missing chart argument '{name}'"))
}
fn parse_scale(value: &Value) -> Result<LinearScale, String> {
    LinearScale {
        domain: field(value, "domain")?.vec2()?.values,
        range: field(value, "range")?.vec2()?.values,
        clamp: field(value, "clamp")?.boolean()?,
    }
    .validate()
}
fn num(args: &OrdMap<String, Value>, name: &str, default: f64) -> Result<f64, String> {
    args.get(name)
        .map(|v| math::to_f64(v.number()?.value))
        .transpose()
        .map(|v| v.unwrap_or(default))
}
fn scalar(value: f64, unit: Unit) -> Result<Value, String> {
    Ok(Value::Number(Number {
        value: math::from_f64(value)?,
        unit,
    }))
}
fn vec2(x: f64, y: f64) -> Value {
    Value::Vec2(Vec2 {
        values: [x, y],
        unit: Unit::Length,
    })
}
fn positive(value: f64, name: &str) -> Result<f64, String> {
    if !value.is_finite() || value <= 0. {
        Err(format!("{name} must be positive and finite"))
    } else {
        Ok(value)
    }
}

struct Chart<'a> {
    module: &'a str,
    identity: &'a [String],
    span: Span,
    id: String,
    args: &'a OrdMap<String, Value>,
    children: Vec<Value>,
    max_elements: usize,
}
impl<'a> Chart<'a> {
    fn new(args: &'a OrdMap<String, Value>, context: Context<'a>) -> Result<Self, String> {
        let id = args
            .get("id")
            .map(Value::string)
            .transpose()?
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{}:{}:{}",
                    context.module,
                    context.span.start,
                    context.identity.join("/")
                )
            });
        Ok(Self {
            module: context.module,
            identity: context.identity,
            span: context.span,
            id,
            args,
            children: vec![],
            max_elements: context.limits.max_elements,
        })
    }
    fn shape(
        &mut self,
        name: &str,
        key: &str,
        values: impl IntoIterator<Item = (&'static str, Value)>,
    ) -> Result<(), String> {
        if self.children.len() + 1 >= self.max_elements {
            return Err(format!(
                "chart exceeds the {} element budget; reduce ticks or samples",
                self.max_elements
            ));
        }
        let mut args: OrdMap<String, Value> =
            values.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        args.insert(
            "id".into(),
            Value::String(format!("{}/{}", self.id, key).into()),
        );
        let shape = native::make_element(name, &args, self.module, self.identity, self.span)?;
        self.children.push(shape);
        Ok(())
    }
    fn line(
        &mut self,
        key: &str,
        a: [f64; 2],
        b: [f64; 2],
        color: &Value,
        width: f64,
        opacity: f64,
    ) -> Result<(), String> {
        self.shape(
            "Line",
            key,
            [
                ("position", vec2((a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5)),
                ("end", vec2(b[0] - a[0], b[1] - a[1])),
                ("width", scalar(width, Unit::Length)?),
                ("fill", color.clone()),
                ("opacity", scalar(opacity, Unit::Number)?),
            ],
        )
    }
    fn finish(self) -> Result<Value, String> {
        let mut args: OrdMap<String, Value> = self
            .args
            .iter()
            .filter(|(name, _)| {
                matches!(
                    name.as_str(),
                    "id" | "position" | "x_align" | "y_align" | "opacity" | "rotation"
                )
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        args.insert("id".into(), Value::String(self.id.into()));
        args.insert("children".into(), Value::List(self.children.into()));
        native::make_element("Group", &args, self.module, self.identity, self.span)
    }
}
fn ticks(args: &OrdMap<String, Value>, scale: LinearScale) -> Result<Vec<f64>, String> {
    let ticks = if let Some(values) = args.get("ticks") {
        values
            .list()?
            .iter()
            .map(|v| math::to_f64(v.number()?.value))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let count = num(args, "tick_count", 6.)?;
        if count.fract() != 0. {
            return Err("tick_count must be an integer from 2 to 100".into());
        }
        scale.ticks(count as usize)?
    };
    if ticks.len() > 1000 {
        return Err("chart allows at most 1000 explicit ticks".into());
    }
    if ticks.windows(2).any(|v| v[0] >= v[1]) {
        return Err("chart ticks must be strictly increasing and unique".into());
    }
    for &tick in &ticks {
        scale.map(tick)?;
    }
    Ok(ticks)
}
fn color(args: &OrdMap<String, Value>, name: &str, default: &str) -> Result<Value, String> {
    args.get(name)
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| Color::parse(default).map(Value::Color))
}
fn number_label(value: f64, percent: bool) -> String {
    let value = if percent { value * 100. } else { value };
    let mut text = format!("{value:.6}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        text = "0".into();
    }
    if percent {
        text.push('%');
    }
    text
}

pub(crate) fn evaluate(
    name: &str,
    args: &OrdMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    match name {
        "LinearScale" => {
            let scale = LinearScale {
                domain: required(args, "domain")?.vec2()?.values,
                range: required(args, "range")?.vec2()?.values,
                clamp: args
                    .get("clamp")
                    .map(Value::boolean)
                    .transpose()?
                    .unwrap_or(false),
            }
            .validate()?;
            return Ok(Value::Record(OrdMap::from_iter([
                (
                    "domain".to_owned(),
                    Value::Vec2(Vec2 {
                        values: scale.domain,
                        unit: Unit::Number,
                    }),
                ),
                ("range".to_owned(), vec2(scale.range[0], scale.range[1])),
                ("clamp".to_owned(), Value::Bool(scale.clamp)),
            ])));
        }
        "scale_value" => {
            return scalar(
                parse_scale(required(args, "scale")?)?
                    .map(math::to_f64(required(args, "value")?.number()?.value)?)?,
                Unit::Length,
            );
        }
        "BoxSummary" => {
            let values = required(args, "values")?
                .list()?
                .iter()
                .map(|v| math::to_f64(v.number()?.value))
                .collect::<Result<Vec<_>, _>>()?;
            let summary = math::chart::summary(&values)?;
            return Ok(Value::Record(
                SUMMARY_FIELDS
                    .into_iter()
                    .zip(summary)
                    .map(|(key, v)| Ok((key.to_owned(), scalar(v, Unit::Number)?)))
                    .collect::<Result<OrdMap<_, _>, String>>()?,
            ));
        }
        _ => {}
    }
    let mut chart = Chart::new(args, context)?;
    let ink = color(
        args,
        "fill",
        if name == "Scatter" || name == "BoxPlot" {
            "blue3"
        } else {
            "light5"
        },
    )?;
    match name {
        "Scatter" => {
            let xs = parse_scale(required(args, "x_scale")?)?;
            let ys = parse_scale(required(args, "y_scale")?)?;
            let rows = required(args, "data")?.list()?;
            if rows.len() + 1 > chart.max_elements {
                return Err(format!(
                    "Scatter has {} samples; element limit is {}",
                    rows.len(),
                    chart.max_elements
                ));
            }
            let radius = positive(num(args, "radius", 2.)?, "radius")?;
            let radius = scalar(radius, Unit::Length)?;
            let point_opacity = scalar(num(args, "point_opacity", 1.)?, Unit::Number)?;
            let keys = args.get("keys").map(Value::list).transpose()?;
            if keys.is_some_and(|v| v.len() != rows.len()) {
                return Err("Scatter keys must have one unique String per data row".into());
            }
            let mut seen = BTreeSet::new();
            for (index, row) in rows.iter().enumerate() {
                let key = keys
                    .map(|v| v[index].string().map(str::to_owned))
                    .transpose()?
                    .unwrap_or_else(|| index.to_string());
                if key.is_empty() || !seen.insert(key.clone()) {
                    return Err(format!(
                        "Scatter sample {index} has an empty or duplicate key '{key}'"
                    ));
                }
                let point = (|| {
                    Ok::<_, String>([
                        xs.map(math::to_f64(field(row, "x")?.number()?.value)?)?,
                        ys.map(math::to_f64(field(row, "y")?.number()?.value)?)?,
                    ])
                })()
                .map_err(|e| format!("Scatter row {}: {e}", index + 1))?;
                chart.shape(
                    "Circle",
                    &format!("point/{key}"),
                    [
                        ("position", vec2(point[0], point[1])),
                        ("radius", radius.clone()),
                        ("opacity", point_opacity.clone()),
                        ("fill", ink.clone()),
                    ],
                )?;
            }
        }
        "BoxPlot" => {
            let scale = parse_scale(required(args, "scale")?)?;
            let summary = required(args, "summary")?;
            let values = SUMMARY_FIELDS
                .map(|key| math::to_f64(field(summary, key)?.number()?.value))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;
            if values.windows(2).any(|v| v[0] > v[1]) {
                return Err("BoxPlot summary requires low <= q1 <= middle <= q3 <= high".into());
            }
            let mapped = values
                .into_iter()
                .map(|v| scale.map(v))
                .collect::<Result<Vec<_>, _>>()?;
            let height = positive(num(args, "height", 16.)?, "height")?;
            let width = positive(num(args, "width", 1.8)?, "width")?;
            let [low, q1, middle, q3, high]: [f64; 5] =
                mapped.try_into().expect("five summary fields");
            for (key, a, b) in [
                ("lower_whisker", [low, 0.], [q1, 0.]),
                ("upper_whisker", [q3, 0.], [high, 0.]),
                ("box_top", [q1, -height / 2.], [q3, -height / 2.]),
                ("box_bottom", [q1, height / 2.], [q3, height / 2.]),
                ("box_left", [q1, -height / 2.], [q1, height / 2.]),
                ("box_right", [q3, -height / 2.], [q3, height / 2.]),
                ("lower_cap", [low, -height * 0.65], [low, height * 0.65]),
                ("upper_cap", [high, -height * 0.65], [high, height * 0.65]),
            ] {
                chart.line(key, a, b, &ink, width, 1.)?;
            }
            chart.line(
                "median",
                [middle, -height * 0.7],
                [middle, height * 0.7],
                &color(args, "median_color", "light1")?,
                width * 1.25,
                1.,
            )?;
            let mut value = chart.finish()?;
            if let Value::Element(element) = &mut value {
                let element = std::sync::Arc::make_mut(element);
                element.horizontal.origin = Number {
                    value: Fraction::new(1u64, 2u64),
                    unit: Unit::Percent,
                };
                element.horizontal.offset = math::from_f64(middle)?;
            }
            return Ok(value);
        }
        "Grid" | "Axis" => {
            let scale = parse_scale(required(args, "scale")?)?;
            let values = ticks(args, scale)?;
            let vertical = args
                .get("vertical")
                .map(Value::boolean)
                .transpose()?
                .unwrap_or(false);
            let xy = |along, across| {
                if vertical {
                    [across, along]
                } else {
                    [along, across]
                }
            };
            let width = positive(
                num(args, "width", if name == "Grid" { 0.75 } else { 1.125 })?,
                "width",
            )?;
            if name == "Grid" {
                let spans = required(args, "spans")?.list()?;
                if spans.len().saturating_mul(values.len()) >= chart.max_elements {
                    return Err("Grid tick/span product exceeds element budget".into());
                }
                let highlighted = args
                    .get("highlight")
                    .map(Value::list)
                    .transpose()?
                    .unwrap_or(&[])
                    .iter()
                    .map(|v| math::to_f64(v.number()?.value))
                    .collect::<Result<Vec<_>, _>>()?;
                for value in values {
                    let along = scale.map(value)?;
                    let opacity = num(
                        args,
                        if highlighted.contains(&value) {
                            "highlight_opacity"
                        } else {
                            "grid_opacity"
                        },
                        if highlighted.contains(&value) {
                            0.34
                        } else {
                            0.15
                        },
                    )?;
                    for (j, span) in spans.iter().enumerate() {
                        let [a, b] = span.vec2()?.values;
                        if !a.is_finite() || !b.is_finite() || a >= b {
                            return Err(format!(
                                "Grid span {} must have finite increasing endpoints",
                                j + 1
                            ));
                        }
                        chart.line(
                            &format!("tick/{value}/span/{j}"),
                            xy(along, a),
                            xy(along, b),
                            &ink,
                            width,
                            opacity,
                        )?;
                    }
                }
            } else {
                axis(&mut chart, args, scale, &values, vertical, &ink, width)?;
            }
        }
        _ => return Err(format!("unknown native chart '{name}'")),
    }
    chart.finish()
}

fn axis(
    chart: &mut Chart<'_>,
    args: &OrdMap<String, Value>,
    scale: LinearScale,
    values: &[f64],
    vertical: bool,
    ink: &Value,
    width: f64,
) -> Result<(), String> {
    let xy = |along, across| {
        if vertical {
            [across, along]
        } else {
            [along, across]
        }
    };
    chart.line(
        "baseline",
        xy(scale.range[0], 0.),
        xy(scale.range[1], 0.),
        ink,
        width,
        0.7,
    )?;
    let length = positive(num(args, "tick_length", 5.)?, "tick_length")?;
    let size = positive(num(args, "font_size", 22.)?, "font_size")?;
    let gap = num(args, "label_gap", 8.)?;
    if gap < 0. {
        return Err("label_gap must be nonnegative".into());
    }
    let font = required(args, "font")?;
    let show_labels = args
        .get("show_labels")
        .map(Value::boolean)
        .transpose()?
        .unwrap_or(true);
    let labels = args.get("labels").map(Value::list).transpose()?;
    if labels.is_some_and(|v| v.len() != values.len()) {
        return Err("Axis labels must have exactly one String per tick".into());
    }
    let format = args
        .get("format")
        .map(Value::string)
        .transpose()?
        .unwrap_or("number");
    if !matches!(format, "number" | "percent") {
        return Err(
            "Axis format must be 'number' or 'percent'; provide labels for custom text".into(),
        );
    }
    let mut intervals = vec![];
    let mut across_extent: f64 = 0.;
    for (index, &value) in values.iter().enumerate() {
        let along = scale.map(value)?;
        let key = value.to_string();
        chart.line(
            &format!("tick/{key}"),
            xy(along, -length / 2.),
            xy(along, length / 2.),
            ink,
            width,
            1.,
        )?;
        if !show_labels {
            continue;
        }
        let label = labels
            .map(|v| v[index].string().map(str::to_owned))
            .transpose()?
            .unwrap_or_else(|| number_label(value, format == "percent"));
        chart.shape(
            "Text",
            &format!("label/{key}"),
            [
                ("text", Value::String(label.into())),
                ("font", font.clone()),
                ("font_size", scalar(size, Unit::Length)?),
                ("fill", ink.clone()),
            ],
        )?;
        let Value::Element(last) = chart.children.last_mut().expect("label") else {
            unreachable!()
        };
        let last = std::sync::Arc::make_mut(last);
        let Geometry::Text { bounds, .. } = last.geometry.as_ref() else {
            unreachable!()
        };
        let half_along = (bounds.max.values[usize::from(vertical)]
            - bounds.min.values[usize::from(vertical)])
            / 2.;
        let half_across = (bounds.max.values[usize::from(!vertical)]
            - bounds.min.values[usize::from(!vertical)])
            / 2.;
        intervals.push((along - half_along, along + half_along));
        across_extent = across_extent.max(2. * half_across);
        let pos = xy(along, length / 2. + gap + half_across);
        last.position = Vec2 {
            values: pos,
            unit: Unit::Length,
        };
    }
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    if intervals.windows(2).any(|v| v[0].1 + gap > v[1].0) {
        return Err(
            "Axis tick labels overlap; reduce tick_count/font_size or increase the scale range"
                .into(),
        );
    }
    if let Some(title) = args.get("title") {
        let [x, y] = xy(
            (scale.range[0] + scale.range[1]) / 2.,
            length / 2. + gap * 2. + across_extent + size / 2.,
        );
        chart.shape(
            "Text",
            "title",
            [
                ("text", title.clone()),
                ("font", font.clone()),
                ("font_size", scalar(size, Unit::Length)?),
                ("position", vec2(x, y)),
                ("fill", ink.clone()),
            ],
        )?;
    }
    Ok(())
}
