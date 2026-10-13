use turso::Database;

use crate::db::turso_ext::*;

use super::dto::*;

pub struct PermanentMetricsRepo {
    db: Database,
}

impl PermanentMetricsRepo {
    pub async fn new(file_name: String) -> Self {
        Self {
            db: open_db(file_name.as_str(), &DDL).await,
        }
    }

    pub async fn insert(&self, dto: &[PermanentMetricDto]) {
        if dto.is_empty() {
            return;
        }

        let connection = self.db.connect().unwrap();

        execute_batch(
            &connection,
            INSERT_SQL,
            dto.iter().map(|itm| itm.to_insert_params()),
        )
        .await
        .unwrap();
    }
}
