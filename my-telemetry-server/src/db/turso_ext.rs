use turso::{Builder, Connection, Database, Row, Value};

/// Opens (or creates) a database file and applies the DDL.
///
/// The DDL strings are kept byte-identical to what my-sqlite used to generate,
/// so files written before the move to turso keep opening, and a rollback to
/// my-sqlite would keep working too.
pub async fn open_db(file_name: &str, ddl: &[&str]) -> Database {
    let db = Builder::new_local(file_name)
        .build()
        .await
        .unwrap_or_else(|err| panic!("Can not open db file '{}'. Err: {:?}", file_name, err));

    let connection = db
        .connect()
        .unwrap_or_else(|err| panic!("Can not connect to '{}'. Err: {:?}", file_name, err));

    for sql in ddl {
        connection
            .execute(*sql, ())
            .await
            .unwrap_or_else(|err| panic!("Can not execute '{}'. Err: {:?}", sql, err));
    }

    db
}

/// Runs a batch of parameter sets through one prepared statement inside a single
/// transaction. This is the replacement for my-sqlite's `bulk_insert_*`.
pub async fn execute_batch(
    connection: &Connection,
    sql: &str,
    rows: impl Iterator<Item = Vec<Value>>,
) -> Result<(), turso::Error> {
    connection.execute("BEGIN", ()).await?;

    let result = execute_batch_inner(connection, sql, rows).await;

    if result.is_err() {
        let _ = connection.execute("ROLLBACK", ()).await;
        return result;
    }

    connection.execute("COMMIT", ()).await?;

    Ok(())
}

async fn execute_batch_inner(
    connection: &Connection,
    sql: &str,
    rows: impl Iterator<Item = Vec<Value>>,
) -> Result<(), turso::Error> {
    let mut statement = connection.prepare(sql).await?;

    for params in rows {
        statement
            .execute(turso::params::Params::Positional(params))
            .await?;
    }

    Ok(())
}

/// Typed access to a row by column index. The queries in this crate always list
/// their columns explicitly, so the index is fixed by the SELECT itself.
pub trait RowExt {
    fn get_i64(&self, index: usize) -> i64;
    fn get_string(&self, index: usize) -> String;
    fn get_opt_string(&self, index: usize) -> Option<String>;
}

impl RowExt for Row {
    fn get_i64(&self, index: usize) -> i64 {
        match self.get_value(index) {
            Ok(Value::Integer(value)) => value,
            Ok(Value::Real(value)) => value as i64,
            other => panic!("Column #{} is not an integer. Got: {:?}", index, other),
        }
    }

    fn get_string(&self, index: usize) -> String {
        self.get_opt_string(index).unwrap_or_default()
    }

    fn get_opt_string(&self, index: usize) -> Option<String> {
        match self.get_value(index) {
            Ok(Value::Text(value)) => Some(value),
            Ok(Value::Null) => None,
            other => panic!("Column #{} is not a text. Got: {:?}", index, other),
        }
    }
}

pub fn as_text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}

pub fn as_opt_text(value: Option<impl Into<String>>) -> Value {
    match value {
        Some(value) => Value::Text(value.into()),
        None => Value::Null,
    }
}
