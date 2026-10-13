use crate::settings::IgnoreEvent;

pub struct IgnoreEvents {
    data: Vec<IgnoreEvent>,
}

impl IgnoreEvents {
    pub fn new(data: Vec<IgnoreEvent>) -> Self {
        Self { data }
    }

    pub fn event_should_be_ignored(&self, name: &str, data: &str) -> bool {
        for ignore in self.data.iter() {
            if &ignore.name == name && &ignore.data == data {
                return true;
            }
        }

        false
    }
}
