use dioxus::prelude::*;
use rust_extensions::StrOrString;

use crate::states::MainState;

#[component]
pub fn SelectHourKey() -> Element {
    let mut main_state = consume_context::<Signal<MainState>>();

    let read_access = main_state.read();

    let selected_hours_ago = read_access.try_get_hours_ago();

    let mut items_to_render = Vec::new();

    if let Some(files) = read_access.files.get_files() {
        for file in files {
            items_to_render.push(render_name(
                file.hours_ago,
                selected_hours_ago,
                file.file_size,
            ));
        }
    } else {
        items_to_render.push(render_name(0, selected_hours_ago, 0));
    }

    rsx! {
        select {
            class: "form-select-sm",
            style: "width:100%;border-color: lightgray;",
            oninput: move |e| {
                let value = e.value().parse::<i64>().unwrap_or_default();
                main_state.write().set_hours_ago(value);
            },
            {items_to_render.into_iter()}
        }
    }
}

fn render_name(hours_ago: i64, selected: Option<i64>, file_size: u64) -> Element {
    let name: StrOrString = match hours_ago {
        0 => "Current Hour".into(),
        1 => "1 Hour ago".into(),
        _ => format!("{} Hours ago", hours_ago).into(),
    };

    let selected = if let Some(selected) = selected {
        hours_ago == selected
    } else {
        false
    };

    if file_size == 0 {
        return rsx! {
            option { selected, value: hours_ago.to_string(), {name.as_str()} }
        };
    }

    let file_size = format_file_size(file_size);

    let name = format!("{} [{}]", name.as_str(), file_size);

    rsx! {
        option { selected, value: hours_ago.to_string(), {name} }
    }
}

fn format_file_size(file_size: u64) -> String {
    if file_size < 1024 {
        return format!("{} B", file_size);
    }

    let file_size = file_size as f64 / 1024.0;

    if file_size < 1024.0 {
        return format!("{:.3} KB", file_size);
    }

    let file_size = file_size / 1024.0;

    if file_size < 1024.0 {
        return format!("{:.3} MB", file_size);
    }

    let file_size = file_size / 1024.0;

    format!("{:.3} GB", file_size)
}
