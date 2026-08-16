use my_http_server::macros::{MyHttpInput, MyHttpObjectStructure};
use my_http_server::HttpFailResult;
use serde::{Deserialize, Serialize};

use crate::db::*;
use crate::ignore_events::IgnoreEvents;

#[derive(MyHttpInput)]
pub struct NewMetricsEvent {
    #[http_body_raw(description = "Metrics")]
    pub body: my_http_server::RawDataTyped<Vec<NewMetric>>,
}

impl NewMetricsEvent {
    pub fn into_dto(self, ignore_events: &IgnoreEvents) -> Result<Vec<MetricDto>, HttpFailResult> {
        let metrics = self.body.deserialize_json();

        if metrics.is_err() {
            let to_print = self.body.as_slice();
            let to_print = if to_print.len() >= 64 {
                std::str::from_utf8(&to_print[0..64]).unwrap()
            } else {
                std::str::from_utf8(to_print).unwrap()
            };

            println!("Invalid json: {}", to_print);
        }

        let metrics = metrics?;

        let mut result: Vec<MetricDto> = Vec::with_capacity(metrics.len());
        for mut metric in metrics {
            if ignore_events.event_should_be_ignored(&metric.service_name, &metric.event_data) {
                continue;
            }

            let mut duration = metric.ended - metric.started;
            if duration < 0 {
                duration = 0;
            }

            if let Some(ip) = metric.ip.take() {
                if metric.tags.is_none() {
                    metric.tags = Some(Vec::new());
                }

                metric.tags.as_mut().unwrap().push(MetricHttpTag {
                    key: "ip".to_string(),
                    value: ip,
                });
            }

            let metric_tags = crate::mappers::metric_tags::get(metric.tags.take());

            println!("Getting metrics by HTTP: {:?}", metric.service_name);

            result.push(MetricDto {
                id: metric.process_id,
                started: metric.started,
                duration_micro: duration,
                name: metric.service_name,
                data: metric.event_data,
                success: metric.success,
                fail: metric.fail,
                tags: metric_tags.tags,
                client_id: metric_tags.client_id,
            })
        }

        Ok(result)
    }
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure)]
pub struct NewMetric {
    #[serde(rename = "processId")]
    pub process_id: i64,
    #[serde(rename = "started")]
    pub started: i64,

    #[serde(rename = "ended")]
    pub ended: i64,
    #[serde(rename = "serviceName")]
    pub service_name: String,
    #[serde(rename = "eventData")]
    pub event_data: String,
    pub success: Option<String>,
    pub fail: Option<String>,
    pub ip: Option<String>,
    pub tags: Option<Vec<MetricHttpTag>>,
}
#[derive(Serialize, Deserialize, MyHttpObjectStructure)]
pub struct MetricHttpTag {
    pub key: String,
    pub value: String,
}
