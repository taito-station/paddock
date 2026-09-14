use dioxus::prelude::*;

use crate::viewmodel::race::HorseView;

#[component]
pub fn HorseCard(horse: HorseView) -> Element {
    let mark_class = if !horse.mark.is_empty() {
        "has-mark"
    } else {
        ""
    };
    let overlay_class = if horse.is_overlay { "overlay" } else { "" };
    let value_class = if horse.is_value { "value" } else { "" };

    rsx! {
        div { class: "horse-card {mark_class} {overlay_class} {value_class}",
            div { class: "horse-header",
                span { class: "horse-mark", "{horse.mark}" }
                span { class: "horse-num", "{horse.horse_num}" }
                span { class: "horse-name", "{horse.horse_name}" }
            }
            div { class: "horse-jockey", "{horse.jockey}" }
            div { class: "horse-probs",
                div { class: "prob-row",
                    span { class: "prob-label", "勝率" }
                    span { class: "prob-value", "{horse.win_prob}" }
                }
                div { class: "prob-row",
                    span { class: "prob-label", "モデル" }
                    span { class: "prob-value", "{horse.pure_win_prob}" }
                }
                div { class: "prob-row",
                    span { class: "prob-label", "市場" }
                    span { class: "prob-value", "{horse.market_implied}" }
                }
            }
            div { class: "horse-odds",
                span { class: "odds-label", "単勝" }
                span { class: "odds-value", "{horse.win_odds}" }
            }
            if let Some(pos) = horse.finishing_position {
                div { class: "horse-result", "{pos}着" }
            }
            if let Some(ref comment) = horse.comment {
                div { class: "horse-comment", "{comment}" }
            }
        }
    }
}
