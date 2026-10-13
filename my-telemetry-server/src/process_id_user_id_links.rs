use ahash::AHashMap;
use rust_extensions::date_time::DateTimeAsMicroseconds;

#[derive(Debug)]
pub struct ProcessIdUserIdLink {
    pub user_id: String,
    pub created_at: DateTimeAsMicroseconds,
}

pub struct ProcessIdUserIdLinks {
    pub items: AHashMap<i64, ProcessIdUserIdLink>,
}

impl ProcessIdUserIdLinks {
    pub fn new() -> Self {
        Self {
            items: AHashMap::new(),
        }
    }

    pub fn update(&mut self, process_id: i64, user_id: &str) {
        if !self.items.contains_key(&process_id) {
            self.items.insert(
                process_id,
                ProcessIdUserIdLink {
                    user_id: user_id.to_string(),
                    created_at: DateTimeAsMicroseconds::now(),
                },
            );
        }
    }

    /// Lets go of the links made `keep_seconds` ago or before
    pub fn gc(&mut self, keep_seconds: i64) {
        let now = DateTimeAsMicroseconds::now();

        self.items
            .retain(|_, itm| (now - itm.created_at).get_full_seconds() < keep_seconds);
    }

    pub fn resolve_user_id(&self, process_id: i64) -> Option<&str> {
        let result = self.items.get(&process_id)?;

        Some(&result.user_id)
    }

    pub fn get_size_and_capacity(&self) -> (usize, usize) {
        (self.items.len(), self.items.capacity())
    }
}
