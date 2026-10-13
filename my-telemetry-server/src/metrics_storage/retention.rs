use rust_extensions::date_time::{DateTimeAsMicroseconds, HourKey, IntervalKey};

use super::MICROS_PER_SECOND;

/// Hours stay on the local disk this long after they end, and are written into until then.
/// After it an hour is uploaded to S3 when it is set up, and deleted either way.
pub const LOCAL_HOURS: i64 = 2;

pub const HOUR_MICROS: i64 = 3600 * MICROS_PER_SECOND;

/// The start of the hour the time is in. Hours are the hours of UTC.
pub fn hour_start_of(time: i64) -> i64 {
    time - time.rem_euclid(HOUR_MICROS)
}

/// `2026101207` - the key of the hour which starts at `hour_start`: what its folder is named
pub fn hour_key_of(hour_start: i64) -> i64 {
    let key: IntervalKey<HourKey> = DateTimeAsMicroseconds::new(hour_start).into();
    key.to_i64()
}

/// Where the hour of the key starts; `None` - the number is not a key of an hour
pub fn hour_start_of_key(hour_key: i64) -> Option<i64> {
    let key: IntervalKey<HourKey> = hour_key.into();
    let hour_start = key.try_to_date_time().ok()?.unix_microseconds;

    // 2026139999 reads as a date too: a key is one only when the hour gives it back
    (hour_key_of(hour_start) == hour_key).then_some(hour_start)
}

/// The hour is not written into any more, and its time on the local disk is over
pub fn local_time_is_over(hour_start: i64, now: i64) -> bool {
    hour_start + HOUR_MICROS + LOCAL_HOURS * HOUR_MICROS <= now
}

/// The start of the latest hour which is older than what is kept: `hours_to_gc` hours
pub fn expired_up_to(now: i64, hours_to_gc: u64) -> i64 {
    hour_start_of(now - hours_to_gc as i64 * HOUR_MICROS)
}

/// The hour is older than what is kept
pub fn is_expired(hour_start: i64, now: i64, hours_to_gc: u64) -> bool {
    hour_start <= expired_up_to(now, hours_to_gc)
}

/// Whether the hour is to be uploaded now: its local time is over and it is still kept.
/// With `hours_to_gc` of 3 or less none is - an hour is that old by the time it is due.
pub fn is_to_upload(hour_start: i64, now: i64, hours_to_gc: u64) -> bool {
    local_time_is_over(hour_start, now) && !is_expired(hour_start, now, hours_to_gc)
}

/// Whether the folder of the hour is to be deleted from the local disk. Without S3 - once
/// its local time is over. With S3 it is uploaded by then and deleted as it is; one still
/// here was not - its uploads fail, or it is not kept long enough to be uploaded - and it
/// goes when it expires.
pub fn is_to_delete_locally(hour_start: i64, now: i64, hours_to_gc: u64, has_s3: bool) -> bool {
    local_time_is_over(hour_start, now) && (!has_s3 || is_expired(hour_start, now, hours_to_gc))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2023-11-14 22:00:00 UTC
    const HOUR: i64 = 1_699_999_200 * MICROS_PER_SECOND;

    #[test]
    fn an_hour_is_named_by_its_key() {
        assert_eq!(hour_key_of(HOUR), 2023111422);
        assert_eq!(hour_start_of(HOUR + 3599 * MICROS_PER_SECOND + 999_999), HOUR);
        assert_eq!(hour_start_of(HOUR + HOUR_MICROS), HOUR + HOUR_MICROS);
        assert_eq!(hour_start_of_key(2023111422), Some(HOUR));
        assert_eq!(hour_start_of_key(2023111423), Some(HOUR + HOUR_MICROS));
        assert_eq!(hour_start_of_key(2023111500), Some(HOUR + 2 * HOUR_MICROS));
        // Not hours
        assert_eq!(hour_start_of_key(2023111424), None);
        assert_eq!(hour_start_of_key(2023130100), None);
        assert_eq!(hour_start_of_key(7), None);
    }

    #[test]
    fn an_hour_is_local_for_two_hours_after_it_ends() {
        let end = HOUR + HOUR_MICROS;

        assert!(!local_time_is_over(HOUR, end));
        assert!(!local_time_is_over(HOUR, end + 2 * HOUR_MICROS - 1));
        assert!(local_time_is_over(HOUR, end + 2 * HOUR_MICROS));
    }

    #[test]
    fn without_s3_an_hour_is_deleted_when_its_local_time_is_over() {
        let over = HOUR + 3 * HOUR_MICROS;

        assert!(!is_to_delete_locally(HOUR, over - 1, 168, false));
        assert!(is_to_delete_locally(HOUR, over, 168, false));
    }

    #[test]
    fn with_s3_an_hour_is_uploaded_and_kept_until_it_expires() {
        let over = HOUR + 3 * HOUR_MICROS;

        assert!(!is_to_upload(HOUR, over - 1, 168));
        assert!(is_to_upload(HOUR, over, 168));
        // Still here - its uploads fail: it stays until it expires
        assert!(!is_to_delete_locally(HOUR, over, 168, true));
        assert!(!is_to_delete_locally(HOUR, HOUR + 168 * HOUR_MICROS - 1, 168, true));
        assert!(is_to_delete_locally(HOUR, HOUR + 168 * HOUR_MICROS, 168, true));
        assert!(!is_to_upload(HOUR, HOUR + 168 * HOUR_MICROS, 168));
    }

    #[test]
    fn an_hour_not_kept_as_long_as_its_upload_takes_is_not_uploaded() {
        let over = HOUR + 3 * HOUR_MICROS;

        for hours_to_gc in [0, 1, 3] {
            assert!(!is_to_upload(HOUR, over, hours_to_gc));
            assert!(is_to_delete_locally(HOUR, over, hours_to_gc, true));
        }

        assert!(is_to_upload(HOUR, over, 4));
    }
}
