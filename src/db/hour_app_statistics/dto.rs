use turso::{Row, Value};

use crate::db::turso_ext::*;

pub const TABLE_NAME: &str = "statistics";

/// Byte-identical to the DDL my-sqlite generated.
pub const DDL: [&str; 1] = [
    "CREATE TABLE IF NOT EXISTS statistics (hour_key bigint,service text,data_hashed text,data text,max bigint,min bigint,errors_amount bigint,success_amount bigint,sum_of_duration bigint,amount bigint,
  PRIMARY KEY (hour_key,service,data_hashed))",
];

pub const COLUMNS: &str =
    "hour_key,service,data_hashed,data,max,min,errors_amount,success_amount,sum_of_duration,amount";

pub const INSERT_SQL: &str = "INSERT OR REPLACE INTO statistics (hour_key,service,data_hashed,data,max,min,errors_amount,success_amount,sum_of_duration,amount) VALUES (?,?,?,?,?,?,?,?,?,?)";

#[derive(Debug)]
pub struct HourAppDataStatisticsDto {
    pub hour_key: i64,
    pub service: String,
    pub data_hashed: String,
    pub data: String,
    pub max: i64,
    pub min: i64,
    pub errors_amount: i64,
    pub success_amount: i64,
    pub sum_of_duration: i64,
    pub amount: i64,
}

impl HourAppDataStatisticsDto {
    pub fn from_row(row: &Row) -> Self {
        Self {
            hour_key: row.get_i64(0),
            service: row.get_string(1),
            data_hashed: row.get_string(2),
            data: row.get_string(3),
            max: row.get_i64(4),
            min: row.get_i64(5),
            errors_amount: row.get_i64(6),
            success_amount: row.get_i64(7),
            sum_of_duration: row.get_i64(8),
            amount: row.get_i64(9),
        }
    }

    pub fn to_insert_params(&self) -> Vec<Value> {
        vec![
            Value::Integer(self.hour_key),
            as_text(self.service.as_str()),
            as_text(self.data_hashed.as_str()),
            as_text(self.data.as_str()),
            Value::Integer(self.max),
            Value::Integer(self.min),
            Value::Integer(self.errors_amount),
            Value::Integer(self.success_amount),
            Value::Integer(self.sum_of_duration),
            Value::Integer(self.amount),
        ]
    }
}
