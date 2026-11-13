//! Compile typed relational plans into one bounded, parameterized SQLite query.
use crate::{Limits, sqlite::quote};
use sanim_core::{
    Type, Value,
    dataframe::{Expression, Frame, INDEX_COLUMN, Plan, Series},
};
use std::collections::BTreeMap;

pub struct Query {
    pub sql: String,
    pub tables: Value,
    pub parameters: Value,
    pub result: Type,
    pub input_fingerprint: String,
}

fn columns(frame: &Frame, alias: Option<&str>) -> Vec<String> {
    frame
        .schema
        .keys()
        .map(|name| alias.map_or_else(|| quote(name), |alias| format!("{alias}.{}", quote(name))))
        .collect()
}
struct Compiler {
    clauses: Vec<String>,
    compiled: BTreeMap<std::sync::Arc<str>, String>,
    tables: BTreeMap<String, Value>,
    parameters: BTreeMap<String, Value>,
    source_keys: Vec<String>,
    limit: usize,
    bytes: usize,
}
impl Compiler {
    fn expression(&mut self, expression: &Expression) -> Result<String, String> {
        Ok(match expression {
            Expression::Column(name) => quote(name),
            Expression::Literal(value) => {
                let name = format!("p{}", self.parameters.len());
                self.parameters.insert(name.clone(), value.clone());
                format!(":{name}")
            }
            Expression::Binary { op, left, right } => {
                let left = self.expression(left)?;
                let right = self.expression(right)?;
                match op.as_str() {
                    "+" | "-" | "*" => format!("({left} {op} {right})"),
                    "concat" => format!("({left} || {right})"),
                    "%" => {
                        format!("({left} - CAST(CAST({left} AS REAL)/{right} AS INTEGER)*{right})")
                    }
                    "/" => format!("(CAST({left} AS REAL) / {right})"),
                    "==" => format!("COALESCE(({left} = {right}),0)"),
                    "!=" | "<" | ">" | "<=" | ">=" => format!("COALESCE(({left} {op} {right}),0)"),
                    "and" | "or" => format!("COALESCE(({left} {} {right}),0)", op.to_uppercase()),
                    _ => {
                        return Err(format!(
                            "Series operator '{op}' is not supported by the SQLite DataFrame backend"
                        ));
                    }
                }
            }
            Expression::Unary { op, value } => {
                let value = self.expression(value)?;
                match op.as_str() {
                    "+" | "-" => format!("({op}{value})"),
                    "not" | "!" => format!("(NOT COALESCE({value},0))"),
                    _ => return Err(format!("unsupported Series unary operator '{op}'")),
                }
            }
            Expression::IsNull(value) => format!("({} IS NULL)", self.expression(value)?),
            Expression::FillNull { value, replacement } => format!(
                "COALESCE({}, {})",
                self.expression(value)?,
                self.expression(replacement)?
            ),
        })
    }
    fn frame(&mut self, frame: &Frame) -> Result<String, String> {
        let identity = frame.fingerprint.clone();
        if let Some(name) = self.compiled.get(&identity) {
            return Ok(name.clone());
        }
        if let Plan::Group { input } = frame.plan.as_ref() {
            let table = self.frame(input)?;
            self.compiled.insert(identity, table.clone());
            return Ok(table);
        }
        let index = quote(INDEX_COLUMN);
        let select = match frame.plan.as_ref() {
            Plan::Group { .. } => unreachable!("group view shares its input CTE"),
            Plan::Source(rows) => {
                let name = format!("input_{}", self.tables.len());
                self.tables.insert(name.clone(), Value::List(rows.clone()));
                self.source_keys.push(frame.fingerprint.to_string());
                let rowid=["rowid","_rowid_","oid"].into_iter().find(|alias|!frame.schema.keys().any(|name|name.eq_ignore_ascii_case(alias))).ok_or("DataFrame input shadows all SQLite rowid aliases (rowid, _rowid_, oid); rename one column to preserve stable row order")?;
                format!(
                    "SELECT {}, {rowid} AS {index} FROM {}",
                    columns(frame, None).join(","),
                    quote(&name)
                )
            }
            Plan::Project { input, columns } => {
                let table = self.frame(input)?;
                let mut projected = columns
                    .iter()
                    .map(|(name, expression)| {
                        Ok(format!(
                            "{} AS {}",
                            self.expression(expression)?,
                            quote(name)
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                projected.push(index.clone());
                format!("SELECT {} FROM {table}", projected.join(","))
            }
            Plan::Filter { input, condition } => {
                let table = self.frame(input)?;
                format!(
                    "SELECT * FROM {table} WHERE {}",
                    self.expression(condition)?
                )
            }
            Plan::Sort {
                input,
                columns: sorted,
                ascending,
            } => {
                let table = self.frame(input)?;
                let mut order = sorted
                    .iter()
                    .map(|name| {
                        format!(
                            "{} {} NULLS LAST",
                            quote(name),
                            if *ascending { "ASC" } else { "DESC" }
                        )
                    })
                    .collect::<Vec<_>>();
                order.push(index.clone());
                format!(
                    "SELECT {}, row_number() OVER(ORDER BY {}) AS {index} FROM {table}",
                    columns(input, None).join(","),
                    order.join(",")
                )
            }
            Plan::Limit { input, count, tail } => {
                let table = self.frame(input)?;
                // Counts are checked integers, never arbitrary SQL text.
                format!(
                    "SELECT * FROM {table} ORDER BY {index} {} LIMIT {count}",
                    if *tail { "DESC" } else { "ASC" }
                )
            }
            Plan::Aggregate {
                input,
                groups,
                columns: aggregated,
            } => {
                let table = self.frame(input)?;
                let mut projected = groups.iter().map(|name| quote(name)).collect::<Vec<_>>();
                for (output, column, operation) in aggregated {
                    let column = quote(column);
                    let expression = match operation.as_str() {
                        "size" => "COUNT(*)".to_owned(),
                        "count" => format!("COUNT({column})"),
                        "sum" => format!("COALESCE(SUM({column}), 0)"),
                        "mean" => format!("AVG({column})"),
                        "min" => format!("MIN({column})"),
                        "max" => format!("MAX({column})"),
                        _ => {
                            return Err(format!("unsupported DataFrame aggregation '{operation}'"));
                        }
                    };
                    projected.push(format!("{expression} AS {}", quote(output)));
                }
                let order = if groups.is_empty() {
                    "1".into()
                } else {
                    groups
                        .iter()
                        .map(|name| quote(name))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                projected.push(format!("row_number() OVER(ORDER BY {order}) AS {index}"));
                let grouped = if groups.is_empty() {
                    String::new()
                } else {
                    format!(
                        " WHERE {} GROUP BY {order}",
                        groups
                            .iter()
                            .map(|name| format!("{} IS NOT NULL", quote(name)))
                            .collect::<Vec<_>>()
                            .join(" AND ")
                    )
                };
                format!("SELECT {} FROM {table}{grouped}", projected.join(","))
            }
            Plan::Merge {
                left,
                right,
                on,
                how,
                columns: projected,
            } => {
                let a = self.frame(left)?;
                let b = self.frame(right)?;
                let mut fields = projected
                    .iter()
                    .map(|(output, side, name)| {
                        format!("{side}.{} AS {}", quote(name), quote(output))
                    })
                    .collect::<Vec<_>>();
                fields.push(format!(
                    "row_number() OVER(ORDER BY l.{index}, r.{index}) AS {index}"
                ));
                // Pandas matches missing join keys with other missing keys.
                let condition = on
                    .iter()
                    .map(|name| format!("l.{} IS r.{}", quote(name), quote(name)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                format!(
                    "SELECT {} FROM {a} AS l {} JOIN {b} AS r ON {condition}",
                    fields.join(","),
                    if how == "left" { "LEFT" } else { "INNER" }
                )
            }
        };
        self.bytes = self
            .bytes
            .checked_add(select.len())
            .filter(|bytes| *bytes <= self.limit)
            .ok_or("compiled DataFrame SQL exceeds the configured byte limit")?;
        let name = format!("q{}", self.clauses.len());
        self.clauses.push(format!("{name} AS ({select})"));
        self.compiled.insert(identity, name.clone());
        Ok(name)
    }
}

pub fn compile(frame: &Frame, series: Option<&Series>, limits: Limits) -> Result<Query, String> {
    if !frame.groups.is_empty() {
        return Err(
            "groupby creates a grouped DataFrame view; call agg/mean/sum/count before to_list()"
                .into(),
        );
    }
    let mut compiler = Compiler {
        clauses: Vec::new(),
        compiled: BTreeMap::new(),
        tables: Default::default(),
        parameters: Default::default(),
        source_keys: Vec::new(),
        limit: limits.max_bytes,
        bytes: 0,
    };
    let table = compiler.frame(frame)?;
    let (selection, result) = if let Some(series) = series {
        frame.same_rows(&series.frame)?;
        (
            format!("{} AS value", compiler.expression(&series.expression)?),
            Type::List(Box::new(series.ty.as_ref().clone())),
        )
    } else {
        (
            columns(frame, None).join(","),
            Type::List(Box::new(Type::Record(frame.schema.as_ref().clone()))),
        )
    };
    let sql = format!(
        "WITH {} SELECT {selection} FROM {table} ORDER BY {}",
        compiler.clauses.join(","),
        quote(INDEX_COLUMN)
    );
    if sql.len() > limits.max_bytes {
        return Err("compiled DataFrame SQL exceeds the configured byte limit".into());
    }
    Ok(Query {
        sql,
        tables: Value::Record(compiler.tables.into_iter().collect()),
        parameters: Value::Record(compiler.parameters.into_iter().collect()),
        result,
        input_fingerprint: sanim_core::digest(
            &serde_json::to_vec(&(compiler.source_keys, limits))
                .map_err(|error| error.to_string())?,
        ),
    })
}

pub fn collect(frame: &Frame, series: Option<&Series>, limits: Limits) -> Result<Value, String> {
    let query = compile(frame, series, limits)?;
    let value = crate::sqlite::query(
        &query.sql,
        &query.tables,
        Some(&query.parameters),
        &query.result,
        limits,
    )?;
    Ok(value)
}
