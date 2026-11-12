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

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn sql_type(ty: &Type) -> Result<&'static str, String> {
    Ok(match ty {
        Type::Bool => "INTEGER",
        Type::String | Type::Fraction => "TEXT",
        Type::Numeric(_) => "ANY",
        Type::Option(inner) => return sql_type(inner),
        _ => return Err(format!("SQL columns require scalar data; found {ty}")),
    })
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
        Value::Data(value) => match value.as_ref() {
            serde_json::Value::Null => Sql::Null,
            serde_json::Value::Bool(v) => Sql::Integer(i64::from(*v)),
            serde_json::Value::String(v) => Sql::Text(v.clone()),
            serde_json::Value::Number(v) => {
                if let Some(i) = v.as_i64() {
                    Sql::Integer(i)
                } else {
                    Sql::Real(
                        v.as_f64()
                            .filter(|v| v.is_finite())
                            .ok_or("SQL numeric parameter must be finite")?,
                    )
                }
            }
            _ => return Err("SQL parameters must be scalar data".into()),
        },
        _ => return Err(format!("{} cannot be bound to SQL", value.ty())),
    })
}
fn output(value: ValueRef<'_>, ty: &Type, limits: Limits) -> Result<Value, String> {
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
        (Type::String, ValueRef::Text(v)) => Ok(Value::String(
            std::str::from_utf8(v).map_err(|e| e.to_string())?.into(),
        )),
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

pub fn query(
    sql: &str,
    tables: &Value,
    parameters: Option<&Value>,
    result: &Type,
    limits: Limits,
) -> Result<Value, String> {
    let Type::Table(row_type) = result else {
        return Err("SQL result requires Table<Row> or Table<Dict>".into());
    };
    if !matches!(row_type.as_ref(), Type::Record(_) | Type::Dict) {
        return Err("SQL result rows require a record or Dict".into());
    }
    let Value::Record(tables) = tables else {
        return Err("SQL tables require named table bindings".into());
    };
    if sql.len() > limits.max_bytes {
        return Err("SQL statement exceeds byte budget".into());
    }
    let mut db = Connection::open_in_memory().map_err(|e| e.to_string())?;
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
        let Value::Table { row_type, rows } = table else {
            return Err(format!("'{name}' must be a Table<Row>"));
        };
        let Type::Record(fields) = row_type.as_ref() else {
            return Err(format!("'{name}' requires a record row schema"));
        };
        if fields.is_empty() {
            return Err(format!("SQL table '{name}' needs at least one column"));
        }
        let definitions = fields
            .iter()
            .map(|(name, ty)| {
                Ok(format!(
                    "{} {}{}",
                    quote(name),
                    sql_type(ty)?,
                    if matches!(ty, Type::Option(_)) {
                        ""
                    } else {
                        " NOT NULL"
                    }
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        run(transaction.execute_batch(&format!(
            "CREATE TABLE {} ({}) STRICT",
            quote(name),
            definitions.join(",")
        )))?;
        let insert = format!(
            "INSERT INTO {} VALUES ({})",
            quote(name),
            vec!["?"; fields.len()].join(",")
        );
        let mut statement = transaction.prepare(&insert).map_err(|e| e.to_string())?;
        for row in rows.iter() {
            cells = cells
                .checked_sub(fields.len() + 1)
                .ok_or("SQL input value budget exceeded")?;
            let Value::Record(values) = row else {
                return Err(format!("SQL '{name}' row is not a record"));
            };
            let values = fields
                .keys()
                .map(|key| {
                    input(
                        values
                            .get(key)
                            .ok_or_else(|| format!("missing SQL input column '{key}'"))?,
                    )
                })
                .collect::<Result<Vec<_>, String>>()?;
            statement
                .execute(rusqlite::params_from_iter(&values))
                .map_err(|e| format!("SQL input '{name}': {e}"))?;
        }
    }
    transaction.commit().map_err(|e| e.to_string())?;
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
    let prepared = (|| -> Result<Value, String> {
        let mut statement = db.prepare(sql).map_err(|e| format!("SQL: {e}"))?;
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
        let parameters = match parameters {
            Some(Value::Record(fields)) => fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), input(v)?)))
                .collect::<Result<BTreeMap<_, _>, String>>()?,
            Some(Value::Dict(fields)) => fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), input(&Value::Data(v.clone().into()))?)))
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
                .ok_or_else(|| format!("missing SQL parameter :{name}"))?;
            statement
                .raw_bind_parameter(index, value)
                .map_err(|e| e.to_string())?;
            bound.insert(name);
        }
        if parameters.keys().any(|k| !bound.contains(k)) {
            return Err(
                "SQL params contains an unused parameter; remove it or reference :name".into(),
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
                    _ => {
                        unstructured.insert(name.clone(), dynamic(value)?);
                    }
                }
            }
            rows.push(if *row_type.as_ref() == Type::Dict {
                Value::Dict(unstructured.into_iter().collect())
            } else {
                Value::Record(fields.into())
            });
        }
        Ok(Value::Table {
            row_type: row_type.clone().into(),
            rows: rows.into(),
        })
    })();
    prepared.map_err(|error|if steps.load(Ordering::Relaxed)>limits.max_sql_steps{format!("SQL instruction budget {} exceeded; simplify the query, add a filter, or increase max_sql_steps",limits.max_sql_steps)}else{error})
}
