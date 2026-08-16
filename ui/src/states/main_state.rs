use std::{collections::BTreeMap, rc::Rc};

use crate::models::*;

use super::{AvailableFiles, DataState, DialogState, RightPanelState};

pub struct MainState {
    pub left_panel: DataState<Rc<BTreeMap<Rc<String>, ServiceHttpModel>>>,
    pub selected_service: Option<Rc<String>>,
    pub right_panel_state: Option<RightPanelState>,
    pub server_data_overview: DataState<Rc<Vec<MetricHttpModel>>>,
    pub dialog: Option<DialogState>,
    pub files: AvailableFiles,
    pub client_id: String,
    pub from_time: TimeModel,
}

impl MainState {
    pub fn new() -> Self {
        Self::create(None, None)
    }

    pub fn new_with_selected_service(service_name: String) -> Self {
        Self::create(
            Some(Rc::new(service_name)),
            Some(RightPanelState::ShowServiceOverview),
        )
    }

    pub fn new_with_selected_action(service_name: String, action: String) -> Self {
        Self::create(
            Some(Rc::new(service_name)),
            Some(RightPanelState::ShowServiceDataOverview(Rc::new(action))),
        )
    }

    pub fn new_with_selected_process(
        service_name: String,
        action: String,
        process_id: i64,
    ) -> Self {
        Self::create(
            Some(Rc::new(service_name)),
            Some(RightPanelState::ShowProcess(Rc::new(action), process_id)),
        )
    }

    fn create(
        selected_service: Option<Rc<String>>,
        right_panel_state: Option<RightPanelState>,
    ) -> Self {
        Self {
            right_panel_state,
            selected_service,
            dialog: None,
            left_panel: DataState::None,
            server_data_overview: DataState::None,
            files: AvailableFiles::new(),
            client_id: crate::storage::client_id::get(),
            from_time: crate::storage::from_time::get(),
        }
    }

    pub fn set_selected(&mut self, selected: Rc<String>) {
        self.selected_service = Some(selected);
        self.right_panel_state = Some(RightPanelState::ShowServiceOverview);
    }

    pub fn get_selected_service(&self) -> Option<Rc<String>> {
        self.selected_service.clone()
    }

    pub fn set_selected_data(&mut self, service_id: Rc<String>, data: Rc<String>) {
        self.right_panel_state = Some(RightPanelState::ShowServiceDataOverview(data));
        self.selected_service = Some(service_id);
    }

    pub fn set_show_process(&mut self, service_id: Rc<String>, data: Rc<String>, process_id: i64) {
        self.right_panel_state = Some(RightPanelState::ShowProcess(data, process_id));
        self.selected_service = Some(service_id);
    }

    pub fn get_right_panel(&self) -> Option<RightPanelState> {
        self.right_panel_state.clone()
    }

    pub fn get_dialog_state(&self) -> Option<&DialogState> {
        self.dialog.as_ref()
    }

    pub fn show_dialog(&mut self, dialog_state: DialogState) {
        self.dialog = Some(dialog_state);
    }

    pub fn hide_dialog(&mut self) {
        self.dialog = None;
    }

    pub fn set_hours_ago(&mut self, hours_ago: i64) {
        crate::storage::hours_ago::set(hours_ago);
        self.right_panel_state = None;
        self.dialog = None;
        self.selected_service = None;
        self.left_panel = DataState::None;
    }

    pub fn try_get_hours_ago(&self) -> Option<i64> {
        self.files
            .get_available_hours_ago(crate::storage::hours_ago::get())
    }

    /// Every request the UI makes is keyed on the hour, and the key comes from the
    /// available-hours list the server sent - never from a local clock.
    pub fn get_hour_key(&self) -> Option<i64> {
        self.files.get_hour_key(crate::storage::hours_ago::get())
    }

    pub fn apply_client_id(&mut self) {
        crate::storage::client_id::set(&self.client_id);
        crate::storage::from_time::set(&self.from_time);
        self.server_data_overview = DataState::None;
    }

    pub fn reset_client_id(&mut self) {
        self.client_id = "".to_string();
        self.from_time = TimeModel::default();
        crate::storage::client_id::set(&self.client_id);
        crate::storage::from_time::set(&self.from_time);
        self.server_data_overview = DataState::None;
    }
}
