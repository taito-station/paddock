use std::sync::Arc;

use chrono::{Datelike, Local, NaiveDate};
use dioxus::prelude::*;
use paddock_domain::race::RaceId;

use crate::router::Route;
use crate::setup::Setup;
use crate::viewmodel::list::{RaceRow, RaceStatus};

const RESULT_POLL_SECS: u64 = 45;

#[component]
pub fn RaceList() -> Element {
    let setup = use_context::<Arc<Setup>>();
    let today = Local::now().date_naive();
    let mut date = use_context::<Signal<NaiveDate>>();
    let mut refresh_tick = use_signal(|| 0u64);
    let mut results_refreshing = use_signal(|| false);
    let mut date_picker_open = use_signal(|| false);

    let setup_dates = setup.clone();
    let available_dates = use_resource(move || {
        let setup = setup_dates.clone();
        async move { setup.interactor.race_dates().await.unwrap_or_default() }
    });

    let setup_res = setup.clone();
    let races = use_resource(move || {
        let setup = setup_res.clone();
        let d = *date.read();
        let _tick = *refresh_tick.read();
        async move {
            let list = setup.interactor.races_by_date(d).await?;
            let confirmed = setup.interactor.result_confirmed_by_date(d).await?;
            let bets = setup.interactor.find_predict_bets(d).await?;
            let skips = setup.interactor.find_predict_race_skips(d).await?;

            let bet_race_ids: std::collections::HashSet<String> =
                bets.iter().map(|b| b.race_id.value().to_string()).collect();
            let skip_set: std::collections::HashSet<RaceId> = skips.into_iter().collect();

            let rows: Vec<RaceRow> = list
                .iter()
                .map(|r| {
                    let mut row = RaceRow::from(r);
                    if confirmed.get(&r.race_id).copied().unwrap_or(false) {
                        row.status = RaceStatus::Confirmed;
                    } else if bet_race_ids.contains(r.race_id.value()) {
                        row.status = RaceStatus::Recorded;
                    } else if skip_set.contains(&r.race_id) {
                        row.status = RaceStatus::Skipped;
                    }
                    row
                })
                .collect();
            Ok::<_, anyhow::Error>(rows)
        }
    });

    let _auto_poll = use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        if *date.read() != today {
            return;
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(RESULT_POLL_SECS)).await;
            refresh_tick += 1;
        }
    });

    let setup_refresh = setup.clone();
    let on_refresh_results = move |_: Event<MouseData>| {
        let setup = setup_refresh.clone();
        let d = *date.read();
        spawn(async move {
            results_refreshing.set(true);
            let _ = setup.results.refresh(d, false).await;
            results_refreshing.set(false);
            refresh_tick += 1;
        });
    };

    let all_dates = available_dates
        .read()
        .as_ref()
        .cloned()
        .unwrap_or_default();
    let current_date = *date.read();

    match &*races.read() {
        Some(Ok(rows)) => {
            rsx! {
                div { class: "race-list",
                    div { class: "race-list-header",
                        button {
                            class: "date-heading-btn",
                            onclick: move |_| {
                                date_picker_open.set(!date_picker_open());
                            },
                            "{current_date} ▾"
                        }
                        button {
                            class: if *results_refreshing.read() { "refresh-btn refreshing" } else { "refresh-btn" },
                            disabled: *results_refreshing.read(),
                            onclick: on_refresh_results,
                            "結果取り込み"
                        }
                    }
                    if *date_picker_open.read() {
                        {render_date_picker(&all_dates, current_date, &mut date, &mut date_picker_open)}
                    }
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
                                    th { "状態" }
                                }
                            }
                            tbody {
                                for row in rows.iter().cloned() {
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

fn render_date_picker(
    all_dates: &[NaiveDate],
    current: NaiveDate,
    date: &mut Signal<NaiveDate>,
    open: &mut Signal<bool>,
) -> Element {
    if all_dates.is_empty() {
        return rsx! { p { class: "empty", "データがありません" } };
    }

    let mut year_months: Vec<(i32, u32)> = all_dates
        .iter()
        .map(|d| (d.year(), d.month()))
        .collect();
    year_months.dedup();

    let mut date = *date;
    let mut open = *open;

    let mut prev_year: Option<i32> = None;

    rsx! {
        div { class: "date-picker-panel",
            for (year, month) in year_months.iter() {
                {
                    let show_year = prev_year != Some(*year);
                    prev_year = Some(*year);
                    let y = *year;
                    let m = *month;
                    rsx! {
                        if show_year {
                            h4 { class: "date-year-label", "{y}" }
                        }
                        div { class: "date-month-row",
                            span { class: "date-month-label", "{m}月" }
                            div { class: "date-chips",
                                for d in all_dates.iter().filter(|d| d.year() == y && d.month() == m) {
                                    {
                                        let d_val = *d;
                                        let day = d.day();
                                        let dow = d.format("%a").to_string();
                                        let is_current = d_val == current;
                                        rsx! {
                                            button {
                                                class: if is_current { "date-chip active" } else { "date-chip" },
                                                onclick: move |_| {
                                                    date.set(d_val);
                                                    open.set(false);
                                                },
                                                "{day}({dow})"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn race_row(row: RaceRow) -> Element {
    let race_id = row.race_id.clone();
    let status_class = row.status.css_class();
    let status_label = row.status.label();
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
            td {
                if !status_class.is_empty() {
                    span { class: "chip {status_class}", "{status_label}" }
                } else {
                    span { class: "status-empty", "{status_label}" }
                }
            }
        }
    }
}
