use dioxus::prelude::*;

use crate::{states::MainState, AppRoute};

/// Which product scope of the env the UI reads.
#[component]
pub fn SelectScope() -> Element {
    let mut main_state = consume_context::<Signal<MainState>>();

    let read_access = main_state.read();

    let selected = read_access.get_scope();

    let scopes = read_access
        .scopes
        .iter()
        .flat_map(|scopes| scopes.scopes.iter());

    let options = scopes.map(|scope| {
        let is_selected = selected
            .as_ref()
            .map_or(false, |selected| selected.as_str() == scope.as_str());

        rsx! {
            option { selected: is_selected, value: scope.as_str(), {scope.as_str()} }
        }
    });

    rsx! {
        select {
            class: "form-select-sm",
            style: "border-color: lightgray;",
            title: "Product scope",
            oninput: move |e| {
                let value = e.value();
                main_state.write().set_scope(value.as_str());
                // A service or a process selected in the route belongs to the scope before
                navigator().push(AppRoute::Home);
            },
            {options}
        }
    }
}
