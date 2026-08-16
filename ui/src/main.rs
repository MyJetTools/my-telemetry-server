use dioxus::prelude::*;

mod api;
mod components;
mod models;
mod states;
mod storage;
mod utils;
mod views;

use views::*;

use crate::states::*;
use crate::utils::from_base_64;

#[derive(Routable, Clone, Debug, PartialEq)]
pub enum AppRoute {
    #[route("/")]
    Home,

    #[route("/actions/:service")]
    Actions { service: String },

    #[route("/last/:service/:action")]
    LastEvents { service: String, action: String },

    #[route("/process/:service/:action/:id")]
    Process {
        service: String,
        action: String,
        id: i64,
    },

    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

#[component]
fn NotFound(segments: Vec<String>) -> Element {
    rsx! { "404: Not Found" }
}

fn main() {
    dioxus::LaunchBuilder::new().launch(|| {
        rsx! {
            document::Link { rel: "icon", href: asset!("/public/favicon.ico") }
            Router::<AppRoute> {}
        }
    })
}

#[component]
fn Home() -> Element {
    use_context_provider(|| Signal::new(MainState::new()));
    rsx! {
        MyLayout {}
    }
}

#[component]
fn Actions(service: String) -> Element {
    use_context_provider(|| Signal::new(MainState::new_with_selected_service(service)));
    rsx! {
        MyLayout {}
    }
}

#[component]
fn LastEvents(service: String, action: String) -> Element {
    use_context_provider(|| {
        Signal::new(MainState::new_with_selected_action(
            service,
            from_base_64(action.as_str()),
        ))
    });
    rsx! {
        MyLayout {}
    }
}

#[component]
fn Process(service: String, action: String, id: i64) -> Element {
    use_context_provider(|| {
        Signal::new(MainState::new_with_selected_process(
            service,
            from_base_64(action.as_str()),
            id,
        ))
    });
    rsx! {
        MyLayout {}
    }
}

#[component]
pub fn MyLayout() -> Element {
    let mut main_state = consume_context::<Signal<MainState>>();

    if main_state.read().files.initialized() {
        return rsx! {
            div { id: "layout",
                div { id: "left-panel", LeftPanel {} }

                div { id: "right-panel", RightPanel {} }
                div { id: "top-panel", RenderTopPanel {} }
                div { id: "bottom-panel", RenderBottomPanel {} }
                dialog::RenderDialog {}
            }
        };
    }

    // The available-hours list is the one thing the whole UI is keyed on - nothing
    // can be rendered before the server has told us which hours it holds and what
    // their hour keys are.
    let mut loading_state = use_signal(|| DataState::None);
    let loading_state_read_access = loading_state.read();

    match loading_state_read_access.as_ref() {
        DataState::None => {
            spawn(async move {
                loading_state.set(DataState::Loading);

                match crate::api::get_available_hours().await {
                    Ok(hours) => {
                        main_state.write().files.set_files(hours);
                        loading_state.set(DataState::Loaded(()));
                    }
                    Err(err) => {
                        loading_state.set(DataState::Error(err.to_string()));
                    }
                }
            });
            render_loading()
        }

        DataState::Loading => render_loading(),

        DataState::Loaded(_) => render_loading(),

        DataState::Error(err) => rsx! { "Error loading available hours: {err}" },
    }
}

fn render_loading() -> Element {
    rsx! { "Loading..." }
}
