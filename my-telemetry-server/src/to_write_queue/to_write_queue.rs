use parking_lot::Mutex;

use crate::db::MetricDto;

/// The metrics which came and are not taken in yet. A writer puts them here and signals
/// the background executor, which takes them at once: nothing waits here but for its turn.
pub struct ToWriteQueue {
    metrics: Mutex<Vec<MetricDto>>,
}

impl ToWriteQueue {
    pub fn new() -> Self {
        Self {
            metrics: Mutex::new(Vec::new()),
        }
    }

    pub fn enqueue(&self, mut to_push: Vec<MetricDto>) {
        let mut metrics = self.metrics.lock();

        if metrics.is_empty() {
            // Nothing is copied: the batch becomes the queue
            std::mem::swap(&mut *metrics, &mut to_push);
        } else {
            metrics.append(&mut to_push);
        }
    }

    /// The metrics which came first, `max` of them at most, in the order they came
    pub fn take(&self, max: usize) -> Vec<MetricDto> {
        let mut metrics = self.metrics.lock();

        if metrics.len() <= max {
            return std::mem::take(&mut *metrics);
        }

        let rest = metrics.split_off(max);
        std::mem::replace(&mut *metrics, rest)
    }

    pub fn get_size_and_capacity(&self) -> (usize, usize) {
        let metrics = self.metrics.lock();
        (metrics.len(), metrics.capacity())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(id: i64) -> MetricDto {
        MetricDto {
            scope: "default".into(),
            id,
            started: 0,
            duration_micro: 0,
            name: String::new(),
            data: String::new(),
            success: None,
            fail: None,
            tags: None,
            client_id: None,
        }
    }

    fn ids(metrics: &[MetricDto]) -> Vec<i64> {
        metrics.iter().map(|metric| metric.id).collect()
    }

    #[test]
    fn metrics_are_taken_in_the_order_they_came() {
        let queue = ToWriteQueue::new();

        assert!(queue.take(10).is_empty());

        queue.enqueue(vec![metric(1), metric(2)]);
        queue.enqueue(vec![metric(3)]);
        queue.enqueue(vec![metric(4), metric(5)]);

        assert_eq!(queue.get_size_and_capacity().0, 5);

        assert_eq!(ids(&queue.take(2)), [1, 2]);
        assert_eq!(ids(&queue.take(2)), [3, 4]);
        assert_eq!(ids(&queue.take(2)), [5]);
        assert!(queue.take(2).is_empty());
    }
}
