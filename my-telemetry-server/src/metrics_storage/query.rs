use crate::mappers::CLIENT_ID_TAG;

use super::Rec;

/// What the storage is asked for: one of the two searches the UI makes
pub enum MetricsQuery<'s> {
    /// The events of an action of a service which started at `from` or later - of one
    /// client, when it is given. The key index says which seconds have them.
    Action {
        name: &'s str,
        data: &'s str,
        client_id: Option<&'s str>,
        from: i64,
    },
    /// Every event of a process: the trace behind one click. The process index says which
    /// seconds have them.
    Process { process_id: i64 },
}

impl<'s> MetricsQuery<'s> {
    pub fn matches(&self, record: &Rec) -> bool {
        match self {
            MetricsQuery::Action {
                name,
                data,
                client_id,
                from,
            } => {
                if record.service_name != *name
                    || record.event_data != *data
                    || record.started_at < *from
                {
                    return false;
                }

                match client_id {
                    // The client id of a record is its `client_id` tag
                    Some(client_id) => record
                        .tags
                        .iter()
                        .any(|tag| tag.key == CLIENT_ID_TAG && tag.value == *client_id),
                    None => true,
                }
            }
            MetricsQuery::Process { process_id } => record.process_id == *process_id,
        }
    }
}
