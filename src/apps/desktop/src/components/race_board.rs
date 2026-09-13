use std::sync::Arc;

use dioxus::prelude::*;
use paddock_domain::race::RaceId;

use crate::components::horse_card::HorseCard;
use crate::setup::Setup;
use crate::viewmodel::race::BoardView;

const DEFAULT_BUDGET: u64 = 5000;
const DEFAULT_ALPHA: f64 = 0.2;

#[component]
pub fn RaceBoard(race_id: String) -> Element {
    let setup = use_context::<Arc<Setup>>();
    let rid = race_id.clone();

    let board = use_resource(move || {
        let setup = setup.clone();
        let rid = rid.clone();
        async move {
            let race_id = RaceId::try_from(rid).map_err(|e| anyhow::anyhow!("{e}"))?;
            setup
                .interactor
                .race_board(&race_id, DEFAULT_BUDGET, Some(DEFAULT_ALPHA), None)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
    });

    match &*board.read_unchecked() {
        Some(Ok(b)) => {
            let view = BoardView::from(b);
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
                    div { class: "board-grid",
                        for horse in view.horses.iter() {
                            HorseCard { horse: horse.clone() }
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "エラー: {e}" } },
        None => rsx! { p { class: "loading", "読み込み中…" } },
    }
}
