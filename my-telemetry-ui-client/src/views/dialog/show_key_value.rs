use std::rc::Rc;

use dioxus::prelude::*;

#[component]
pub fn ShowKeyValue(value: Rc<String>) -> Element {
    rsx! {
        div { {value.as_str()} }
    }
}
