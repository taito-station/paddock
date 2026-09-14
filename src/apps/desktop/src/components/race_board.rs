use std::sync::Arc;

use chrono::Local;
use dioxus::prelude::*;
use paddock_domain::race::RaceId;

use crate::components::exec_panel::ExecPanel;
use crate::components::horse_card::HorseCard;
use crate::setup::Setup;
use crate::viewmodel::DEFAULT_BUDGET;
use crate::viewmodel::exec::BetView;
use crate::viewmodel::race::BoardView;
const DEFAULT_ALPHA: f64 = 0.2;
const POLL_SECS: u64 = 60;

#[component]
pub fn RaceBoard(race_id: String) -> Element {
    let setup = use_context::<Arc<Setup>>();
    let mut refresh_tick = use_signal(|| 0u64);
    let mut odds_refreshing = use_signal(|| false);

    let rid = race_id.clone();
    let setup_res = setup.clone();
    let board = use_resource(move || {
        let setup = setup_res.clone();
        let rid = rid.clone();
        let _tick = *refresh_tick.read();
        async move {
            let race_id = RaceId::try_from(rid).map_err(|e| anyhow::anyhow!("{e}"))?;
            setup
                .interactor
                .race_board(&race_id, DEFAULT_BUDGET, Some(DEFAULT_ALPHA), None)
                .await
                .map(|b| BoardView::from(&b))
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
    });

    let _auto_poll = use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
            if let Some(Ok(view)) = &*board.read_unchecked()
                && (view.result_confirmed || post_time_passed(view))
            {
                break;
            }
            refresh_tick += 1;
        }
    });

    let setup_odds = setup.clone();
    let rid_odds = race_id.clone();
    let on_refresh_odds = move |_: Event<MouseData>| {
        let setup = setup_odds.clone();
        let rid = rid_odds.clone();
        spawn(async move {
            odds_refreshing.set(true);
            if let Ok(race_id) = RaceId::try_from(rid)
                && let Err(e) = setup.odds.refresh_race_odds(&race_id).await
            {
                tracing::warn!("オッズ更新失敗: {e}");
            }
            odds_refreshing.set(false);
            refresh_tick += 1;
        });
    };

    match &*board.read_unchecked() {
        Some(Ok(view)) => {
            let should_poll = !view.result_confirmed && !post_time_passed(view);
            rsx! {
                div { class: "board-view",
                    div { class: "board-header",
                        h2 { class: "board-title",
                            "{view.race_num}R {view.venue} {view.surface}{view.distance}m"
                        }
                        if let Some(ref name) = view.race_name {
                            span { class: "race-name-badge", "{name}" }
                        }
                        if let Some(ref cls) = view.race_class {
                            span { class: "race-class-badge", "{cls}" }
                        }
                        if let Some(ref pt) = view.post_time {
                            span { class: "post-time", "発走 {pt}" }
                        }
                        if should_poll {
                            button {
                                class: if *odds_refreshing.read() { "refresh-btn refreshing" } else { "refresh-btn" },
                                disabled: *odds_refreshing.read(),
                                onclick: on_refresh_odds,
                                "オッズ更新"
                            }
                        }
                    }
                    div { class: "board-summary",
                        if view.is_confused {
                            span { class: "chip konsen", "混戦" }
                        }
                        span { class: "axis-prob",
                            "◎勝率 {view.axis_win_prob * 100.0:.1}%"
                        }
                        span { class: "qualifying",
                            "qualifying {view.qualifying_count}頭"
                        }
                        if view.result_confirmed {
                            span { class: "chip result-done", "確定" }
                        }
                    }
                    if !view.bets.is_empty() {
                        {bet_table(&view.bets)}
                    }
                    div { class: "board-grid",
                        for horse in view.horses.iter() {
                            HorseCard { horse: horse.clone() }
                        }
                    }
                    ExecPanel {
                        race_id: view.race_id.clone(),
                        date: view.date,
                        bets: view.bets.clone(),
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "エラー: {e}" } },
        None => rsx! { p { class: "loading", "読み込み中…" } },
    }
}

fn post_time_passed(view: &BoardView) -> bool {
    let Some(pt) = view.post_time_raw else {
        return false;
    };
    let race_dt = chrono::NaiveDateTime::new(view.date, pt);
    Local::now().naive_local() > race_dt
}

fn bet_table(bets: &[BetView]) -> Element {
    let total: u64 = bets.iter().map(|b| b.stake).sum();
    rsx! {
        div { class: "bet-section",
            h3 { class: "bet-section-title", "買い目 (計 ¥{total})" }
            table { class: "bet-table",
                thead {
                    tr {
                        th { "券種" }
                        th { "組合せ" }
                        th { "金額" }
                        th { "EV" }
                        th { "的中率" }
                    }
                }
                tbody {
                    for bet in bets.iter() {
                        tr {
                            td { "{bet.bet_type}" }
                            td { class: "mono", "{bet.combination}" }
                            td { class: "num", "¥{bet.stake}" }
                            td { class: "num", "{bet.ev}" }
                            td { class: "num", "{bet.hit_prob}" }
                        }
                    }
                }
            }
        }
    }
}
