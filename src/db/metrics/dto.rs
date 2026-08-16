use rust_extensions::date_time::DateTimeAsMicroseconds;
use serde_derive::{Deserialize, Serialize};
use turso::Value;

/// The in-memory model of one metric, used everywhere from ingest through the caches to
/// the reader API.
///
/// It is no longer a database row: metrics live in [`crate::storage_by_hour`], where the
/// on-disk form is `writer::TelemetryGrpcEvent`. The DDL, column list and row mapping that
/// used to sit here went with the `metrics` table.
#[derive(Debug, Clone)]
pub struct MetricDto {
    /// `ProcessId` - a correlation id, not unique.
    pub id: i64,
    pub started: i64,
    pub duration_micro: i64,
    pub name: String,
    pub data: String,
    pub success: Option<String>,
    pub fail: Option<String>,
    pub tags: Option<Vec<EventTagDto>>,
    pub client_id: Option<String>,
}

impl MetricDto {
    pub fn get_started(&self) -> DateTimeAsMicroseconds {
        DateTimeAsMicroseconds::new(self.started)
    }

    pub fn get_tag_value(&self, key: &str) -> Option<&str> {
        let tags = self.tags.as_ref()?;

        for itm in tags {
            if itm.key == key {
                return Some(&itm.value);
            }
        }

        None
    }

    pub fn remove_tag_value(&mut self, key: &str) -> Option<EventTagDto> {
        let tags = self.tags.as_mut()?;

        let index = tags.iter().position(|x| x.key == key)?;

        let result = tags.remove(index);

        Some(result)
    }

    pub fn update_user_id_to_client_id(&mut self, user_id_tag: &str, client_id_tag: &str) {
        if let Some(tags) = &mut self.tags {
            let index = tags.iter().position(|x| x.key == user_id_tag);

            if let Some(index) = index {
                let user_id = tags.remove(index);
                tags.push(EventTagDto {
                    key: client_id_tag.to_string(),
                    value: user_id.value,
                });
            }
        }
    }

    pub fn add_tag(&mut self, key: String, value: String) -> &str {
        if let Some(tags) = self.tags.as_mut() {
            tags.push(EventTagDto { key, value });
        } else {
            self.tags = Some(vec![EventTagDto { key, value }]);
        }

        self.tags.as_ref().unwrap().last().unwrap().value.as_str()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EventTagDto {
    pub key: String,
    pub value: String,
}

impl EventTagDto {
    /// Still used by `permanent_metrics`, which remains a turso table with a `tags json`
    /// column.
    pub fn from_db_json(src: Option<String>) -> Option<Vec<Self>> {
        let src = src?;
        serde_json::from_str(src.as_str()).ok()
    }

    pub fn to_db_json(tags: Option<&[Self]>) -> Value {
        match tags {
            Some(tags) => match serde_json::to_string(tags) {
                Ok(json) => Value::Text(json),
                Err(_) => Value::Null,
            },
            None => Value::Null,
        }
    }
}
