use turso::Value;

use crate::db::turso_ext::*;
use crate::db::{EventTagDto, MetricDto};

pub const TABLE_NAME: &str = "permanent_metrics";

/// Byte-identical to the DDL my-sqlite generated.
pub const DDL: [&str; 3] = [
    "CREATE TABLE IF NOT EXISTS permanent_metrics (id bigint,client_id text,started bigint,duration_micro bigint,name text,data text,success text,fail text,tags json,
  PRIMARY KEY (client_id,started,name,data))",
    "CREATE INDEX IF NOT EXISTS process_id_idx on permanent_metrics (id ASC)",
    "CREATE INDEX IF NOT EXISTS started_idx on permanent_metrics (started ASC)",
];

pub const INSERT_SQL: &str = "INSERT OR REPLACE INTO permanent_metrics (id,client_id,started,duration_micro,name,data,success,fail,tags) VALUES (?,?,?,?,?,?,?,?,?)";

#[derive(Debug)]
pub struct PermanentMetricDto {
    pub id: i64,
    pub client_id: String,
    pub started: i64,
    pub duration_micro: i64,
    pub name: String,
    pub data: String,
    pub success: Option<String>,
    pub fail: Option<String>,
    pub tags: Option<Vec<EventTagDto>>,
}

impl PermanentMetricDto {
    pub fn to_insert_params(&self) -> Vec<Value> {
        vec![
            Value::Integer(self.id),
            as_text(self.client_id.as_str()),
            Value::Integer(self.started),
            Value::Integer(self.duration_micro),
            as_text(self.name.as_str()),
            as_text(self.data.as_str()),
            as_opt_text(self.success.as_deref()),
            as_opt_text(self.fail.as_deref()),
            EventTagDto::to_db_json(self.tags.as_deref()),
        ]
    }
}

impl Into<PermanentMetricDto> for MetricDto {
    fn into(self) -> PermanentMetricDto {
        PermanentMetricDto {
            id: self.id,
            client_id: self.client_id.unwrap_or_default(),
            started: self.started,
            duration_micro: self.duration_micro,
            name: self.name,
            data: self.data,
            success: self.success,
            fail: self.fail,
            tags: self.tags,
        }
    }
}
