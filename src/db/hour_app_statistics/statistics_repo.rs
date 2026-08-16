use rust_extensions::date_time::{HourKey, IntervalKey};
use turso::{params::Params, Database, Value};

use crate::db::turso_ext::*;

use super::dto::*;

pub struct HourAppDataStatisticsRepo {
    db: Database,
}

impl HourAppDataStatisticsRepo {
    pub async fn new(file_name: String) -> Self {
        Self {
            db: open_db(file_name.as_str(), &DDL).await,
        }
    }

    pub async fn update_metrics(&self, dto_s: &[HourAppDataStatisticsDto]) {
        if dto_s.is_empty() {
            return;
        }

        let connection = self.db.connect().unwrap();

        execute_batch(
            &connection,
            INSERT_SQL,
            dto_s.iter().map(|itm| itm.to_insert_params()),
        )
        .await
        .unwrap();
    }

    pub async fn get(&self, hour_key: IntervalKey<HourKey>) -> Vec<HourAppDataStatisticsDto> {
        // `#[order_by_desc]` sat on hour_key in the old DTO; spelled out here.
        let sql = format!(
            "select {} from {} where hour_key = ? order by hour_key desc",
            COLUMNS, TABLE_NAME
        );

        self.query(sql.as_str(), vec![Value::Integer(hour_key.to_i64())])
            .await
    }

    pub async fn get_by_app(
        &self,
        hour_key: IntervalKey<HourKey>,
        app: &str,
    ) -> Vec<HourAppDataStatisticsDto> {
        let sql = format!(
            "select {} from {} where hour_key = ? and service = ?",
            COLUMNS, TABLE_NAME
        );

        self.query(
            sql.as_str(),
            vec![Value::Integer(hour_key.to_i64()), as_text(app)],
        )
        .await
    }

    async fn query(&self, sql: &str, params: Vec<Value>) -> Vec<HourAppDataStatisticsDto> {
        let connection = self.db.connect().unwrap();

        let mut rows = connection
            .query(sql, Params::Positional(params))
            .await
            .unwrap();

        let mut result = Vec::new();

        while let Some(row) = rows.next().await.unwrap() {
            result.push(HourAppDataStatisticsDto::from_row(&row));
        }

        result
    }
}
