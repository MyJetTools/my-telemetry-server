#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct TimeModel {
    pub min: i64,
    pub sec: i64,
}

impl TimeModel {
    pub fn from_str(s: &str) -> Self {
        let mut parts = s.split(':');

        let mut min = 0;
        let mut sec = 0;

        if let Some(next) = parts.next() {
            sec = next.parse().unwrap_or(0);
        }

        if let Some(next) = parts.next() {
            min = sec;
            sec = next.parse().unwrap_or(0);
        }

        Self { min, sec }
    }

    pub fn to_string(&self) -> String {
        format!("{:02}:{:02}", self.min, self.sec)
    }

    pub fn to_seconds_within_hour(&self) -> i64 {
        self.min * 60 + self.sec
    }
}
