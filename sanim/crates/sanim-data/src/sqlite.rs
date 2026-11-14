//! Actual SQLite in-memory queries. No files, extensions, mutation, or ambient inputs.
use crate::{Limits, validate};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::{Value as Sql, ValueRef},
};
use sanim_core::{Fraction, Number, Type, Value, math};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) use sanim_core::sql::quote_identifier as quote;

fn sql_type(ty: &Type) -> Result<&'static str, String> {
    if Type::String.accepts(ty) {
        return Ok("TEXT");
    }
    Ok(match ty {
        Type::Bool => "INTEGER",
        Type::String | Type::Fraction => "TEXT",
        Type::Numeric(_) | Type::RefinedNumeric(_) => "ANY",
        Type::Option(inner) => return sql_type(inner),
        Type::Union(_) => {
            return Err(format!(
                "SQL scalar columns need one concrete type, found union {ty}; project alternatives into separate columns or query unstructured Dict rows"
            ));
        }
        _ => return Err(format!("SQL columns require scalar data; found {ty}")),
    })
}
/// The language checker and SQLite registration use the same supported row schemas.
pub fn validate_row_type(ty: &Type) -> Result<(), String> {
    match ty {
        Type::Record(fields) => {
            if fields.is_empty() { return Err("SQL record rows require at least one scalar column".into()); }
            for (name, ty) in fields { sql_type(ty).map_err(|error| format!("SQL column '{name}': {error}"))?; }
            Ok(())
        }
        Type::Dict => Ok(()),
        Type::Unknown => Err("SQL input needs a known element schema; annotate an empty list as List<Row>, for example let rows:List<Sample>=[]; an empty list still has length zero".into()),
        _ => sql_type(ty).map(|_| ()),
    }
}
fn input(value: &Value) -> Result<Sql, String> {
    Ok(match value {
        Value::None => Sql::Null,
        Value::Some(value) => return input(value),
        Value::Bool(value) => Sql::Integer(i64::from(*value)),
        Value::String(value) => Sql::Text(value.to_string()),
        Value::Number(value) => {
            if value.value.fract() == Fraction::from(0) {
                use num_traits::ToPrimitive;
                if let Some(integer) = value.value.to_i64() {
                    return Ok(Sql::Integer(integer));
                }
            }
            Sql::Real(math::to_f64(value.value)?)
        }
        Value::Fraction(value) => Sql::Text(value.to_string()),
        Value::Data(value) => return json_input(value),
        _ => return Err(format!("{} cannot be bound to SQL", value.ty())),
    })
}
fn json_input(value: &serde_json::Value) -> Result<Sql, String> {
    Ok(match value {
        serde_json::Value::Null => Sql::Null,
        serde_json::Value::Bool(value) => Sql::Integer(i64::from(*value)),
        serde_json::Value::String(value) => Sql::Text(value.clone()),
        serde_json::Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Sql::Integer(integer)
            } else {
                Sql::Real(value.as_f64().filter(|value| value.is_finite()).ok_or("SQL numeric parameter must be finite")?)
            }
        }
        _ => return Err("SQL values must be scalar data; nested objects and arrays require an explicit scalar projection".into()),
    })
}

fn columns(
    name: &str,
    items: &sanim_core::List,
) -> Result<Vec<(String, &'static str, bool)>, String> {
    validate_row_type(&items.element_type)
        .map_err(|error| format!("SQL input '{name}': {error}"))?;
    match items.element_type.as_ref() {
        Type::Record(fields) => fields.iter().map(|(name, ty)| Ok((name.clone(), sql_type(ty)?, matches!(ty, Type::Option(_))))).collect(),
        Type::Dict => {
            let mut names = BTreeSet::new();
            for (index, row) in items.iter().enumerate() {
                let Value::Dict(fields) = row else { return Err(format!("SQL input '{name}' row {index}: expected Dict, found {}", row.ty())); };
                names.extend(fields.keys().cloned());
            }
            if names.is_empty() {
                return Err(format!("SQL input '{name}' has no column schema: an empty List<Dict> or dictionaries without keys cannot declare SQL columns. Use List<Row> with a record type, or provide dictionary rows with scalar fields; len(...) remains valid for an empty list."));
            }
            Ok(names.into_iter().map(|name| (name, "ANY", true)).collect())
        }
        scalar => Ok(vec![("value".into(), sql_type(scalar).map_err(|_| format!("SQL input '{name}' requires scalar items, record rows or Dict rows; found List<{scalar}>. Project nested values into scalar columns first."))?, matches!(scalar, Type::Option(_)))]),
    }
}
fn row_values(
    name: &str,
    index: usize,
    item: &Value,
    ty: &Type,
    columns: &[(String, &'static str, bool)],
) -> Result<Vec<Sql>, String> {
    match (ty, item) {
        (Type::Record(_), Value::Record(fields)) => columns
            .iter()
            .map(|(column, _, _)| {
                input(fields.get(column).ok_or_else(|| {
                    format!("SQL input '{name}' row {index}: missing column '{column}'")
                })?)
            })
            .collect(),
        (Type::Dict, Value::Dict(fields)) => columns
            .iter()
            .map(|(column, _, _)| {
                fields
                    .get(column)
                    .map_or(Ok(Sql::Null), json_input)
                    .map_err(|error| {
                        format!("SQL input '{name}' row {index}, column '{column}': {error}")
                    })
            })
            .collect(),
        (Type::Record(_) | Type::Dict, _) => Err(format!(
            "SQL input '{name}' row {index}: expected {ty}, found {}",
            item.ty()
        )),
        _ => Ok(vec![input(item).map_err(|error| {
            format!("SQL input '{name}' item {index}: {error}")
        })?]),
    }
}

fn output(value: ValueRef<'_>, ty: &Type, limits: Limits) -> Result<Value, String> {
    if let Type::RefinedNumeric(domain) = ty {
        let value = output(value, &Type::Numeric(domain.unit), limits)?;
        sanim_core::validate_value(ty, &value)?;
        return Ok(value);
    }
    if let Type::Option(inner) = ty {
        return if value == ValueRef::Null {
            Ok(Value::None)
        } else {
            output(value, inner, limits).map(|v| Value::Some(std::sync::Arc::new(v)))
        };
    }
    match (ty, value) {
        (_, ValueRef::Null) => Err(format!(
            "SQL returned NULL for {ty}; use Option<{ty}> for nullable columns"
        )),
        (Type::Bool, ValueRef::Integer(v)) if matches!(v, 0 | 1) => Ok(Value::Bool(v == 1)),
        (ty, ValueRef::Text(v)) if Type::String.accepts(ty) => {
            let value = Value::String(std::str::from_utf8(v).map_err(|e| e.to_string())?.into());
            sanim_core::validate_value(ty, &value)?;
            Ok(value)
        }
        (Type::Numeric(unit), ValueRef::Integer(v)) => Ok(Value::Number(Number {
            value: Fraction::from(v),
            unit: *unit,
        })),
        (Type::Numeric(unit), ValueRef::Real(v)) => Ok(Value::Number(Number {
            value: math::from_f64(v)?,
            unit: *unit,
        })),
        (Type::Fraction, ValueRef::Text(v)) => validate(
            &serde_json::Value::String(std::str::from_utf8(v).map_err(|e| e.to_string())?.into()),
            ty,
            limits,
        ),
        _ => Err(format!(
            "SQL value {value:?} does not match {ty}; use CAST in SQL or change the result schema"
        )),
    }
}
fn dynamic(value: ValueRef<'_>) -> Result<serde_json::Value, String> {
    Ok(match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(v) => v.into(),
        ValueRef::Real(v) => serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .ok_or("SQL result must be finite")?,
        ValueRef::Text(v) => {
            serde_json::Value::String(std::str::from_utf8(v).map_err(|e| e.to_string())?.into())
        }
        ValueRef::Blob(_) => {
            return Err("SQL BLOB results are not supported; CAST to TEXT explicitly".into());
        }
    })
}

fn budget_error(error: String, limits: Limits, instructions_exceeded: bool) -> String {
    if instructions_exceeded || error.contains("interrupted") {
        format!(
            "SQL instruction budget {} exceeded; simplify the query, filter earlier, or increase max_sql_steps",
            limits.max_sql_steps
        )
    } else if error.contains("database or disk is full") {
        format!(
            "SQLite in-memory page budget {} bytes exceeded; reduce input data or increase max_bytes",
            limits.max_bytes
        )
    } else {
        error
    }
}

/// An immutable in-memory snapshot of input tables, reusable by read-only queries.
pub struct Database {
    db: Connection,
    limits: Limits,
    remaining_cells: usize,
    steps: Arc<AtomicU64>,
    import_steps: u64,
    pub bytes: usize,
}
impl Database {
    pub fn new(tables: &Value, limits: Limits) -> Result<Self, String> {
        Self::load(tables, limits).map_err(|error| budget_error(error, limits, false))
    }
    fn load(tables: &Value, limits: Limits) -> Result<Self, String> {
        let Value::Record(tables) = tables else {
            return Err("SQL tables require named table bindings".into());
        };
        let mut db = Connection::open_in_memory().map_err(|e| e.to_string())?;
        for config in [
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_DQS_DML,
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_DQS_DDL,
        ] {
            db.set_db_config(config, false)
                .map_err(|error| error.to_string())?;
        }
        let run = |result: rusqlite::Result<()>| result.map_err(|e| format!("SQLite: {e}"));
        run(db.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA page_size=4096;"))?;
        const PAGE_BYTES: usize = 4096;
        run(db.execute_batch(&format!(
            "PRAGMA max_page_count={};",
            (limits.max_bytes / PAGE_BYTES).max(1)
        )))?;
        for (limit, value) in [
            (Limit::SQLITE_LIMIT_LENGTH, limits.max_bytes),
            (Limit::SQLITE_LIMIT_SQL_LENGTH, limits.max_bytes),
            (Limit::SQLITE_LIMIT_EXPR_DEPTH, limits.max_depth),
            (Limit::SQLITE_LIMIT_COLUMN, limits.max_values.min(2000)),
            (Limit::SQLITE_LIMIT_ATTACHED, 0),
        ] {
            db.set_limit(limit, i32::try_from(value).unwrap_or(i32::MAX))
                .map_err(|e| e.to_string())?;
        }
        let steps = Arc::new(AtomicU64::new(0));
        let used = steps.clone();
        let max = limits.max_sql_steps;
        // One callback per instruction gives a deterministic hard work budget.
        db.progress_handler(1, Some(move || used.fetch_add(1, Ordering::Relaxed) >= max))
            .map_err(|e| e.to_string())?;
        let mut cells = limits.max_values;
        let transaction = db.transaction().map_err(|e| e.to_string())?;
        for (name, table) in tables {
            if name.to_ascii_lowercase().starts_with("sqlite_") || name.contains('\0') {
                return Err(format!("invalid SQL table name {name:?}"));
            }
            let Value::List(items) = table else {
                return Err(format!(
                    "SQL input '{name}' must be a List, found {}",
                    table.ty()
                ));
            };
            let columns = columns(name, items)?;
            if columns.is_empty() {
                return Err(format!(
                    "SQL input '{name}' needs at least one scalar column"
                ));
            }
            let definitions = columns
                .iter()
                .map(|(name, storage, nullable)| {
                    format!(
                        "{} {storage}{}",
                        quote(name),
                        if *nullable { "" } else { " NOT NULL" }
                    )
                })
                .collect::<Vec<_>>();
            run(transaction.execute_batch(&format!(
                "CREATE TABLE {} ({}) STRICT",
                quote(name),
                definitions.join(",")
            )))?;
            let insert = format!(
                "INSERT INTO {} VALUES ({})",
                quote(name),
                vec!["?"; columns.len()].join(",")
            );
            let mut statement = transaction
                .prepare(&insert)
                .map_err(|error| error.to_string())?;
            for (index, row) in items.iter().enumerate() {
                cells = cells
                    .checked_sub(columns.len() + 1)
                    .ok_or("SQL input value budget exceeded")?;
                let values = row_values(name, index, row, &items.element_type, &columns)?;
                statement
                    .execute(rusqlite::params_from_iter(&values))
                    .map_err(|error| format!("SQL input '{name}' row {index}: {error}"))?;
            }
        }
        transaction.commit().map_err(|e| e.to_string())?;
        let pages: i64 = db
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        let bytes = usize::try_from(pages)
            .unwrap_or(usize::MAX)
            .saturating_mul(PAGE_BYTES);
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
                Authorization::Allow
            }
            AuthAction::Function { function_name }
                if !matches!(
                    function_name.to_ascii_lowercase().as_str(),
                    "current_timestamp"
                        | "current_time"
                        | "current_date"
                        | "random"
                        | "randomblob"
                        | "load_extension"
                        | "date"
                        | "time"
                        | "datetime"
                        | "julianday"
                        | "unixepoch"
                        | "strftime"
                        | "timediff"
                ) =>
            {
                Authorization::Allow
            }
            _ => Authorization::Deny,
        }))
        .map_err(|e| e.to_string())?;
        let import_steps = steps.load(Ordering::Relaxed);
        Ok(Self {
            db,
            limits,
            remaining_cells: cells,
            steps,
            import_steps,
            bytes,
        })
    }
    pub fn query(
        &mut self,
        sql: &str,
        parameters: Option<&Value>,
        result: &Type,
    ) -> Result<Value, String> {
        let Type::List(row_type, _) = result else {
            return Err(
                "SQL result requires List<Row>, List<Dict>, or a list of scalar values".into(),
            );
        };
        validate_row_type(row_type)?;
        let limits = self.limits;
        if sql.len() > limits.max_bytes {
            return Err("SQL statement exceeds byte budget".into());
        }
        // Reusing imported tables must obey the same deterministic instruction
        // and cell budgets as executing the query against a fresh database.
        self.steps.store(self.import_steps, Ordering::Relaxed);
        let mut cells = self.remaining_cells;
        let db = &self.db;
        let steps = &self.steps;
        let prepared = (|| -> Result<Value, String> {
            let template_parameters = matches!(parameters,Some(Value::Record(fields)) if fields.keys().any(|name|name.starts_with(sanim_core::sql::PARAMETER_PREFIX)));
            let mut statement = db.prepare(sql).map_err(|error|if template_parameters{format!("SQL: {error}. Interpolation holes bind scalar values; use SqlIdent(name) for table/column names, and place value holes outside SQL quotes and comments")}else{format!("SQL: {error}")})?;
            if !statement.readonly() || statement.column_count() == 0 {
                return Err("query requires one read-only SELECT or WITH statement".into());
            }
            let columns = statement
                .column_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let unique = columns.iter().collect::<BTreeSet<_>>();
            if unique.len() != columns.len() {
                return Err(
                    "SQL result has duplicate column names; give each expression a unique AS alias"
                        .into(),
                );
            }
            if let Type::Record(fields) = row_type.as_ref()
                && fields.keys().collect::<BTreeSet<_>>() != unique
            {
                return Err(format!(
                    "SQL columns {columns:?} do not match result fields {:?}; use AS aliases",
                    fields.keys().collect::<Vec<_>>()
                ));
            }
            if !matches!(row_type.as_ref(), Type::Record(_) | Type::Dict) && columns.len() != 1 {
                return Err(format!(
                    "SQL result {result} requires exactly one scalar column, found {}; use List<Dict> or a record row type for multiple columns",
                    columns.len()
                ));
            }
            let parameters = match parameters {
                Some(Value::Record(fields)) => fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), input(v)?)))
                    .collect::<Result<BTreeMap<_, _>, String>>()?,
                Some(Value::Dict(fields)) => fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), json_input(v)?)))
                    .collect::<Result<_, String>>()?,
                None => BTreeMap::new(),
                _ => return Err("SQL params require Dict or a named record".into()),
            };
            let mut bound = BTreeSet::new();
            for index in 1..=statement.parameter_count() {
                let name = statement
                    .parameter_name(index)
                    .and_then(|n| n.strip_prefix(':'))
                    .ok_or("SQL parameters must use :name, not positional ? parameters")?
                    .to_string();
                let value = parameters
                    .get(&name)
                    .ok_or_else(|| format!("missing SQL parameter :{name}; use {{value}} interpolation in a Template instead of a separate params argument"))?;
                statement
                    .raw_bind_parameter(index, value)
                    .map_err(|e| e.to_string())?;
                bound.insert(name);
            }
            if parameters.keys().any(|k| !bound.contains(k)) {
                return Err(
                    "an SQL interpolation is not a parameter position (it may be inside quotes or a comment); write column={value} without SQL quotes, and use SqlIdent(name) for identifiers".into(),
                );
            }
            let mut cursor = statement.raw_query();
            let mut rows = vec![];
            let mut bytes = 0usize;
            while let Some(row) = cursor.next().map_err(|e| format!("SQL execution: {e}"))? {
                cells = cells
                    .checked_sub(columns.len() + 1)
                    .ok_or("SQL result value budget exceeded")?;
                let mut fields = BTreeMap::new();
                let mut unstructured = BTreeMap::new();
                for (i, name) in columns.iter().enumerate() {
                    let value = row.get_ref(i).map_err(|e| e.to_string())?;
                    let length = match value {
                        ValueRef::Text(v) | ValueRef::Blob(v) => v.len(),
                        _ => std::mem::size_of::<f64>(),
                    };
                    bytes = bytes
                        .checked_add(length + name.len())
                        .filter(|n| *n <= limits.max_bytes)
                        .ok_or("SQL result byte budget exceeded")?;
                    match row_type.as_ref() {
                        Type::Record(types) => {
                            fields.insert(
                                name.clone(),
                                output(value, &types[name], limits).map_err(|e| {
                                    format!("SQL row {}, column '{name}': {e}", rows.len())
                                })?,
                            );
                        }
                        Type::Dict => {
                            unstructured.insert(name.clone(), dynamic(value)?);
                        }
                        _ => {
                            fields.insert(
                                name.clone(),
                                output(value, row_type, limits).map_err(|error| {
                                    format!("SQL row {}, column '{name}': {error}", rows.len())
                                })?,
                            );
                        }
                    }
                }
                rows.push(if *row_type.as_ref() == Type::Dict {
                    Value::Dict(unstructured.into_iter().collect())
                } else if matches!(row_type.as_ref(), Type::Record(_)) {
                    Value::Record(fields.into())
                } else {
                    fields
                        .pop_first()
                        .expect("validated scalar SQL result column")
                        .1
                });
            }
            if let Type::List(_, length) = result
                && !length.allows(rows.len())
            {
                return Err(format!(
                    "expected SQL result {result}, found List with {} items",
                    rows.len()
                ));
            }
            Ok(Value::List(sanim_core::List::typed(
                row_type.as_ref().clone(),
                rows,
            )))
        })();
        prepared.map_err(|error| {
            budget_error(
                error,
                limits,
                steps.load(Ordering::Relaxed) > limits.max_sql_steps,
            )
        })
    }
}
pub fn query(
    sql: &str,
    tables: &Value,
    parameters: Option<&Value>,
    result: &Type,
    limits: Limits,
) -> Result<Value, String> {
    Database::new(tables, limits)?.query(sql, parameters, result)
}

/// Borrowed request shared by SQL and the DataFrame compiler.
pub struct Request<'a> {
    pub sql: &'a str,
    pub tables: &'a Value,
    pub parameters: Option<&'a Value>,
    pub result: &'a Type,
    pub limits: Limits,
}

#[derive(Debug)]
pub struct Execution {
    pub value: Value,
    pub reused_database: bool,
    /// Instructions actually executed for this request, including imports only
    /// when its database was new. The per-query hard budget still charges input.
    pub instructions: u64,
}

/// Imported SQLite snapshots use the same bounded LRU as other editor caches.
#[derive(Default)]
pub struct Databases {
    entries: sanim_core::cache::Cache<String, std::sync::Mutex<Database>>,
    capacity: (usize, usize),
}
impl Databases {
    pub fn configure(&mut self, bytes: usize, entries: usize) {
        if self.capacity != (bytes, entries) {
            self.entries = sanim_core::cache::Cache::new(bytes, entries);
            self.capacity = (bytes, entries);
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    /// input_key identifies table names, immutable row contents, and exact limits.
    pub fn query(&mut self, input_key: String, request: Request<'_>) -> Result<Execution, String> {
        let previous = self.entries.get(&input_key);
        let reused = previous.is_some();
        let database = if let Some(database) = previous {
            database
        } else {
            let database = Database::new(request.tables, request.limits)?;
            let bytes = database.bytes.saturating_add(input_key.len());
            let database = Arc::new(std::sync::Mutex::new(database));
            self.entries.insert(input_key, database.clone(), bytes);
            database
        };
        let mut database = database.lock().expect("SQLite database cache poisoned");
        if database.limits != request.limits {
            return Err("SQLite input cache key reused with different execution limits".into());
        }
        let value = database.query(request.sql, request.parameters, request.result)?;
        let instructions = database
            .steps
            .load(Ordering::Relaxed)
            .saturating_sub(if reused { database.import_steps } else { 0 });
        Ok(Execution {
            value,
            reused_database: reused,
            instructions,
        })
    }
}
