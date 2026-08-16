use dioxus::prelude::*;

use super::ShowKeyValue;
use crate::states::{DialogState, MainState};

#[component]
pub fn RenderDialog() -> Element {
    let main_state = consume_context::<Signal<MainState>>();

    let dialog = main_state.read();

    let dialog = match dialog.get_dialog_state() {
        None => rsx! {},
        Some(dialog_state) => {
            let dialog_class = "modal-dialog";

            let (dialog_content, header) = match dialog_state {
                DialogState::ShowKeyValue { the_key, value } => {
                    let result = rsx! {
                        ShowKeyValue { value: value.clone() }
                    };
                    (result, format!("Show Value for Key {}", the_key.as_str()))
                }
            };

            rsx! {
                div { id: "dialog-pad",

                    div { class: "{dialog_class}",
                        div { class: "modal-content",
                            div { class: "modal-header",
                                h5 { class: "modal-title", "{header}" }
                                button {
                                    r#type: "button",
                                    class: "btn btn-default btn-sm",
                                    onclick: move |_| {
                                        consume_context::<Signal<MainState>>().write().hide_dialog();
                                    },
                                    "X"
                                }
                            }
                            {dialog_content}
                        }
                    }
                }
            }
        }
    };

    dialog
}
