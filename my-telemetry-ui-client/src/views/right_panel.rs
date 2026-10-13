use dioxus::prelude::*;

use crate::{states::*, views::*};

#[component]
pub fn RightPanel() -> Element {
    let main_state = consume_context::<Signal<MainState>>();

    let right_panel_state = main_state.read().get_right_panel();
    match right_panel_state {
        Some(state) => match state {
            RightPanelState::ShowServiceOverview => rsx! {
                ServicesOverview {}
            },
            RightPanelState::ShowServiceDataOverview(data) => {
                rsx! {
                    ServiceDataOverview { data: data.clone() }
                }
            }
            RightPanelState::ShowProcess(data, process_id) => {
                rsx! {
                    ProcessOverview { data, process_id }
                }
            }
        },
        None => {
            rsx!("Nothing selected")
        }
    }
}
