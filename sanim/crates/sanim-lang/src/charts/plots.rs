//! Statistical plots share native shapes, palettes, scales, and font shaping.
use super::*;
use sanim_core::math::chart::plot::{self, DensityOptions, Normalization};

#[derive(Clone, Copy)]
enum Kind {
    Polyline,
    Range,
    Line,
    Area,
    Band,
    Histogram,
    Density,
    Normal,
    Ecdf,
    Legend,
}
const PLOTS: &[(&str, Kind)] = &[
    ("Polyline", Kind::Polyline),
    ("DataRange", Kind::Range),
    ("LinePlot", Kind::Line),
    ("AreaPlot", Kind::Area),
    ("BandPlot", Kind::Band),
    ("Histogram", Kind::Histogram),
    ("DensityPlot", Kind::Density),
    ("NormalCurve", Kind::Normal),
    ("ECDFPlot", Kind::Ecdf),
    ("Legend", Kind::Legend),
];

pub(super) fn expensive(name: &str) -> Option<&'static str> {
    let kind = PLOTS
        .iter()
        .find_map(|(candidate, kind)| (*candidate == name).then_some(*kind))?;
    Some(match kind {
        Kind::Density => "Gaussian kernel density estimation",
        Kind::Histogram | Kind::Ecdf | Kind::Range => "statistical data processing",
        Kind::Polyline | Kind::Line | Kind::Area | Kind::Band | Kind::Normal => {
            "curve/area geometry processing"
        }
        Kind::Legend => "legend text shaping",
    })
}

pub(super) fn builtins(common: &[Parameter]) -> Vec<Builtin> {
    let num = Type::Numeric(Unit::Number);
    let length = Type::Numeric(Unit::Length);
    let list = |ty| Type::List(Box::new(ty), sanim_core::ListLength::Unknown);
    let p = |name, ty, required| Parameter { name, ty, required };
    PLOTS
        .iter()
        .map(|&(name, kind)| {
            let mut parameters = if matches!(kind, Kind::Range) {
                Vec::new()
            } else {
                common
                    .iter()
                    .filter(|parameter| parameter.name != "scale")
                    .cloned()
                    .collect()
            };
            match kind {
                Kind::Range => parameters.extend([
                    p("values", list(num.clone()), true),
                    p("padding", num.clone(), false),
                    p("include_zero", Type::Bool, false),
                ]),
                Kind::Polyline => parameters.extend([
                    p("points", list(Type::Vector(2, Some(Unit::Length))), true),
                    p("width", length.clone(), false),
                ]),
                Kind::Legend => parameters.extend([
                    p("labels", list(Type::String), true),
                    p("colors", list(Type::Color), false),
                    p("font", Type::Font, false),
                    p("font_size", length.clone(), false),
                    p("gap", length.clone(), false),
                    p("swatch_width", length.clone(), false),
                    p("vertical", Type::Bool, false),
                ]),
                _ => {
                    parameters.extend([
                        p("x_scale", scale_type(), true),
                        p("y_scale", scale_type(), true),
                        p("width", length.clone(), false),
                    ]);
                    match kind {
                        Kind::Line | Kind::Area => {
                            parameters.extend([
                                p(
                                    "data",
                                    list(record([("x", num.clone()), ("y", num.clone())])),
                                    true,
                                ),
                                p("interpolation", Type::String, false),
                                p("subdivisions", num.clone(), false),
                            ]);
                            if matches!(kind, Kind::Area) {
                                parameters.extend([
                                    p("baseline", num.clone(), false),
                                    p("fill_opacity", num.clone(), false),
                                ]);
                            }
                        }
                        Kind::Band => parameters.extend([
                            p(
                                "data",
                                list(record([
                                    ("x", num.clone()),
                                    ("low", num.clone()),
                                    ("high", num.clone()),
                                ])),
                                true,
                            ),
                            p("fill_opacity", num.clone(), false),
                        ]),
                        Kind::Histogram => parameters.extend([
                            p("values", list(num.clone()), true),
                            p("bins", num.clone(), false),
                            p("edges", list(num.clone()), false),
                            p("normalization", Type::String, false),
                            p("gap", num.clone(), false),
                        ]),
                        Kind::Density | Kind::Normal => {
                            if matches!(kind, Kind::Density) {
                                parameters.extend([
                                    p("values", list(num.clone()), true),
                                    p("bandwidth", num.clone(), false),
                                ]);
                            } else {
                                parameters.extend([
                                    p("mean", num.clone(), false),
                                    p("std_dev", num.clone(), false),
                                ]);
                            }
                            parameters.extend([
                                p("points", num.clone(), false),
                                p("shade", Type::Bool, false),
                                p("fill_opacity", num.clone(), false),
                            ]);
                        }
                        Kind::Ecdf => parameters.push(p("values", list(num.clone()), true)),
                        _ => unreachable!("non-statistical constructor"),
                    }
                }
            }
            Builtin {
                name,
                parameters,
                result: if matches!(kind, Kind::Range) {
                    Type::Vector(2, Some(Unit::Number))
                } else {
                    Type::Element
                },
                schedule: false,
                timing: Default::default(),
                content: Default::default(),
                execute: Execution::Native(evaluate),
                check: native_types::fixed,
            }
        })
        .collect()
}

fn numbers(args: &OrdMap<String, Value>, name: &str) -> Result<Vec<f64>, String> {
    required(args, name)?
        .list()?
        .iter()
        .map(|value| math::to_f64(value.number()?.value))
        .collect()
}
fn integer(
    args: &OrdMap<String, Value>,
    name: &str,
    default: usize,
    max: usize,
) -> Result<usize, String> {
    let value = num(args, name, default as f64)?;
    if !value.is_finite() || value.fract() != 0. || value < 1. || value > max as f64 {
        return Err(format!("{name} must be an integer from 1 to {max}"));
    }
    Ok(value as usize)
}
fn boolean(args: &OrdMap<String, Value>, name: &str, default: bool) -> Result<bool, String> {
    args.get(name)
        .map(Value::boolean)
        .transpose()
        .map(|value| value.unwrap_or(default))
}
fn text<'a>(
    args: &'a OrdMap<String, Value>,
    name: &str,
    default: &'a str,
) -> Result<&'a str, String> {
    args.get(name)
        .map(Value::string)
        .transpose()
        .map(|value| value.unwrap_or(default))
}
fn fraction(args: &OrdMap<String, Value>, name: &str, default: f64) -> Result<f64, String> {
    let value = num(args, name, default)?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!("{name} must be finite and between 0 and 1"));
    }
    Ok(value)
}
fn rows(args: &OrdMap<String, Value>) -> Result<Vec<[f64; 2]>, String> {
    required(args, "data")?
        .list()?
        .iter()
        .enumerate()
        .map(|(index, row)| {
            Ok([
                math::to_f64(field(row, "x")?.number()?.value)
                    .map_err(|error| format!("row {} x: {error}", index + 1))?,
                math::to_f64(field(row, "y")?.number()?.value)
                    .map_err(|error| format!("row {} y: {error}", index + 1))?,
            ])
        })
        .collect()
}
fn map_points(
    points: &[[f64; 2]],
    xs: LinearScale,
    ys: LinearScale,
) -> Result<Vec<[f64; 2]>, String> {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            Ok([
                xs.map(point[0])
                    .map_err(|error| format!("plot point {} x: {error}", index + 1))?,
                ys.map(point[1])
                    .map_err(|error| format!("plot point {} y: {error}", index + 1))?,
            ])
        })
        .collect()
}

impl Chart<'_> {
    /// Native geometry is centered once so each point keeps the same parent-local
    /// scale coordinates as Scatter, regardless of asymmetric bounds.
    fn path(
        &mut self,
        key: &str,
        points: &[[f64; 2]],
        ink: &Value,
        width: Option<f64>,
        opacity: f64,
    ) -> Result<(), String> {
        let minimum = if width.is_some() { 2 } else { 3 };
        if points.len() < minimum {
            return Ok(());
        }
        if points.len() > plot::MAX_POINTS {
            return Err(format!(
                "plot exceeds {} vertices; reduce interpolation/sampling resolution",
                plot::MAX_POINTS
            ));
        }
        let mut low = points[0];
        let mut high = low;
        for point in points.iter().skip(1) {
            for axis in 0..2 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        let center = [
            math::float_arithmetic("+", low[0] / 2., high[0] / 2.)?,
            math::float_arithmetic("+", low[1] / 2., high[1] / 2.)?,
        ];
        let vertices = points
            .iter()
            .map(|point| {
                Ok(vec2(
                    math::float_arithmetic("-", point[0], center[0])?,
                    math::float_arithmetic("-", point[1], center[1])?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut values = vec![
            ("points", Value::List(vertices.into())),
            ("position", vec2(center[0], center[1])),
            ("fill", ink.clone()),
            ("opacity", scalar(opacity, Unit::Number)?),
        ];
        if let Some(width) = width {
            values.push(("width", scalar(width, Unit::Length)?));
        }
        self.shape(
            if width.is_some() {
                "Polyline"
            } else {
                "Polygon"
            },
            key,
            values,
        )
    }
}

fn work_units(kind: Kind, args: &OrdMap<String, Value>) -> Result<usize, String> {
    let count = match kind {
        Kind::Polyline => required(args, "points")?.list()?.len(),
        Kind::Range | Kind::Histogram | Kind::Density | Kind::Ecdf => {
            required(args, "values")?.list()?.len()
        }
        Kind::Line | Kind::Area | Kind::Band => required(args, "data")?.list()?.len(),
        Kind::Normal => 0,
        Kind::Legend => {
            return required(args, "labels")?
                .list()?
                .iter()
                .try_fold(0usize, |work, label| {
                    work.checked_add(label.string()?.len().saturating_add(1))
                        .ok_or_else(|| "Legend work count overflow".into())
                });
        }
    };
    let sorting = count.max(1).ilog2() as usize + 1;
    let multiplier = match kind {
        Kind::Range => 4,
        Kind::Polyline => 2,
        Kind::Line | Kind::Area => {
            if text(args, "interpolation", "linear")? == "monotone" {
                integer(args, "subdivisions", 8, 64)?.saturating_add(4)
            } else {
                2
            }
        }
        Kind::Band => 4,
        Kind::Histogram => {
            let bins = if let Some(edges) = args.get("edges") {
                edges.list()?.len().saturating_sub(1)
            } else {
                integer(args, "bins", 20, plot::MAX_BINS)?
            };
            bins.max(1).ilog2() as usize + 3
        }
        Kind::Ecdf => sorting.saturating_add(3),
        Kind::Density => integer(args, "points", 128, plot::MAX_POINTS)?
            .checked_add(if args.contains_key("bandwidth") {
                2
            } else {
                sorting.saturating_add(4)
            })
            .ok_or("DensityPlot work count overflow")?,
        Kind::Normal => {
            return integer(args, "points", 128, plot::MAX_POINTS)?
                .checked_mul(4)
                .ok_or_else(|| "NormalCurve work count overflow".into());
        }
        Kind::Legend => unreachable!("legend handled above"),
    };
    count
        .checked_mul(multiplier)
        .ok_or_else(|| "plot work count overflow".into())
}

fn evaluate(
    name: &str,
    args: &OrdMap<String, Value>,
    mut context: Context<'_>,
) -> Result<Value, String> {
    let kind = PLOTS
        .iter()
        .find_map(|(candidate, kind)| (*candidate == name).then_some(*kind))
        .ok_or("unknown plot constructor")?;
    let max_work = usize::try_from(context.remaining_work).unwrap_or(usize::MAX);
    context.charge_work(work_units(kind, args)?)?;
    if matches!(kind, Kind::Polyline) {
        return native::make_element(name, args, context.module, context.identity, context.span);
    }
    if matches!(kind, Kind::Range) {
        let bounds = plot::extent(
            &numbers(args, "values")?,
            num(args, "padding", 0.05)?,
            boolean(args, "include_zero", false)?,
        )?;
        return Ok(Value::Vec2(Vec2 {
            values: bounds,
            unit: Unit::Number,
        }));
    }
    let max_values = context.limits.data.max_values;
    let mut chart = Chart::new(name, args, context)?;
    if matches!(kind, Kind::Legend) {
        legend(&mut chart, args)?;
        return chart.finish();
    }
    let xs = parse_scale(required(args, "x_scale")?)?;
    let ys = parse_scale(required(args, "y_scale")?)?;
    let width = positive(num(args, "width", 2.5)?, "width")?;
    let ink = color(args, "fill", "blue3")?;
    match kind {
        Kind::Line | Kind::Area => {
            let data = rows(args)?;
            let points = plot::curve(
                &data,
                text(args, "interpolation", "linear")?,
                integer(args, "subdivisions", 8, 64)?,
            )?;
            let mut mapped = map_points(&points, xs, ys)?;
            if matches!(kind, Kind::Area) && !points.is_empty() {
                let baseline = ys.map(num(args, "baseline", 0.)?)?;
                mapped.push([mapped.last().expect("nonempty curve")[0], baseline]);
                mapped.push([mapped[0][0], baseline]);
                chart.path(
                    "area",
                    &mapped,
                    &ink,
                    None,
                    fraction(args, "fill_opacity", 0.22)?,
                )?;
            } else {
                chart.path("curve", &mapped, &ink, Some(width), 1.)?;
            }
        }
        Kind::Band => {
            let data = required(args, "data")?.list()?;
            if data.len().saturating_mul(2) > plot::MAX_POINTS {
                return Err("BandPlot has too many rows; reduce data before plotting".into());
            }
            let mut lower = Vec::with_capacity(data.len());
            let mut upper = Vec::with_capacity(data.len());
            let mut previous = None;
            for (index, row) in data.iter().enumerate() {
                let x = math::to_f64(field(row, "x")?.number()?.value)?;
                let low = math::to_f64(field(row, "low")?.number()?.value)?;
                let high = math::to_f64(field(row, "high")?.number()?.value)?;
                if previous.is_some_and(|previous| x <= previous) || low > high {
                    return Err(format!(
                        "BandPlot row {} requires increasing x and low <= high",
                        index + 1
                    ));
                }
                previous = Some(x);
                lower.push([xs.map(x)?, ys.map(low)?]);
                upper.push([xs.map(x)?, ys.map(high)?]);
            }
            lower.extend(upper.into_iter().rev());
            chart.path(
                "band",
                &lower,
                &ink,
                None,
                fraction(args, "fill_opacity", 0.20)?,
            )?;
        }
        Kind::Histogram => {
            let values = numbers(args, "values")?;
            if values.len() > max_values || values.len() > max_work {
                return Err(
                    "Histogram input exceeds the configured sample/operation budget".into(),
                );
            }
            if args.contains_key("bins") && args.contains_key("edges") {
                return Err("Histogram accepts bins or explicit edges, not both".into());
            }
            let edges = if args.contains_key("edges") {
                numbers(args, "edges")?
            } else {
                plot::bin_edges(xs.domain, integer(args, "bins", 20, plot::MAX_BINS)?)?
            };
            for edge in &edges {
                xs.map(*edge)?;
            }
            let heights = plot::histogram(
                &values,
                &edges,
                Normalization::parse(text(args, "normalization", "count")?)?,
            )?;
            let baseline = ys.map(0.)?;
            let gap = fraction(args, "gap", 0.08)?;
            if gap == 1. {
                return Err("Histogram gap must be less than 1".into());
            }
            for (index, height) in heights.into_iter().enumerate() {
                if height == 0. {
                    continue;
                }
                let (left, right) = (xs.map(edges[index])?, xs.map(edges[index + 1])?);
                let top = ys
                    .map(height)
                    .map_err(|error| format!("Histogram bin {}: {error}", index + 1))?;
                if top == baseline {
                    continue;
                }
                chart.shape(
                    "Rect",
                    &format!("bin/{index}"),
                    [
                        ("position", vec2((left + right) / 2., (top + baseline) / 2.)),
                        (
                            "width",
                            scalar((right - left).abs() * (1. - gap), Unit::Length)?,
                        ),
                        ("height", scalar((top - baseline).abs(), Unit::Length)?),
                        ("fill", ink.clone()),
                    ],
                )?;
            }
        }
        Kind::Density | Kind::Normal => {
            let options = DensityOptions {
                domain: xs.domain,
                points: integer(args, "points", 128, plot::MAX_POINTS)?,
                bandwidth: args
                    .get("bandwidth")
                    .map(|value| math::to_f64(value.number()?.value))
                    .transpose()?,
                max_work,
            };
            let points = if matches!(kind, Kind::Density) {
                let values = numbers(args, "values")?;
                if values.len() > max_values {
                    return Err("DensityPlot exceeds the configured sample budget".into());
                }
                plot::density(&values, options)?
            } else {
                plot::normal_density(num(args, "mean", 0.)?, num(args, "std_dev", 1.)?, options)?
            };
            let mapped = map_points(&points, xs, ys)?;
            if boolean(args, "shade", true)? {
                let baseline = ys.map(0.)?;
                let mut area = mapped.clone();
                area.push([area.last().expect("density points")[0], baseline]);
                area.push([area[0][0], baseline]);
                chart.path(
                    "area",
                    &area,
                    &ink,
                    None,
                    fraction(args, "fill_opacity", 0.18)?,
                )?;
            }
            chart.path("curve", &mapped, &ink, Some(width), 1.)?;
        }
        Kind::Ecdf => {
            let values = numbers(args, "values")?;
            if values.len() > max_values || values.len() > max_work {
                return Err("ECDFPlot exceeds the configured sample/operation budget".into());
            }
            let mut points = plot::ecdf(&values)?;
            if !points.is_empty() {
                if points[0][0] > xs.domain[0] {
                    points.insert(0, [xs.domain[0], 0.]);
                }
                if points.last().expect("ECDF points")[0] < xs.domain[1] {
                    points.push([xs.domain[1], 1.]);
                }
            }
            chart.path(
                "curve",
                &map_points(&points, xs, ys)?,
                &ink,
                Some(width),
                1.,
            )?;
        }
        _ => unreachable!("handled constructor"),
    }
    chart.finish()
}

fn legend(chart: &mut Chart<'_>, args: &OrdMap<String, Value>) -> Result<(), String> {
    const PALETTE: [&str; 6] = ["blue3", "orange3", "green3", "purple3", "red3", "yellow3"];
    let labels = required(args, "labels")?.list()?;
    let colors = args.get("colors").map(Value::list).transpose()?;
    if colors.is_some_and(|colors| colors.len() != labels.len()) {
        return Err("Legend colors must have exactly one Color per label".into());
    }
    if labels.len() > 100 {
        return Err("Legend supports at most 100 entries".into());
    }
    let size = positive(num(args, "font_size", 14.)?, "font_size")?;
    let gap = positive(num(args, "gap", 16.)?, "gap")?;
    let swatch = positive(num(args, "swatch_width", 20.)?, "swatch_width")?;
    let vertical = boolean(args, "vertical", false)?;
    let font = required(args, "font")?;
    let mut cursor = 0.;
    for (index, label) in labels.iter().enumerate() {
        let ink = colors
            .map(|colors| Ok(colors[index].clone()))
            .unwrap_or_else(|| color(&OrdMap::new(), "fill", PALETTE[index % PALETTE.len()]))?;
        let (x, y) = if vertical {
            (0., index as f64 * (size + gap))
        } else {
            (cursor, 0.)
        };
        chart.line(
            &format!("swatch/{index}"),
            [x - swatch / 2., y],
            [x + swatch / 2., y],
            &ink,
            2.5,
            1.,
        )?;
        chart.shape(
            "Text",
            &format!("label/{index}"),
            [
                ("text", label.clone()),
                ("font", font.clone()),
                ("font_size", scalar(size, Unit::Length)?),
                ("fill", color(args, "fill", "light2")?),
            ],
        )?;
        let Value::Element(element) = chart.children.last_mut().expect("legend label") else {
            unreachable!()
        };
        let element = std::sync::Arc::make_mut(element);
        let Geometry::Text { bounds, .. } = element.geometry.as_ref() else {
            unreachable!()
        };
        let text_width = bounds.max.values[0] - bounds.min.values[0];
        element.position = Vec2 {
            values: [x + swatch / 2. + gap / 2. + text_width / 2., y],
            unit: Unit::Length,
        };
        cursor += swatch + gap + text_width + gap;
    }
    Ok(())
}
