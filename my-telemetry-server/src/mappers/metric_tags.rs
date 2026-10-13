use crate::http::controllers::metric_event::models::*;
use crate::{db::*, writer_grpc::EventGrpcTag};

pub const USER_ID_TAG: &str = "user_id";
pub const CLIENT_ID_TAG: &str = "client_id";

pub trait MetricTag {
    fn into_key_value(self) -> (String, String);
}

impl MetricTag for EventGrpcTag {
    fn into_key_value(self) -> (String, String) {
        (self.key, self.value)
    }
}

impl MetricTag for MetricHttpTag {
    fn into_key_value(self) -> (String, String) {
        (self.key, self.value)
    }
}

pub struct MetricTagsResult {
    pub tags: Option<Vec<EventTagDto>>,
    pub client_id: Option<String>,
}

pub fn get(src: Option<Vec<impl MetricTag>>) -> MetricTagsResult {
    let mut result = MetricTagsResult {
        tags: None,
        client_id: None,
    };

    if src.is_none() {
        return result;
    }

    let src = src.unwrap();
    let capacity = src.len();

    for itm in src {
        let (key, value) = itm.into_key_value();

        if key == USER_ID_TAG {
            result.client_id = Some(value);
            continue;
        }

        if key == CLIENT_ID_TAG {
            result.client_id = Some(value);
            continue;
        }

        if result.tags.is_none() {
            result.tags = Some(Vec::with_capacity(capacity));
        }

        result
            .tags
            .as_mut()
            .unwrap()
            .push(EventTagDto { key, value });
    }

    result
}
