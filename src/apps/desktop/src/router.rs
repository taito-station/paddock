use dioxus::prelude::*;

use crate::components::{Header, RaceBoard, RaceList};

#[derive(Routable, Clone, PartialEq)]
pub enum Route {
    #[layout(AppLayout)]
    #[route("/")]
    RaceList {},
    #[route("/board/:race_id")]
    RaceBoard { race_id: String },
}

#[component]
fn AppLayout() -> Element {
    rsx! {
        Header {}
        main { class: "app-main", Outlet::<Route> {} }
    }
}
