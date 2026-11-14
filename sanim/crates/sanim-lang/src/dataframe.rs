//! Pandas-style names are a typed surface over the shared SQLite plan builder.
use crate::{
    native::Context,
    native_types,
    prelude::{Builtin, Execution, Parameter},
};
use sanim_core::{
    Type, Unit, Value,
    dataframe::{Frame, Series},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

const FRAME: u8 = 1;
const SERIES: u8 = 2;
const GROUPED_FRAME: u8 = 4;

#[derive(Clone, Copy)]
pub(crate) enum MaterializationEffect {
    Never,
    Always,
    SeriesOnly,
}
impl MaterializationEffect {
    pub fn applies(self, series: bool) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::SeriesOnly => series,
        }
    }
}

pub(crate) struct Definition {
    pub name: &'static str,
    pub parameters: &'static [(&'static str, bool)],
    pub materialization: MaterializationEffect,
    receivers: u8,
}
const DEFINITIONS: &[Definition] = &[
    Definition {
        name: "DataFrame",
        parameters: &[("rows", true)],
        receivers: 0,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.to_list",
        parameters: &[("frame", true)],
        receivers: FRAME | SERIES,
        materialization: MaterializationEffect::Always,
    },
    Definition {
        name: "DataFrame.select",
        parameters: &[("frame", true), ("columns", true)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.drop",
        parameters: &[("frame", true), ("columns", true)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.rename",
        parameters: &[("frame", true), ("columns", true)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.assign",
        parameters: &[("frame", true), ("columns", true)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.head",
        parameters: &[("frame", true), ("n", false)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.tail",
        parameters: &[("frame", true), ("n", false)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.sort_values",
        parameters: &[("frame", true), ("by", true), ("ascending", false)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.groupby",
        parameters: &[("frame", true), ("by", true)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.agg",
        parameters: &[("frame", true), ("operations", true)],
        receivers: FRAME | GROUPED_FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.mean",
        parameters: &[("frame", true)],
        receivers: FRAME | SERIES | GROUPED_FRAME,
        materialization: MaterializationEffect::SeriesOnly,
    },
    Definition {
        name: "DataFrame.sum",
        parameters: &[("frame", true)],
        receivers: FRAME | SERIES | GROUPED_FRAME,
        materialization: MaterializationEffect::SeriesOnly,
    },
    Definition {
        name: "DataFrame.count",
        parameters: &[("frame", true)],
        receivers: FRAME | SERIES | GROUPED_FRAME,
        materialization: MaterializationEffect::SeriesOnly,
    },
    Definition {
        name: "DataFrame.min",
        parameters: &[("frame", true)],
        receivers: FRAME | SERIES | GROUPED_FRAME,
        materialization: MaterializationEffect::SeriesOnly,
    },
    Definition {
        name: "DataFrame.max",
        parameters: &[("frame", true)],
        receivers: FRAME | SERIES | GROUPED_FRAME,
        materialization: MaterializationEffect::SeriesOnly,
    },
    Definition {
        name: "DataFrame.merge",
        parameters: &[
            ("frame", true),
            ("right", true),
            ("on", true),
            ("how", false),
            ("suffixes", false),
        ],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.isna",
        parameters: &[("frame", true)],
        receivers: SERIES,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.fillna",
        parameters: &[("frame", true), ("value", true)],
        receivers: SERIES,
        materialization: MaterializationEffect::Never,
    },
    Definition {
        name: "DataFrame.dropna",
        parameters: &[("frame", true), ("subset", false)],
        receivers: FRAME,
        materialization: MaterializationEffect::Never,
    },
];

fn registry() -> &'static [Builtin] {
    static BUILTINS: OnceLock<Vec<Builtin>> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        DEFINITIONS
            .iter()
            .map(|definition| {
                let mut builtin = Builtin {
                    name: definition.name,
                    parameters: definition
                        .parameters
                        .iter()
                        .map(|(name, required)| Parameter {
                            name,
                            ty: match *name {
                                "ascending" => Type::Bool,
                                "how" => Type::String,
                                "n" => Type::Numeric(Unit::Number),
                                _ => Type::Unknown,
                            },
                            required: *required,
                        })
                        .collect(),
                    result: Type::Unknown,
                    schedule: false,
                    timing: Default::default(),
                    content: Default::default(),
                    execute: Execution::Access(execute),
                    check: native_types::fixed,
                };
                crate::prelude::finalize(&mut builtin);
                builtin
            })
            .collect()
    })
}
pub(crate) fn register(output: &mut Vec<Builtin>) {
    output.push(registry()[0].clone());
}
pub fn methods() -> impl Iterator<Item = &'static Builtin> {
    registry().iter().skip(1)
}
/// Receiver support belongs to the same registry used by type checking and dispatch.
/// Editors can offer methods without copying the runtime's operation names.
pub fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    let receiver = match receiver {
        Type::DataFrame { groups, .. } if groups.is_empty() => FRAME,
        Type::DataFrame { .. } => GROUPED_FRAME,
        Type::Series(_) => SERIES,
        _ => 0,
    };
    registry()
        .iter()
        .zip(DEFINITIONS)
        .skip(1)
        .filter_map(move |(builtin, definition)| {
            (definition.receivers & receiver != 0).then_some(builtin)
        })
}

pub(crate) fn definition(name: &str) -> Option<&'static Definition> {
    DEFINITIONS
        .iter()
        .find(|definition| definition.name == name)
}

fn strings(value: &Value) -> Result<Vec<String>, String> {
    if let Value::String(name) = value {
        return Ok(vec![name.to_string()]);
    }
    value
        .list()?
        .iter()
        .map(|value| value.string().map(str::to_owned))
        .collect()
}
fn mapping(value: &Value) -> Result<BTreeMap<String, String>, String> {
    match value {
        Value::Record(fields) => fields
            .iter()
            .map(|(name, value)| Ok((name.clone(), value.string()?.to_owned())))
            .collect(),
        Value::Dict(fields) => fields
            .iter()
            .map(|(name, value)| {
                value
                    .as_str()
                    .map(|value| (name.clone(), value.into()))
                    .ok_or_else(|| format!("column mapping '{name}' requires a String"))
            })
            .collect(),
        _ => Err("column mappings require a named record or string-key dictionary".into()),
    }
}
fn frame(args: &imbl::OrdMap<String, Value>) -> Result<&Frame, String> {
    match args.get("frame") {
        Some(Value::DataFrame(frame)) => Ok(frame),
        Some(value) => Err(format!(
            "DataFrame method requires DataFrame, found {}",
            value.ty()
        )),
        None => Err("missing DataFrame receiver".into()),
    }
}
pub(crate) fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: Context<'_>,
) -> Result<Value, String> {
    if name == "DataFrame" {
        let Some(Value::List(rows)) = args.get("rows") else {
            return Err("DataFrame requires a typed List<Row>".into());
        };
        let Type::Record(columns) = rows.element_type.as_ref() else {
            return Err("DataFrame requires typed record rows".into());
        };
        context.charge_work(rows.len().saturating_mul(columns.len().saturating_add(1)))?;
        return Frame::new(rows.clone()).map(|frame| Value::DataFrame(Arc::new(frame)));
    }
    let operation = name
        .strip_prefix("DataFrame.")
        .ok_or("unknown DataFrame operation")?;
    if let Some(Value::Series(series)) = args.get("frame") {
        return match operation {
            "isna" => series.isna().map(|series| Value::Series(Arc::new(series))),
            "fillna" => series
                .fillna(args.get("value").ok_or("fillna requires value")?)
                .map(|series| Value::Series(Arc::new(series))),
            _ => Err(format!(
                "Series.{operation} is not a lazy column operation; use to_list() to request values"
            )),
        };
    }
    let frame = frame(args)?;
    let required = |name| {
        args.get(name)
            .ok_or_else(|| format!("{operation} requires '{name}'"))
    };
    let result = match operation {
        "select" => frame.select(&strings(required("columns")?)?),
        "drop" => frame.drop_columns(&strings(required("columns")?)?),
        "rename" => frame.rename(&mapping(required("columns")?)?),
        "assign" => {
            let Value::Record(values) = required("columns")? else {
                return Err("assign requires a named record of scalar/Series expressions, such as assign({ratio=df[\"a\"]/df[\"b\"]})".into());
            };
            frame.assign(
                &values
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            )
        }
        "head" | "tail" => frame.limit(
            args.get("n")
                .map(|value| value.number()?.usize())
                .transpose()?
                .unwrap_or(5),
            operation == "tail",
        ),
        "sort_values" => frame.sort(
            strings(required("by")?)?,
            args.get("ascending")
                .map(Value::boolean)
                .transpose()?
                .unwrap_or(true),
        ),
        "groupby" => frame.groupby(strings(required("by")?)?),
        "agg" => frame.aggregate(&mapping(required("operations")?)?),
        "mean" | "sum" | "count" | "min" | "max" => {
            let operations = sanim_core::dataframe::default_aggregations(
                &frame.schema,
                &frame.groups,
                operation,
            )?;
            frame.aggregate(&operations)
        }
        "merge" => {
            let Value::DataFrame(right) = required("right")? else {
                return Err("merge right must be a DataFrame".into());
            };
            let suffixes = args
                .get("suffixes")
                .map(strings)
                .transpose()?
                .unwrap_or_else(|| vec!["_x".into(), "_y".into()]);
            if suffixes.len() != 2 {
                return Err("merge suffixes must contain exactly two Strings".into());
            }
            frame.merge(
                right,
                strings(required("on")?)?,
                args.get("how")
                    .map(Value::string)
                    .transpose()?
                    .unwrap_or("inner"),
                [&suffixes[0], &suffixes[1]],
            )
        }
        "dropna" => {
            let names = args
                .get("subset")
                .map(strings)
                .transpose()?
                .unwrap_or_else(|| frame.schema.keys().cloned().collect());
            let mut mask = None;
            for name in names {
                let present = frame.column(&name)?.isna()?.unary("not")?;
                mask = Some(if let Some(previous) = mask {
                    Series::binary(
                        "and",
                        Value::Series(Arc::new(previous)),
                        Value::Series(Arc::new(present)),
                    )?
                } else {
                    present
                });
            }
            if let Some(mask) = mask {
                frame.filter(&mask)
            } else {
                Ok(frame.clone())
            }
        }
        _ => Err(format!(
            "DataFrame.{operation} is not supported for this receiver"
        )),
    }?;
    Ok(Value::DataFrame(Arc::new(result)))
}

pub(crate) struct Materialization {
    pub frame: Frame,
    pub series: Option<Series>,
    pub scalar: bool,
}
pub(crate) fn materialization(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
) -> Result<Option<Materialization>, String> {
    if name == "len" {
        let frame = match args.get("items") {
            Some(Value::DataFrame(frame)) => Some(frame.as_ref()),
            Some(Value::Series(series)) => Some(&series.frame),
            _ => None,
        };
        if let Some(frame) = frame {
            if !frame.groups.is_empty() {
                return Err("aggregate a grouped DataFrame before requesting its length".into());
            }
            let name = frame
                .schema
                .keys()
                .next()
                .ok_or("DataFrame has no columns")?;
            let aggregate = frame.aggregate(&BTreeMap::from([(name.clone(), "size".into())]))?;
            let series = aggregate.column(name)?;
            return Ok(Some(Materialization {
                frame: aggregate,
                series: Some(series),
                scalar: true,
            }));
        }
    }
    let Some(definition) = definition(name) else {
        return Ok(None);
    };
    if !definition
        .materialization
        .applies(matches!(args.get("frame"), Some(Value::Series(_))))
    {
        return Ok(None);
    }
    let operation = name.strip_prefix("DataFrame.").unwrap_or(name);
    let value = args.get("frame").ok_or("missing DataFrame receiver")?;
    match (operation, value) {
        ("to_list", Value::DataFrame(frame)) => Ok(Some(Materialization {
            frame: frame.as_ref().clone(),
            series: None,
            scalar: false,
        })),
        ("to_list", Value::Series(series)) => Ok(Some(Materialization {
            frame: series.frame.clone(),
            series: Some(series.as_ref().clone()),
            scalar: false,
        })),
        ("mean" | "sum" | "count" | "min" | "max", Value::Series(series)) => {
            let selected = series.frame.project_series("value", series)?;
            let aggregate =
                selected.aggregate(&BTreeMap::from([("value".into(), operation.into())]))?;
            let column = aggregate.column("value")?;
            Ok(Some(Materialization {
                frame: aggregate,
                series: Some(column),
                scalar: true,
            }))
        }
        _ => Ok(None),
    }
}

/// Named assigned columns are sugar for the same record argument. Timeline
/// controls remain ordinary controls, with no second evaluator or plan form.
pub(crate) fn named_assignments<T>(
    arguments: Vec<(Option<String>, T)>,
    pack: impl FnOnce(Vec<(String, T)>) -> T,
) -> Vec<(Option<String>, T)> {
    if arguments
        .iter()
        .any(|(name, _)| name.is_none() || name.as_deref() == Some("columns"))
    {
        return arguments;
    }
    let (columns, mut controls): (Vec<_>, Vec<_>) = arguments
        .into_iter()
        .partition(|(name, _)| !matches!(name.as_deref(), Some("duration" | "ease")));
    if !columns.is_empty() {
        controls.insert(
            0,
            (
                Some("columns".into()),
                pack(
                    columns
                        .into_iter()
                        .map(|(name, value)| (name.expect("named assignment"), value))
                        .collect(),
                ),
            ),
        )
    }
    controls
}
