use chrono::Local;
use dioxus::prelude::*;

use crate::router::Route;

#[component]
pub fn Header() -> Element {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let mut date_input = use_signal(|| today);
    let nav = use_navigator();

    rsx! {
        header { class: "app-header",
            div { class: "header-left",
                Link { to: Route::RaceList {}, class: "header-title", "paddock" }
            }
            div { class: "header-right",
                input {
                    r#type: "date",
                    class: "date-picker",
                    value: "{date_input}",
                    onchange: move |e| {
                        let v: String = e.value();
                        date_input.set(v);
                        nav.push(Route::RaceList {});
                    },
                }
            }
        }
    }
}
