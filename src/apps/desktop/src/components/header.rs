use dioxus::prelude::*;

use crate::router::Route;

#[component]
pub fn Header() -> Element {
    rsx! {
        header { class: "app-header",
            div { class: "header-left",
                Link { to: Route::RaceList {}, class: "header-title", "paddock" }
            }
        }
    }
}
