use std::sync::Arc;

use chrono::NaiveDate;
use dioxus::prelude::*;
use paddock_domain::race::RaceId;
use paddock_use_case::repository::session::PredictBetRecord;

use crate::setup::Setup;
use crate::viewmodel::DEFAULT_BUDGET;
use crate::viewmodel::exec::{BetView, SessionView};

#[derive(Debug, Clone, PartialEq)]
enum PanelState {
    Loading,
    NoSession,
    Ready,
    Recorded,
    Skipped,
    Error(String),
}

#[component]
pub fn ExecPanel(race_id: String, date: NaiveDate, bets: Vec<BetView>) -> Element {
    let setup = use_context::<Arc<Setup>>();
    let mut panel_state = use_signal(|| PanelState::Loading);
    let mut session = use_signal::<Option<SessionView>>(|| None);
    let mut working = use_signal(|| false);

    let setup_init = setup.clone();
    let rid_init = race_id.clone();
    let _init = use_resource(move || {
        let setup = setup_init.clone();
        let rid = rid_init.clone();
        async move {
            let sess = setup.interactor.find_predict_session(date).await;
            match sess {
                Ok(None) => {
                    panel_state.set(PanelState::NoSession);
                }
                Ok(Some(s)) => {
                    session.set(Some(SessionView::from(&s)));
                    let race_id = match RaceId::try_from(rid) {
                        Ok(r) => r,
                        Err(e) => {
                            panel_state.set(PanelState::Error(format!("{e}")));
                            return;
                        }
                    };
                    let bets_result = setup.interactor.find_predict_bets(date).await;
                    let skips_result = setup.interactor.find_predict_race_skips(date).await;
                    match (bets_result, skips_result) {
                        (Ok(recorded_bets), Ok(skips)) => {
                            let has_bets = recorded_bets.iter().any(|b| b.race_id == race_id);
                            let is_skipped = skips.contains(&race_id);
                            if has_bets {
                                panel_state.set(PanelState::Recorded);
                            } else if is_skipped {
                                panel_state.set(PanelState::Skipped);
                            } else {
                                panel_state.set(PanelState::Ready);
                            }
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            panel_state.set(PanelState::Error(format!("{e}")));
                        }
                    }
                }
                Err(e) => {
                    panel_state.set(PanelState::Error(format!("{e}")));
                }
            }
        }
    });

    let setup_create = setup.clone();
    let on_create_session = move |_: Event<MouseData>| {
        let setup = setup_create.clone();
        spawn(async move {
            working.set(true);
            match setup
                .interactor
                .create_predict_session(date, DEFAULT_BUDGET)
                .await
            {
                Ok(s) => {
                    session.set(Some(SessionView::from(&s)));
                    panel_state.set(PanelState::Ready);
                }
                Err(e) => panel_state.set(PanelState::Error(format!("{e}"))),
            }
            working.set(false);
        });
    };

    let setup_record = setup.clone();
    let rid_record = race_id.clone();
    let bets_for_record = bets.clone();
    let on_record = move |_: Event<MouseData>| {
        let setup = setup_record.clone();
        let rid = rid_record.clone();
        let bets = bets_for_record.clone();
        spawn(async move {
            working.set(true);
            let race_id = match RaceId::try_from(rid) {
                Ok(r) => r,
                Err(e) => {
                    panel_state.set(PanelState::Error(format!("{e}")));
                    working.set(false);
                    return;
                }
            };
            let bet_records: Vec<PredictBetRecord> = bets
                .iter()
                .map(|b| PredictBetRecord {
                    race_id: race_id.clone(),
                    bet_type: b.type_slug.clone(),
                    combination: b.combination.clone(),
                    stake: b.stake,
                    payout: 0,
                    ev: b.ev_raw,
                })
                .collect();
            match setup
                .interactor
                .record_race_outcome(date, &race_id, bet_records)
                .await
            {
                Ok(s) => {
                    session.set(Some(SessionView::from(&s)));
                    panel_state.set(PanelState::Recorded);
                }
                Err(e) => panel_state.set(PanelState::Error(format!("{e}"))),
            }
            working.set(false);
        });
    };

    let setup_skip = setup.clone();
    let rid_skip = race_id.clone();
    let on_skip = move |_: Event<MouseData>| {
        let setup = setup_skip.clone();
        let rid = rid_skip.clone();
        spawn(async move {
            working.set(true);
            let race_id = match RaceId::try_from(rid) {
                Ok(r) => r,
                Err(e) => {
                    panel_state.set(PanelState::Error(format!("{e}")));
                    working.set(false);
                    return;
                }
            };
            match setup
                .interactor
                .record_race_outcome(date, &race_id, vec![])
                .await
            {
                Ok(s) => {
                    session.set(Some(SessionView::from(&s)));
                    panel_state.set(PanelState::Skipped);
                }
                Err(e) => panel_state.set(PanelState::Error(format!("{e}"))),
            }
            working.set(false);
        });
    };

    let state = panel_state.read().clone();
    let is_working = *working.read();

    rsx! {
        div { class: "exec-panel",
            h3 { class: "exec-title", "執行" }
            if let Some(ref s) = *session.read() {
                div { class: "session-info",
                    span { "予算 ¥{s.budget}" }
                    span { "残高 ¥{s.balance}" }
                    span { "累計 ¥{s.total_bet}" }
                }
            }
            match state {
                PanelState::Loading => rsx! {
                    p { class: "loading", "読み込み中…" }
                },
                PanelState::NoSession => rsx! {
                    div { class: "exec-actions",
                        button {
                            class: "btn btn-primary",
                            disabled: is_working,
                            onclick: on_create_session,
                            "セッション作成 (¥{DEFAULT_BUDGET})"
                        }
                    }
                },
                PanelState::Ready => rsx! {
                    div { class: "exec-actions",
                        button {
                            class: "btn btn-primary",
                            disabled: is_working || bets.is_empty(),
                            onclick: on_record,
                            "記録する"
                        }
                        button {
                            class: "btn btn-secondary",
                            disabled: is_working,
                            onclick: on_skip,
                            "見送り"
                        }
                    }
                },
                PanelState::Recorded => rsx! {
                    span { class: "chip recorded", "購入済み" }
                },
                PanelState::Skipped => rsx! {
                    span { class: "chip skipped", "見送り" }
                },
                PanelState::Error(ref e) => rsx! {
                    p { class: "error", "エラー: {e}" }
                },
            }
        }
    }
}
