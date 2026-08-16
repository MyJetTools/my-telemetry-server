use prost::Message;

use crate::db::MetricDto;
use crate::writer_grpc::TelemetryGrpcEvent;

/// A record on disk is `[payload length: u16 LE][payload: TelemetryGrpcEvent protobuf]`.
pub const LEN_PREFIX_SIZE: usize = 2;

/// The u16 prefix is the hard ceiling on one record. Anything bigger is truncated rather
/// than dropped - see [`encode`].
pub const MAX_PAYLOAD_LEN: usize = u16::MAX as usize;

/// Appended to a field that had to be cut. A truncated stack trace must not read as a
/// complete one in the UI.
const TRUNCATED_MARKER: &str = "...[truncated]";

/// Serializes one metric into `[len][payload]`.
///
/// A metric whose payload exceeds [`MAX_PAYLOAD_LEN`] is shrunk, not dropped: the largest
/// free-text field is cut and marked, repeatedly, until the record fits. Tags go last,
/// because they are the only part a query filters on.
pub fn encode(dto: MetricDto) -> Vec<u8> {
    let mut event: TelemetryGrpcEvent = dto.into();

    if event.encoded_len() > MAX_PAYLOAD_LEN {
        shrink_to_max_len(&mut event);
    }

    let payload_len = event.encoded_len();

    let mut result = Vec::with_capacity(LEN_PREFIX_SIZE + payload_len);
    result.extend_from_slice(&(payload_len as u16).to_le_bytes());
    event.encode(&mut result).unwrap();

    result
}

/// Reads one payload back. `None` means the bytes are not a decodable record - a torn tail
/// after a crash, or a file written by something else.
pub fn decode(payload: &[u8]) -> Option<MetricDto> {
    let event = TelemetryGrpcEvent::decode(payload).ok()?;
    Some(event.into())
}

/// Walks `[len][payload]` records over a buffer. Stops at the first record that does not
/// fit the remaining bytes, so a torn tail truncates the scan instead of panicking.
pub struct RecordsIterator<'s> {
    src: &'s [u8],
    pos: usize,
}

impl<'s> RecordsIterator<'s> {
    pub fn new(src: &'s [u8]) -> Self {
        Self { src, pos: 0 }
    }

    /// Offset of the next unread byte - i.e. how much of the buffer held whole records.
    pub fn position(&self) -> usize {
        self.pos
    }
}

impl<'s> Iterator for RecordsIterator<'s> {
    /// `(offset of the record within the buffer, payload bytes)`
    type Item = (usize, &'s [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + LEN_PREFIX_SIZE > self.src.len() {
            return None;
        }

        let len = u16::from_le_bytes([self.src[self.pos], self.src[self.pos + 1]]) as usize;

        let payload_from = self.pos + LEN_PREFIX_SIZE;
        let payload_to = payload_from + len;

        if payload_to > self.src.len() {
            return None;
        }

        let record_at = self.pos;
        self.pos = payload_to;

        Some((record_at, &self.src[payload_from..payload_to]))
    }
}

fn shrink_to_max_len(event: &mut TelemetryGrpcEvent) {
    // Free-text fields first, biggest one each round - a 200Kb stack trace in `fail` should
    // not cost us the `data` we index screens by.
    while event.encoded_len() > MAX_PAYLOAD_LEN {
        let excess = event.encoded_len() - MAX_PAYLOAD_LEN;

        let Some(field) = biggest_text_field(event) else {
            break;
        };

        if field.len() <= TRUNCATED_MARKER.len() {
            break;
        }

        let keep = field
            .len()
            .saturating_sub(excess + TRUNCATED_MARKER.len())
            .max(1);

        truncate_utf8(field, keep);
        field.push_str(TRUNCATED_MARKER);
    }

    // Text alone was not enough: the size is in the tags. Drop them from the back.
    while event.encoded_len() > MAX_PAYLOAD_LEN && event.tags.pop().is_some() {}
}

fn biggest_text_field(event: &mut TelemetryGrpcEvent) -> Option<&mut String> {
    let mut candidates: Vec<&mut String> = Vec::with_capacity(4);

    if let Some(fail) = event.fail.as_mut() {
        candidates.push(fail);
    }
    if let Some(success) = event.success.as_mut() {
        candidates.push(success);
    }
    candidates.push(&mut event.event_data);
    candidates.push(&mut event.service_name);

    candidates.into_iter().max_by_key(|itm| itm.len())
}

/// `String::truncate` panics off a char boundary, and these strings are client supplied.
fn truncate_utf8(src: &mut String, max_bytes: usize) {
    if src.len() <= max_bytes {
        return;
    }

    let mut boundary = max_bytes;
    while boundary > 0 && !src.is_char_boundary(boundary) {
        boundary -= 1;
    }

    src.truncate(boundary);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::EventTagDto;

    fn dto(started: i64, data: &str, fail: Option<String>) -> MetricDto {
        MetricDto {
            id: 42,
            started,
            duration_micro: 15,
            name: "my-service".to_string(),
            data: data.to_string(),
            success: None,
            fail,
            tags: Some(vec![EventTagDto {
                key: "k".to_string(),
                value: "v".to_string(),
            }]),
            client_id: Some("client-1".to_string()),
        }
    }

    #[test]
    fn round_trip_keeps_every_field() {
        let encoded = encode(dto(1_700_000_000_000_000, "GET /orders", None));

        let mut it = RecordsIterator::new(&encoded);
        let (at, payload) = it.next().unwrap();
        assert_eq!(at, 0);
        assert!(it.next().is_none());

        let back = decode(payload).unwrap();

        assert_eq!(back.id, 42);
        assert_eq!(back.started, 1_700_000_000_000_000);
        assert_eq!(back.duration_micro, 15);
        assert_eq!(back.name, "my-service");
        assert_eq!(back.data, "GET /orders");
        // client_id travels as a tag on disk and is lifted back out on the way in.
        assert_eq!(back.client_id.as_deref(), Some("client-1"));
        assert_eq!(back.get_tag_value("k"), Some("v"));
    }

    #[test]
    fn iterator_walks_many_records() {
        let mut buffer = Vec::new();
        for i in 0..5 {
            buffer.extend_from_slice(&encode(dto(1_000 + i, "data", None)));
        }

        let found: Vec<i64> = RecordsIterator::new(&buffer)
            .map(|(_, payload)| decode(payload).unwrap().started)
            .collect();

        assert_eq!(found, vec![1_000, 1_001, 1_002, 1_003, 1_004]);
    }

    #[test]
    fn torn_tail_stops_the_scan_instead_of_panicking() {
        let mut buffer = encode(dto(1_000, "data", None));
        let whole = buffer.len();
        buffer.extend_from_slice(&encode(dto(2_000, "data", None)));
        buffer.truncate(whole + 5); // second record cut mid payload

        let mut it = RecordsIterator::new(&buffer);
        assert!(it.next().is_some());
        assert!(it.next().is_none());
        assert_eq!(it.position(), whole);
    }

    #[test]
    fn oversized_record_is_truncated_marked_and_still_fits() {
        let huge = "x".repeat(MAX_PAYLOAD_LEN * 2);
        let encoded = encode(dto(1_000, "GET /orders", Some(huge)));

        assert!(encoded.len() <= LEN_PREFIX_SIZE + MAX_PAYLOAD_LEN);

        let (_, payload) = RecordsIterator::new(&encoded).next().unwrap();
        let back = decode(payload).unwrap();

        assert!(back.fail.unwrap().ends_with(TRUNCATED_MARKER));
        // The fields a query filters on survive the cut.
        assert_eq!(back.data, "GET /orders");
        assert_eq!(back.name, "my-service");
    }

    #[test]
    fn multibyte_text_is_cut_on_a_char_boundary() {
        let huge = "Ф".repeat(MAX_PAYLOAD_LEN);
        let encoded = encode(dto(1_000, "data", Some(huge)));

        let (_, payload) = RecordsIterator::new(&encoded).next().unwrap();
        assert!(decode(payload).is_some());
    }
}
