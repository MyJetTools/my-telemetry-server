use crate::models::AvailableHourHttpModel;

pub struct AvailableFiles {
    files: Option<Vec<AvailableHourHttpModel>>,
}

impl AvailableFiles {
    pub fn new() -> Self {
        Self { files: None }
    }

    pub fn initialized(&self) -> bool {
        self.files.is_some()
    }

    pub fn set_files(&mut self, files: Vec<AvailableHourHttpModel>) {
        self.files = Some(files);
    }

    pub fn get_files(&self) -> Option<&Vec<AvailableHourHttpModel>> {
        self.files.as_ref()
    }

    /// The stored `hours_ago` if the server still has that hour, otherwise the
    /// first hour it does have - a selection made yesterday must not leave the UI
    /// pointing at an hour that has since been garbage-collected.
    fn resolve(&self, hours_ago: i64) -> Option<&AvailableHourHttpModel> {
        let files = self.files.as_ref()?;

        for file in files {
            if file.hours_ago == hours_ago {
                return Some(file);
            }
        }

        files.get(0)
    }

    pub fn get_available_hours_ago(&self, hours_ago: i64) -> Option<i64> {
        Some(self.resolve(hours_ago)?.hours_ago)
    }

    /// The hour key is always taken from what the server reported - the browser has
    /// no clock it could derive one from under wasm.
    pub fn get_hour_key(&self, hours_ago: i64) -> Option<i64> {
        Some(self.resolve(hours_ago)?.hour_key)
    }
}
