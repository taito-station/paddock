use std::sync::Arc;

use chrono::Local;
use dioxus::prelude::*;

use crate::router::Route;
use crate::setup::Setup;
use crate::viewmodel::list::RaceRow;

#[component]
pub fn RaceList() -> Element {
    let setup = use_context::<Arc<Setup>>();
    let today = Local::now().date_naive();
    let date = use_signal(|| today);

    let races = use_resource(move || {
        let setup = setup.clone();
        let d = *date.read();
        async move { setup.interactor.races_by_date(d).await }
    });

    match &*races.read() {
        Some(Ok(list)) => {
            let rows: Vec<RaceRow> = list.iter().map(RaceRow::from).collect();
            rsx! {
                div { class: "race-list",
                    h2 { class: "section-title", "{date.read()}" }
                    if rows.is_empty() {
                        p { class: "empty", "この日のレースはありません" }
                    } else {
                        table { class: "race-table",
                            thead {
                                tr {
                                    th { "R" }
                                    th { "開催" }
                                    th { "芝ダ" }
                                    th { "距離" }
                                }
                            }
                            tbody {
                                for row in rows {
                                    {race_row(row)}
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "エラー: {e}" } },
        None => rsx! { p { class: "loading", "読み込み中…" } },
    }
}

fn race_row(row: RaceRow) -> Element {
    let race_id = row.race_id.clone();
    rsx! {
        tr {
            class: "race-row",
            onclick: move |_| {
                let nav = navigator();
                nav.push(Route::RaceBoard { race_id: race_id.clone() });
            },
            td { "{row.race_num}" }
            td { "{row.venue}" }
            td { "{row.surface}" }
            td { "{row.distance}m" }
        }
    }
}
