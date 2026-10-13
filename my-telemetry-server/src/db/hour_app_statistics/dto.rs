use turso::{Row, Value};

use crate::db::turso_ext::*;

pub const TABLE_NAME: &str = "statistics";

/// An action of a service has a row per product scope its metrics went to in the hour.
pub const DDL: [&str; 1] = [
    "CREATE TABLE IF NOT EXISTS statistics (hour_key bigint,scope text,service text,data_hashed text,data text,max bigint,min bigint,errors_amount bigint,success_amount bigint,sum_of_duration bigint,amount bigint,
  PRIMARY KEY (hour_key,scope,service,data_hashed))",
];

pub const COLUMNS: &str =
    "hour_key,scope,service,data_hashed,data,max,min,errors_amount,success_amount,sum_of_duration,amount";

pub const INSERT_SQL: &str = "INSERT OR REPLACE INTO statistics (hour_key,scope,service,data_hashed,data,max,min,errors_amount,success_amount,sum_of_duration,amount) VALUES (?,?,?,?,?,?,?,?,?,?,?)";

#[derive(Debug)]
pub struct HourAppDataStatisticsDto {
    pub hour_key: i64,
    /// The product scope the metrics of the action went to
    pub scope: String,
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
            scope: row.get_string(1),
            service: row.get_string(2),
            data_hashed: row.get_string(3),
            data: row.get_string(4),
            max: row.get_i64(5),
            min: row.get_i64(6),
            errors_amount: row.get_i64(7),
            success_amount: row.get_i64(8),
            sum_of_duration: row.get_i64(9),
            amount: row.get_i64(10),
        }
    }

    pub fn to_insert_params(&self) -> Vec<Value> {
        vec![
            Value::Integer(self.hour_key),
            as_text(self.scope.as_str()),
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
