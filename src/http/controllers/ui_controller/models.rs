// The wire models themselves live in `rest-api-shared` and are used verbatim by
// the wasm UI in `ui/`, so the request the UI builds and the request this server
// parses can not drift apart. Only the mapping from the storage DTOs - which the
// UI never sees - stays here.
pub use rest_api_shared::*;

use crate::db::*;

impl From<&EventTagDto> for TagHttpModel {
    fn from(value: &EventTagDto) -> Self {
        Self {
            key: value.key.clone(),
            value: value.value.clone(),
        }
    }
}

/// The client id is lifted out of the tags on the way in (it gets its own column
/// so it can be filtered on), so it has to be put back on the way out - otherwise
/// the UI would show every tag of an event except the one identifying the client.
/// This mirrors what `mappers::metric_tags::to_tag_grpc_model` does for gRPC.
fn map_tags(tags: Option<Vec<EventTagDto>>, client_id: Option<String>) -> Vec<TagHttpModel> {
    let mut result: Vec<TagHttpModel> = match tags {
        Some(tags) => tags.iter().map(|tag| tag.into()).collect(),
        None => Vec::new(),
    };

    if let Some(client_id) = client_id {
        result.push(TagHttpModel {
            key: crate::mappers::CLIENT_ID_TAG.to_string(),
            value: client_id,
        });
    }

    result
}

impl Into<MetricHttpModel> for MetricDto {
    fn into(self) -> MetricHttpModel {
        MetricHttpModel {
            id: self.id,
            started: self.started,
            duration: self.duration_micro,
            success: self.success,
            error: self.fail,
            tags: map_tags(self.tags, self.client_id),
        }
    }
}

impl Into<MetricByProcessModel> for MetricDto {
    fn into(self) -> MetricByProcessModel {
        MetricByProcessModel {
            id: self.name,
            data: self.data,
            started: self.started,
            duration: self.duration_micro,
            success: self.success,
            error: self.fail,
            tags: map_tags(self.tags, self.client_id),
        }
    }
}
