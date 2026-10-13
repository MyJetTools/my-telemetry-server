use std::time::Duration;

use crate::{
    app_ctx::AppContext,
    db::PermanentMetricDto,
    mappers::CLIENT_ID_TAG,
    metrics_storage::{ReadyBucket, Rec},
    permanent_users::PermanentUsersList,
    process_id_user_id_links::ProcessIdUserIdLinks,
    writer_grpc::EventGrpcTag,
};

/// A link of a process to its client is kept this long past the time the metrics wait in
/// the storage: an event which comes this long after the one which named the client is
/// still of the client
const LINK_SECONDS: i64 = 20;

/// Writes the seconds which have waited `WindowSeconds` in the memory of the storage - or
/// `all` of them, and the indexes with them, when the process is about to stop.
///
/// This is where a metric gets the `client_id` of its process. The client is named by one
/// of the events of a process, and events are sent when they end: the one which names the
/// client - the request the others are a part of - is often the last to come. The time a
/// second waits in memory is the time it has to come in.
pub async fn write_ready_metrics(app: &AppContext, all: bool) {
    let window_seconds = app.settings_reader.get_window_seconds().await;

    let window = (!all).then_some(Duration::from_secs(window_seconds));

    // Nothing is awaited between the moment the seconds are taken and the moment they are
    // handed to the write: the statistics are locked first, and what is kept for good is
    // kept after. A tick which is given up on, or a table which does not take a row, does
    // not cost the seconds it took.
    let (ready, permanent_metrics) = {
        let mut cache = app.cache.lock().await;
        let cache = &mut *cache;

        let mut permanent_metrics = Vec::new();

        let ready = app.repo.take_ready(window, |ready| {
            permanent_metrics = name_clients(
                ready,
                &cache.process_id_user_id_links,
                &cache.permanent_users_list,
            );
        });

        cache
            .process_id_user_id_links
            .gc(window_seconds as i64 + LINK_SECONDS);

        (ready, permanent_metrics)
    };

    app.repo.write_ready(ready, all).await;

    if !permanent_metrics.is_empty() {
        app.permanent_metrics.insert(&permanent_metrics).await
    }
}

/// Gives every metric which names no client the client of its process, when one of the
/// events of the process named it. Returns the metrics of the clients which are kept for
/// good, as they are kept.
fn name_clients(
    ready: &mut [ReadyBucket],
    links: &ProcessIdUserIdLinks,
    permanent_users: &PermanentUsersList,
) -> Vec<PermanentMetricDto> {
    let mut permanent_metrics = Vec::new();

    for record in ready.iter_mut().flat_map(|bucket| bucket.records.iter_mut()) {
        if client_id_of(record).is_none() {
            if let Some(client_id) = links.resolve_user_id(record.process_id) {
                record.tags.push(EventGrpcTag {
                    key: CLIENT_ID_TAG.to_string(),
                    value: client_id.to_string(),
                });
            }
        }

        if let Some(client_id) = client_id_of(record) {
            if permanent_users.is_permanent(client_id) {
                permanent_metrics.push(PermanentMetricDto::of_record(record, client_id));
            }
        }
    }

    permanent_metrics
}

/// The client a metric names: its `client_id` tag
fn client_id_of(record: &Rec) -> Option<&str> {
    record
        .tags
        .iter()
        .find(|tag| tag.key == CLIENT_ID_TAG)
        .map(|tag| tag.value.as_str())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn record(process_id: i64, action: &str, client_id: Option<&str>) -> Rec {
        let mut tags = vec![EventGrpcTag {
            key: "ip".to_string(),
            value: "10.0.0.1".to_string(),
        }];

        if let Some(client_id) = client_id {
            tags.push(EventGrpcTag {
                key: CLIENT_ID_TAG.to_string(),
                value: client_id.to_string(),
            });
        }

        Rec {
            process_id,
            started_at: 1000,
            finished_at: 1700,
            service_name: "api".to_string(),
            event_data: action.to_string(),
            success: Some("ok".to_string()),
            fail: None,
            tags,
        }
    }

    fn bucket(records: Vec<Rec>) -> ReadyBucket {
        ReadyBucket {
            scope: Arc::from("default"),
            second: 0,
            records,
        }
    }

    #[test]
    fn a_metric_gets_the_client_of_its_process() {
        let mut links = ProcessIdUserIdLinks::new();
        // Named by an event of process 1 - one which came after the others, or before
        links.update(1, "client-1");

        let mut permanent_users = PermanentUsersList::new();
        permanent_users.add_permanent_user("client-7".to_string());

        let mut ready = vec![
            bucket(vec![
                record(1, "inner", None),
                record(1, "request", Some("client-1")),
                record(2, "no-client", None),
            ]),
            bucket(vec![
                record(1, "later", None),
                // It names a client of its own: the one of the process is not put over it
                record(1, "other", Some("client-7")),
            ]),
        ];

        let permanent = name_clients(&mut ready, &links, &permanent_users);

        let clients: Vec<(String, Option<String>)> = ready
            .iter()
            .flat_map(|bucket| bucket.records.iter())
            .map(|record| {
                (
                    record.event_data.clone(),
                    client_id_of(record).map(str::to_string),
                )
            })
            .collect();

        assert_eq!(
            clients,
            [
                ("inner".to_string(), Some("client-1".to_string())),
                ("request".to_string(), Some("client-1".to_string())),
                ("no-client".to_string(), None),
                ("later".to_string(), Some("client-1".to_string())),
                ("other".to_string(), Some("client-7".to_string())),
            ]
        );

        // The tag is put once, after the tags the metric came with
        let inner = &ready[0].records[0];
        assert_eq!(inner.tags.len(), 2);
        assert_eq!(inner.tags[0].key, "ip");
        assert_eq!(ready[0].records[1].tags.len(), 2);

        // What is kept for good is what the client who is kept for good did - as the
        // table has it: the client apart from the tags
        assert_eq!(permanent.len(), 1);
        assert_eq!(permanent[0].client_id, "client-7");
        assert_eq!(permanent[0].data, "other");
        assert_eq!(permanent[0].id, 1);
        assert_eq!(permanent[0].started, 1000);
        assert_eq!(permanent[0].duration_micro, 700);

        let tags = permanent[0].tags.as_ref().unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].key, "ip");
    }
}
