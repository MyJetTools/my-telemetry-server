use turso::{Row, Value};

use crate::db::turso_ext::*;

pub const TABLE_NAME: &str = "hour_statistics";

/// Byte-identical to the DDL my-sqlite generated.
pub const DDL: [&str; 1] = [
    "CREATE TABLE IF NOT EXISTS hour_statistics (hour_key bigint,app text,duration_micros bigint,amount bigint,
  PRIMARY KEY (hour_key,app))",
];

pub const COLUMNS: &str = "hour_key,app,duration_micros,amount";

/// `OR REPLACE` is what my-sqlite's `bulk_insert_or_update` compiled down to.
pub const INSERT_SQL: &str =
    "INSERT OR REPLACE INTO hour_statistics (hour_key,app,duration_micros,amount) VALUES (?,?,?,?)";

#[derive(Debug)]
pub struct HourStatisticsDto {
    pub hour_key: i64,
    pub app: String,
    pub duration_micros: i64,
    pub amount: i64,
}

impl HourStatisticsDto {
    pub fn from_row(row: &Row) -> Self {
        Self {
            hour_key: row.get_i64(0),
            app: row.get_string(1),
            duration_micros: row.get_i64(2),
            amount: row.get_i64(3),
        }
    }

    pub fn to_insert_params(&self) -> Vec<Value> {
        vec![
            Value::Integer(self.hour_key),
            as_text(self.app.as_str()),
            Value::Integer(self.duration_micros),
            Value::Integer(self.amount),
        ]
    }
}
