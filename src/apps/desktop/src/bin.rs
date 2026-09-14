mod components;
mod router;
mod setup;
mod viewmodel;

use std::sync::Arc;

use chrono::Local;
use dioxus::prelude::*;

use crate::router::Route;

const STYLES: Asset = asset!("/src/assets/styles.css");

fn main() {
    LaunchBuilder::new()
        .with_cfg(desktop! {
            dioxus::desktop::Config::new().with_window(
                dioxus::desktop::WindowBuilder::new()
                    .with_title("paddock")
                    .with_inner_size(dioxus::desktop::LogicalSize::new(1400.0, 900.0)),
            )
        })
        .launch(App);
}

#[component]
fn App() -> Element {
    let setup = use_hook(|| {
        let s = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(setup::build())
        });
        Arc::new(s.expect("DB connection failed"))
    });

    let initial_date = use_hook(|| {
        let s = setup.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                s.interactor
                    .race_dates()
                    .await
                    .ok()
                    .and_then(|v| v.into_iter().next())
                    .unwrap_or_else(|| Local::now().date_naive())
            })
        })
    });

    use_context_provider(|| setup.clone());
    use_context_provider(|| Signal::new(initial_date));

    rsx! {
        document::Stylesheet { href: STYLES }
        Router::<Route> {}
    }
}
