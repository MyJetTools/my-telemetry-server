use rust_extensions::date_time::{HourKey, IntervalKey};
use turso::{params::Params, Database, Value};

use crate::db::turso_ext::*;

use super::dto::*;

pub struct HourStatisticsRepo {
    db: Database,
}

impl HourStatisticsRepo {
    pub async fn new(file_name: String) -> Self {
        println!("Creating HourStatisticsRepo with file_name: {}", file_name);
        Self {
            db: open_db(file_name.as_str(), &DDL).await,
        }
    }

    pub async fn update(&self, dto_s: &[HourStatisticsDto]) {
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

    /// The services of the hour in every product scope
    pub async fn get(&self, hour: IntervalKey<HourKey>) -> Vec<HourStatisticsDto> {
        let sql = format!("select {} from {} where hour_key = ?", COLUMNS, TABLE_NAME);

        self.query(sql.as_str(), vec![Value::Integer(hour.to_i64())])
            .await
    }

    /// The services of the hour in the product scope
    pub async fn get_by_scope(
        &self,
        hour: IntervalKey<HourKey>,
        scope: &str,
    ) -> Vec<HourStatisticsDto> {
        let sql = format!(
            "select {} from {} where hour_key = ? and scope = ?",
            COLUMNS, TABLE_NAME
        );

        self.query(
            sql.as_str(),
            vec![Value::Integer(hour.to_i64()), as_text(scope)],
        )
        .await
    }

    async fn query(&self, sql: &str, params: Vec<Value>) -> Vec<HourStatisticsDto> {
        let connection = self.db.connect().unwrap();

        let mut rows = connection
            .query(sql, Params::Positional(params))
            .await
            .unwrap();

        let mut result = Vec::new();

        while let Some(row) = rows.next().await.unwrap() {
            result.push(HourStatisticsDto::from_row(&row));
        }

        result
    }
}
