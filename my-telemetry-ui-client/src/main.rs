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

    // The env, its product scope and the available-hours list of the scope are what the
    // whole UI is keyed on - nothing can be rendered before the server has told us which
    // my-telemetry-servers it reads, which product scopes the selected one has, which
    // hours the selected scope holds and what their hour keys are. A change of the env or
    // of the scope resets `loading`, which brings us back here.
    let loading = main_state.read().loading.clone();

    match loading {
        DataState::None => {
            spawn(async move {
                main_state.write().loading = DataState::Loading;

                let loaded = match load_env_and_hours(main_state).await {
                    Ok(()) => DataState::Loaded(()),
                    Err(err) => DataState::Error(err),
                };

                main_state.write().loading = loaded;
            });
            render_loading()
        }

        DataState::Loading => render_loading(),

        DataState::Loaded(_) => render_loading(),

        DataState::Error(err) => rsx! { "Error loading available hours: {err}" },
    }
}

async fn load_env_and_hours(mut main_state: Signal<MainState>) -> Result<(), String> {
    let has_envs = main_state.read().envs.is_some();

    if !has_envs {
        let app_info = crate::api::get_app_info()
            .await
            .map_err(|err| err.to_string())?;

        main_state.write().set_envs(app_info.envs);
    }

    let env = main_state.read().get_env();

    let Some(env) = env else {
        return Err("my-telemetry-ui has no envs in its settings".to_string());
    };

    let has_scopes = main_state.read().scopes.is_some();

    if !has_scopes {
        let scopes = crate::api::get_product_scopes(env.as_str())
            .await
            .map_err(|err| err.to_string())?;

        main_state.write().set_scopes(scopes);
    }

    let scope = main_state.read().get_scope();

    let Some(scope) = scope else {
        return Err("The product scopes are not loaded".to_string());
    };

    let hours = crate::api::get_available_hours(env.as_str(), scope.as_str())
        .await
        .map_err(|err| err.to_string())?;

    main_state.write().files.set_files(hours);

    Ok(())
}

fn render_loading() -> Element {
    rsx! { "Loading..." }
}
