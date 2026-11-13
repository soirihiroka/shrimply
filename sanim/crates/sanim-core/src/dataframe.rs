//! Typed, immutable relational plans. Rows stay in the existing persistent List.
//! This module builds plans; all row computation is performed by SQLite.
use crate::{List, Type, Unit, Value, digest, math::operations};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const MAX_PLAN_DEPTH: usize = 64;
pub const MAX_PLAN_NODES: usize = 4096;
pub const MAX_COLUMNS: usize = 1024;
pub const INDEX_COLUMN: &str = "__sanim_row_index";
pub const GROUPED_ROWS_ERROR: &str = "groupby creates a grouped view; aggregate it with agg(), mean(), sum(), count(), min(), or max() before indexing or materializing rows";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Expression {
    Column(String),
    Literal(Value),
    Binary {
        op: String,
        left: Arc<Expression>,
        right: Arc<Expression>,
    },
    Unary {
        op: String,
        value: Arc<Expression>,
    },
    IsNull(Arc<Expression>),
    FillNull {
        value: Arc<Expression>,
        replacement: Arc<Expression>,
    },
}

#[derive(Clone, Debug)]
pub enum Plan {
    Source(List),
    Group {
        input: Frame,
    },
    Project {
        input: Frame,
        columns: Vec<(String, Expression)>,
    },
    Filter {
        input: Frame,
        condition: Expression,
    },
    Sort {
        input: Frame,
        columns: Vec<String>,
        ascending: bool,
    },
    Limit {
        input: Frame,
        count: usize,
        tail: bool,
    },
    Aggregate {
        input: Frame,
        groups: Vec<String>,
        columns: Vec<(String, String, String)>,
    },
    Merge {
        left: Frame,
        right: Frame,
        on: Vec<String>,
        how: String,
        columns: Vec<(String, String, String)>,
    },
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub plan: Arc<Plan>,
    pub schema: Arc<BTreeMap<String, Type>>,
    pub groups: Arc<[String]>,
    pub fingerprint: Arc<str>,
    pub depth: usize,
    pub nodes: usize,
    pub metadata_bytes: usize,
    pub own_bytes: usize,
    pub own_items: usize,
    pub retained_bytes: usize,
    pub retained_items: usize,
}
impl PartialEq for Frame {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint && self.groups == other.groups
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub frame: Frame,
    pub expression: Arc<Expression>,
    pub ty: Arc<Type>,
    pub depth: usize,
    pub nodes: usize,
    pub expression_bytes: usize,
}

impl Serialize for Frame {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Description<'a> {
            schema: &'a BTreeMap<String, Type>,
            groups: &'a [String],
            fingerprint: &'a str,
            plan_nodes: usize,
        }
        Description {
            schema: &self.schema,
            groups: &self.groups,
            fingerprint: &self.fingerprint,
            plan_nodes: self.nodes,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Frame {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "DataFrame is an executable query plan; construct it from a typed List of rows instead of deserializing plan metadata",
        ))
    }
}

pub fn scalar_type(ty: &Type) -> bool {
    match ty {
        Type::Bool | Type::String | Type::Numeric(_) => true,
        Type::Option(inner) => {
            matches!(inner.as_ref(), Type::Bool | Type::String | Type::Numeric(_))
        }
        _ => false,
    }
}
pub fn validate_schema(schema: &BTreeMap<String, Type>) -> Result<(), String> {
    if schema.is_empty() || schema.len() > MAX_COLUMNS {
        return Err(format!(
            "DataFrame requires 1..={MAX_COLUMNS} scalar columns"
        ));
    }
    let mut names = BTreeSet::new();
    for (name, ty) in schema {
        if name.is_empty() || name.contains('\0') || name.eq_ignore_ascii_case(INDEX_COLUMN) {
            return Err(format!("invalid or reserved DataFrame column {name:?}"));
        }
        if !names.insert(name.to_lowercase()) {
            return Err(format!(
                "DataFrame column {name:?} differs only in case from another column; SQLite column names are case-insensitive"
            ));
        }
        if !scalar_type(ty) {
            return Err(format!(
                "DataFrame column '{name}' must contain Bool, String, Number, or Option of a scalar; found {ty}"
            ));
        }
    }
    Ok(())
}

pub fn unknown_column(schema: &BTreeMap<String, Type>, name: &str) -> String {
    format!(
        "DataFrame has no column '{name}'; available columns: {}",
        schema.keys().cloned().collect::<Vec<_>>().join(", ")
    )
}
pub fn selected_schema(
    schema: &BTreeMap<String, Type>,
    names: &[String],
) -> Result<BTreeMap<String, Type>, String> {
    let mut output = BTreeMap::new();
    for name in names {
        let ty = schema
            .get(name)
            .ok_or_else(|| unknown_column(schema, name))?;
        if output.insert(name.clone(), ty.clone()).is_some() {
            return Err(format!("DataFrame selection repeats column '{name}'"));
        }
    }
    validate_schema(&output)?;
    Ok(output)
}
pub fn renamed_schema(
    schema: &BTreeMap<String, Type>,
    names: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Type>, String> {
    for name in names.keys() {
        if !schema.contains_key(name) {
            return Err(unknown_column(schema, name));
        }
    }
    let mut output = BTreeMap::new();
    for (name, ty) in schema {
        let name = names.get(name).unwrap_or(name).clone();
        if output.insert(name.clone(), ty.clone()).is_some() {
            return Err(format!("rename creates duplicate column '{name}'"));
        }
    }
    validate_schema(&output)?;
    Ok(output)
}
pub fn aggregated_schema(
    schema: &BTreeMap<String, Type>,
    groups: &[String],
    operations: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Type>, String> {
    if operations.is_empty() {
        return Err("agg requires at least one column aggregation".into());
    }
    let mut output = if groups.is_empty() {
        BTreeMap::new()
    } else {
        selected_schema(schema, groups)?
    };
    for (name, operation) in operations {
        let ty = schema
            .get(name)
            .ok_or_else(|| unknown_column(schema, name))?;
        if output.contains_key(name) {
            return Err(format!(
                "aggregation output '{name}' conflicts with a group key"
            ));
        }
        output.insert(name.clone(), aggregate_type(operation, ty)?);
    }
    validate_schema(&output)?;
    Ok(output)
}
pub fn assigned_schema(
    schema: &BTreeMap<String, Type>,
    values: &BTreeMap<String, Type>,
) -> Result<BTreeMap<String, Type>, String> {
    let mut output = schema.clone();
    for (name, ty) in values {
        let ty = if let Type::Series(inner) = ty {
            inner.as_ref()
        } else {
            ty
        };
        if !scalar_type(ty) {
            return Err(format!(
                "assigned DataFrame column '{name}' needs a scalar or Series, found {ty}"
            ));
        }
        output.insert(name.clone(), ty.clone());
    }
    validate_schema(&output)?;
    Ok(output)
}
pub type MergeColumns = Vec<(String, String, String)>;
pub fn merged_schema(
    left: &BTreeMap<String, Type>,
    right: &BTreeMap<String, Type>,
    on: &[String],
    how: &str,
    suffixes: [&str; 2],
) -> Result<(BTreeMap<String, Type>, MergeColumns), String> {
    if on.is_empty() || !matches!(how, "inner" | "left") {
        return Err("merge requires nonempty join columns and how inner or left".into());
    }
    for key in on {
        let a = left.get(key).ok_or_else(|| unknown_column(left, key))?;
        let b = right.get(key).ok_or_else(|| unknown_column(right, key))?;
        if a != b {
            return Err(format!(
                "merge key '{key}' has incompatible types {a} and {b}"
            ));
        }
    }
    let mut schema = BTreeMap::new();
    let mut columns = Vec::new();
    for (side, fields) in [("l", left), ("r", right)] {
        for (name, ty) in fields {
            if side == "r" && on.contains(name) {
                continue;
            }
            let output =
                if !on.contains(name) && left.contains_key(name) && right.contains_key(name) {
                    format!("{name}{}", suffixes[usize::from(side == "r")])
                } else {
                    name.clone()
                };
            let ty = if side == "r" && how == "left" && !matches!(ty, Type::Option(_)) {
                Type::Option(Box::new(ty.clone()))
            } else {
                ty.clone()
            };
            if schema.insert(output.clone(), ty).is_some() {
                return Err(format!(
                    "merge creates duplicate column '{output}'; choose distinct suffixes"
                ));
            }
            columns.push((output, side.into(), name.clone()));
        }
    }
    validate_schema(&schema)?;
    Ok((schema, columns))
}

impl Frame {
    pub fn new(rows: List) -> Result<Self, String> {
        let Type::Record(schema) = rows.element_type.as_ref() else {
            return Err(format!(
                "DataFrame requires List<Row> with a known record schema, found List<{}>; annotate empty rows explicitly",
                rows.element_type
            ));
        };
        validate_schema(schema)?;
        let schema = Arc::new(schema.clone());
        let value = Value::List(rows.clone());
        let mut bytes = usize::MAX;
        let mut items = usize::MAX;
        crate::value_budget::charge(&value, &mut bytes, &mut items, MAX_PLAN_DEPTH)?;
        let source_bytes = usize::MAX - bytes;
        let source_items = usize::MAX - items;
        let fingerprint = digest(&serde_json::to_vec(&value).map_err(|error| error.to_string())?);
        let metadata_bytes = schema_bytes(&schema)?
            .saturating_add(std::mem::size_of::<Self>() + std::mem::size_of::<Plan>());
        Ok(Self {
            plan: Arc::new(Plan::Source(rows)),
            schema,
            groups: Arc::new([]),
            fingerprint: fingerprint.into(),
            depth: 1,
            nodes: 1,
            metadata_bytes,
            own_bytes: metadata_bytes.saturating_add(source_bytes),
            own_items: source_items + 1,
            retained_bytes: metadata_bytes.saturating_add(source_bytes),
            retained_items: source_items + 1,
        })
    }
    fn derived(
        &self,
        plan: Plan,
        schema: BTreeMap<String, Type>,
        description: impl Serialize,
        extra: Option<&Frame>,
    ) -> Result<Self, String> {
        validate_schema(&schema)?;
        let depth = self.depth.max(extra.map_or(0, |frame| frame.depth)) + 1;
        let expression_nodes = plan.expression_nodes()?;
        let nodes = self
            .nodes
            .checked_add(extra.map_or(0, |frame| frame.nodes))
            .and_then(|nodes| nodes.checked_add(1 + expression_nodes))
            .ok_or("DataFrame plan size overflow")?;
        if depth > MAX_PLAN_DEPTH || nodes > MAX_PLAN_NODES {
            return Err(format!(
                "DataFrame query plan exceeds {MAX_PLAN_DEPTH} levels or {MAX_PLAN_NODES} nodes; materialize an intermediate result with to_list()"
            ));
        }
        let encoded = serde_json::to_vec(&(
            &self.fingerprint,
            extra.map(|frame| &frame.fingerprint),
            description,
        ))
        .map_err(|error| error.to_string())?;
        let metadata_bytes = schema_bytes(&schema)?.saturating_add(
            std::mem::size_of::<Self>() + std::mem::size_of::<Plan>() + encoded.len(),
        );
        let fingerprint = digest(&encoded);
        Ok(Self {
            plan: Arc::new(plan),
            schema: Arc::new(schema),
            groups: Arc::new([]),
            fingerprint: fingerprint.into(),
            depth,
            nodes,
            metadata_bytes,
            own_bytes: metadata_bytes,
            own_items: 1 + expression_nodes,
            retained_bytes: self
                .retained_bytes
                .saturating_add(
                    extra
                        .filter(|frame| frame.fingerprint != self.fingerprint)
                        .map_or(0, |frame| frame.retained_bytes),
                )
                .saturating_add(metadata_bytes),
            retained_items: self
                .retained_items
                .saturating_add(
                    extra
                        .filter(|frame| frame.fingerprint != self.fingerprint)
                        .map_or(0, |frame| frame.retained_items),
                )
                .saturating_add(1 + expression_nodes),
        })
    }
    pub fn ty(&self) -> Type {
        Type::DataFrame {
            row: Box::new(Type::Record(self.schema.as_ref().clone())),
            groups: self.groups.to_vec(),
        }
    }
    pub fn column(&self, name: &str) -> Result<Series, String> {
        if !self.groups.is_empty() {
            return Err(GROUPED_ROWS_ERROR.into());
        }
        let ty = self
            .schema
            .get(name)
            .ok_or_else(|| self.unknown_column(name))?;
        Ok(Series {
            frame: self.clone(),
            expression: Arc::new(Expression::Column(name.into())),
            ty: Arc::new(ty.clone()),
            depth: 1,
            nodes: 1,
            expression_bytes: std::mem::size_of::<Expression>() + name.len(),
        })
    }
    pub fn unknown_column(&self, name: &str) -> String {
        unknown_column(&self.schema, name)
    }
    pub fn select(&self, names: &[String]) -> Result<Self, String> {
        let schema = selected_schema(&self.schema, names)?;
        let columns = names
            .iter()
            .map(|name| (name.clone(), Expression::Column(name.clone())))
            .collect();
        self.derived(
            Plan::Project {
                input: self.clone(),
                columns,
            },
            schema,
            ("select", names),
            None,
        )
    }
    pub fn filter(&self, condition: &Series) -> Result<Self, String> {
        self.same_rows(&condition.frame)?;
        if condition.ty.as_ref() != &Type::Bool {
            return Err(format!(
                "DataFrame row selection requires Series<Bool>, found Series<{}>; use a comparison or isna()",
                condition.ty
            ));
        }
        self.derived(
            Plan::Filter {
                input: self.clone(),
                condition: condition.expression.as_ref().clone(),
            },
            self.schema.as_ref().clone(),
            ("filter", &condition.expression),
            None,
        )
    }
    pub fn same_rows(&self, other: &Frame) -> Result<(), String> {
        if self.fingerprint != other.fingerprint {
            return Err("Series belong to different DataFrame row plans; build the mask/assigned column from this exact frame or merge the frames explicitly".into());
        }
        Ok(())
    }
    pub fn limit(&self, count: usize, tail: bool) -> Result<Self, String> {
        self.derived(
            Plan::Limit {
                input: self.clone(),
                count,
                tail,
            },
            self.schema.as_ref().clone(),
            ("limit", count, tail),
            None,
        )
    }
    pub fn sort(&self, columns: Vec<String>, ascending: bool) -> Result<Self, String> {
        if columns.is_empty() {
            return Err("sort_values requires at least one column".into());
        }
        for name in &columns {
            if !self.schema.contains_key(name) {
                return Err(self.unknown_column(name));
            }
        }
        self.derived(
            Plan::Sort {
                input: self.clone(),
                columns: columns.clone(),
                ascending,
            },
            self.schema.as_ref().clone(),
            ("sort", columns, ascending),
            None,
        )
    }
    pub fn drop_columns(&self, columns: &[String]) -> Result<Self, String> {
        for name in columns {
            if !self.schema.contains_key(name) {
                return Err(self.unknown_column(name));
            }
        }
        self.select(
            &self
                .schema
                .keys()
                .filter(|name| !columns.contains(name))
                .cloned()
                .collect::<Vec<_>>(),
        )
    }
    pub fn rename(&self, names: &BTreeMap<String, String>) -> Result<Self, String> {
        let schema = renamed_schema(&self.schema, names)?;
        let columns = self
            .schema
            .keys()
            .map(|name| {
                (
                    names.get(name).unwrap_or(name).clone(),
                    Expression::Column(name.clone()),
                )
            })
            .collect();
        self.derived(
            Plan::Project {
                input: self.clone(),
                columns,
            },
            schema,
            ("rename", names),
            None,
        )
    }
    pub fn assign(&self, values: &BTreeMap<String, Value>) -> Result<Self, String> {
        let schema = assigned_schema(
            &self.schema,
            &values
                .iter()
                .map(|(name, value)| (name.clone(), value.ty()))
                .collect(),
        )?;
        let mut columns: BTreeMap<_, _> = self
            .schema
            .keys()
            .map(|name| (name.clone(), Expression::Column(name.clone())))
            .collect();
        for (name, value) in values {
            let (expression, _) = self.expression(value)?;
            columns.insert(name.clone(), expression);
        }
        self.derived(
            Plan::Project {
                input: self.clone(),
                columns: columns.into_iter().collect(),
            },
            schema,
            ("assign", values),
            None,
        )
    }
    pub fn project_series(&self, name: &str, series: &Series) -> Result<Self, String> {
        self.same_rows(&series.frame)?;
        self.derived(
            Plan::Project {
                input: self.clone(),
                columns: vec![(name.into(), series.expression.as_ref().clone())],
            },
            BTreeMap::from([(name.into(), series.ty.as_ref().clone())]),
            ("series", name, &series.expression),
            None,
        )
    }
    pub fn expression(&self, value: &Value) -> Result<(Expression, Type), String> {
        if let Value::Series(series) = value {
            self.same_rows(&series.frame)?;
            Ok((
                series.expression.as_ref().clone(),
                series.ty.as_ref().clone(),
            ))
        } else if scalar_type(&value.ty()) {
            Ok((Expression::Literal(value.clone()), value.ty()))
        } else {
            Err(format!(
                "DataFrame expressions require a scalar or Series, found {}",
                value.ty()
            ))
        }
    }
    pub fn groupby(&self, keys: Vec<String>) -> Result<Self, String> {
        if keys.is_empty() {
            return Err("groupby requires at least one column".into());
        }
        for key in &keys {
            if !self.schema.contains_key(key) {
                return Err(self.unknown_column(key));
            }
        }
        let mut grouped = self.derived(
            Plan::Group {
                input: self.clone(),
            },
            self.schema.as_ref().clone(),
            ("groupby", &keys),
            None,
        )?;
        grouped.groups = keys.into();
        Ok(grouped)
    }
    pub fn aggregate(&self, operations: &BTreeMap<String, String>) -> Result<Self, String> {
        let schema = aggregated_schema(&self.schema, &self.groups, operations)?;
        let columns = operations
            .iter()
            .map(|(name, operation)| (name.clone(), name.clone(), operation.clone()))
            .collect();
        self.derived(
            Plan::Aggregate {
                input: self.clone(),
                groups: self.groups.to_vec(),
                columns,
            },
            schema,
            ("agg", operations, &self.groups),
            None,
        )
    }
    pub fn merge(
        &self,
        right: &Frame,
        on: Vec<String>,
        how: &str,
        suffixes: [&str; 2],
    ) -> Result<Self, String> {
        let (schema, columns) = merged_schema(&self.schema, &right.schema, &on, how, suffixes)?;
        self.derived(
            Plan::Merge {
                left: self.clone(),
                right: right.clone(),
                on: on.clone(),
                how: how.into(),
                columns,
            },
            schema,
            ("merge", on, how, suffixes),
            Some(right),
        )
    }
}

pub fn aggregate_type(operation: &str, ty: &Type) -> Result<Type, String> {
    let inner = if let Type::Option(inner) = ty {
        inner.as_ref()
    } else {
        ty
    };
    match operation {
        "count" | "size" => Ok(Type::Numeric(Unit::Number)),
        "mean" | "sum" if matches!(inner, Type::Numeric(_)) => Ok(if operation == "sum" {
            inner.clone()
        } else {
            Type::Option(Box::new(inner.clone()))
        }),
        "min" | "max" if scalar_type(inner) => Ok(Type::Option(Box::new(inner.clone()))),
        _ => Err(format!(
            "aggregation '{operation}' is not supported for {ty}; use count/size, mean/sum for numeric columns, or min/max for scalar columns"
        )),
    }
}

pub fn default_aggregations(
    schema: &BTreeMap<String, Type>,
    groups: &[String],
    operation: &str,
) -> Result<BTreeMap<String, String>, String> {
    let columns = schema
        .iter()
        .filter(|(name, ty)| !groups.contains(name) && aggregate_type(operation, ty).is_ok())
        .map(|(name, _)| (name.clone(), operation.into()))
        .collect::<BTreeMap<_, _>>();
    if columns.is_empty() {
        return Err(format!("{operation} found no compatible non-group columns"));
    }
    Ok(columns)
}

impl Series {
    pub fn binary(op: &str, left: Value, right: Value) -> Result<Self, String> {
        let frame = match (&left, &right) {
            (Value::Series(series), _) | (_, Value::Series(series)) => series.frame.clone(),
            _ => return Err("a Series expression needs at least one column".into()),
        };
        let (depth, nodes) = expression_size(&[&left, &right])?;
        let expression_bytes = expression_bytes(&[&left, &right])?.saturating_add(op.len());
        let (left, a) = frame.expression(&left)?;
        let (right, b) = frame.expression(&right)?;
        let ty = binary_type(op, &a, &b)?;
        let op = if op == "+" && matches!(nonnullable(&a), Type::String) {
            "concat"
        } else {
            op
        };
        Ok(Self {
            frame,
            expression: Arc::new(Expression::Binary {
                op: op.into(),
                left: Arc::new(left),
                right: Arc::new(right),
            }),
            ty: Arc::new(ty),
            depth,
            nodes,
            expression_bytes,
        })
    }
    pub fn unary(&self, op: &str) -> Result<Self, String> {
        let ty = unary_type(op, &self.ty)?;
        let (depth, nodes) = expression_size(&[&Value::Series(Arc::new(self.clone()))])?;
        let expression_bytes = self
            .expression_bytes
            .saturating_add(std::mem::size_of::<Expression>());
        Ok(Self {
            frame: self.frame.clone(),
            expression: Arc::new(Expression::Unary {
                op: op.into(),
                value: self.expression.clone(),
            }),
            ty: Arc::new(ty),
            depth,
            nodes,
            expression_bytes,
        })
    }
    pub fn isna(&self) -> Result<Self, String> {
        let (depth, nodes) = expression_size(&[&Value::Series(Arc::new(self.clone()))])?;
        let expression_bytes = self
            .expression_bytes
            .saturating_add(std::mem::size_of::<Expression>());
        Ok(Self {
            frame: self.frame.clone(),
            expression: Arc::new(Expression::IsNull(self.expression.clone())),
            ty: Arc::new(Type::Bool),
            depth,
            nodes,
            expression_bytes,
        })
    }
    pub fn fillna(&self, replacement: &Value) -> Result<Self, String> {
        let inner = if let Type::Option(inner) = self.ty.as_ref() {
            inner.as_ref()
        } else {
            self.ty.as_ref()
        };
        let (depth, nodes) =
            expression_size(&[&Value::Series(Arc::new(self.clone())), replacement])?;
        let expression_bytes =
            expression_bytes(&[&Value::Series(Arc::new(self.clone())), replacement])?;
        let (replacement, ty) = self.frame.expression(replacement)?;
        if !inner.accepts(&ty) {
            return Err(format!("fillna replacement must be {inner}, found {ty}"));
        }
        Ok(Self {
            frame: self.frame.clone(),
            expression: Arc::new(Expression::FillNull {
                value: self.expression.clone(),
                replacement: Arc::new(replacement),
            }),
            ty: Arc::new(inner.clone()),
            depth,
            nodes,
            expression_bytes,
        })
    }
}

fn schema_bytes(schema: &BTreeMap<String, Type>) -> Result<usize, String> {
    let mut remaining = usize::MAX;
    for (name, ty) in schema {
        crate::type_budget::charge(ty, &mut remaining)?;
        remaining = remaining
            .checked_sub(name.len() + std::mem::size_of::<String>())
            .ok_or("DataFrame schema byte size overflow")?;
    }
    Ok(usize::MAX - remaining)
}
fn expression_size(values: &[&Value]) -> Result<(usize, usize), String> {
    let depth = values
        .iter()
        .map(|value| {
            if let Value::Series(series) = value {
                series.depth
            } else {
                1
            }
        })
        .max()
        .unwrap_or(0)
        + 1;
    let nodes = values.iter().try_fold(1usize, |nodes, value| {
        nodes
            .checked_add(if let Value::Series(series) = value {
                series.nodes
            } else {
                1
            })
            .ok_or("Series expression size overflow")
    })?;
    if depth > MAX_PLAN_DEPTH || nodes > MAX_PLAN_NODES {
        return Err(format!(
            "Series expression exceeds {MAX_PLAN_DEPTH} levels or {MAX_PLAN_NODES} nodes; materialize an intermediate DataFrame"
        ));
    }
    Ok((depth, nodes))
}

fn nonnullable(ty: &Type) -> &Type {
    if let Type::Option(inner) = ty {
        inner
    } else {
        ty
    }
}
/// Null arithmetic stays nullable; comparisons use false for missing values,
/// making row masks total. Missingness itself is tested explicitly with isna().
pub fn binary_type(op: &str, left: &Type, right: &Type) -> Result<Type, String> {
    if !matches!(
        op,
        "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "and" | "or"
    ) {
        return Err(format!(
            "Series operator '{op}' is not supported by the SQLite DataFrame backend"
        ));
    }
    let ty = operations::binary_type(op, nonnullable(left), nonnullable(right))?;
    if matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=" | "and" | "or") {
        Ok(Type::Bool)
    } else if matches!(left, Type::Option(_)) || matches!(right, Type::Option(_)) {
        Ok(Type::Option(Box::new(ty)))
    } else {
        Ok(ty)
    }
}
pub fn unary_type(op: &str, ty: &Type) -> Result<Type, String> {
    let result = operations::unary_type(op, nonnullable(ty))?;
    if matches!(op, "not" | "!") {
        Ok(Type::Bool)
    } else if matches!(ty, Type::Option(_)) {
        Ok(Type::Option(Box::new(result)))
    } else {
        Ok(result)
    }
}
fn expression_bytes(values: &[&Value]) -> Result<usize, String> {
    let mut bytes = std::mem::size_of::<Expression>();
    for value in values {
        let size = if let Value::Series(series) = value {
            series.expression_bytes
        } else {
            let mut remaining = usize::MAX;
            let mut items = usize::MAX;
            crate::value_budget::charge(value, &mut remaining, &mut items, MAX_PLAN_DEPTH)?;
            usize::MAX - remaining
        };
        bytes = bytes
            .checked_add(size)
            .ok_or("Series expression byte size overflow")?;
    }
    Ok(bytes)
}
impl Plan {
    fn expression_nodes(&self) -> Result<usize, String> {
        fn count(expression: &Expression) -> Result<usize, String> {
            let children = match expression {
                Expression::Column(_) | Expression::Literal(_) => 0,
                Expression::Binary { left, right, .. }
                | Expression::FillNull {
                    value: left,
                    replacement: right,
                } => count(left)?
                    .checked_add(count(right)?)
                    .ok_or("DataFrame expression size overflow")?,
                Expression::Unary { value, .. } | Expression::IsNull(value) => count(value)?,
            };
            children.checked_add(1).filter(|size|*size<=MAX_PLAN_NODES).ok_or_else(||format!("DataFrame expression exceeds {MAX_PLAN_NODES} nodes; materialize an intermediate result"))
        }
        let result = match self {
            Self::Project { columns, .. } => {
                columns.iter().try_fold(0usize, |size, (_, expression)| {
                    size.checked_add(count(expression)?)
                        .filter(|size| *size <= MAX_PLAN_NODES)
                        .ok_or_else(|| {
                            "DataFrame project expressions exceed node budget".to_owned()
                        })
                })?
            }
            Self::Filter { condition, .. } => count(condition)?,
            _ => 0,
        };
        Ok(result)
    }
}
impl Frame {
    /// Charge each retained plan/source once. The persistent set belongs to an
    /// execution checkpoint; this never scans shared input rows during chaining.
    pub fn retain(
        &self,
        seen: &mut imbl::OrdSet<String>,
        bytes: &mut usize,
        items: &mut usize,
    ) -> Result<(), String> {
        if seen.contains(self.fingerprint.as_ref()) {
            return Ok(());
        }
        *bytes = bytes
            .checked_sub(self.own_bytes)
            .ok_or("retained DataFrame byte budget exceeded")?;
        *items = items
            .checked_sub(self.own_items)
            .ok_or("retained DataFrame value budget exceeded")?;
        seen.insert(self.fingerprint.to_string());
        match self.plan.as_ref() {
            Plan::Source(_) => {}
            Plan::Merge { left, right, .. } => {
                left.retain(seen, bytes, items)?;
                right.retain(seen, bytes, items)?
            }
            Plan::Group { input }
            | Plan::Project { input, .. }
            | Plan::Filter { input, .. }
            | Plan::Sort { input, .. }
            | Plan::Limit { input, .. }
            | Plan::Aggregate { input, .. } => input.retain(seen, bytes, items)?,
        }
        Ok(())
    }
}
