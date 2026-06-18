//! 確率推定で使う重み・キャップ・prior の定数群。値の根拠は各 doc コメント（backtest/ADR 参照）。

use super::model::RateTriple;

pub(crate) const COURSE_GATE_WEIGHT: f64 = 2.0;
pub(crate) const SURFACE_WEIGHT: f64 = 1.0;
pub(crate) const DISTANCE_WEIGHT: f64 = 1.0;
pub(crate) const JOCKEY_WEIGHT: f64 = 1.0;
/// 調教師（trainer）項の重み。#87 で母数（results.trainer）を充足し backtest（0.0/0.5/1.0/2.0 を
/// 比較, 2026-03-28〜05-31 / 144 レース, #81 後ロジック）で再検証した。項を有効化すると校正が改善
/// （0.0→0.5 で LogLoss 単勝 0.60→0.40、Brier 系は小幅）。0.5/1.0/2.0 は拮抗で、1.0 が LogLoss 単勝・
/// Brier 複勝で最良（Brier 単勝のみ 2.0 が僅差だが小標本ゆえ過適合回避で 1.0）。jockey と同値（ADR 0012）。
pub(crate) const TRAINER_WEIGHT: f64 = 1.0;
/// 馬場状態（track_condition）項の重み。#73 バックテスト（0.25/0.5/1.0/1.5/2.0 を比較）で
/// 1.0 が的中率・回収率のピークだったため採用（ADR 0011）。
pub(crate) const TRACK_CONDITION_WEIGHT: f64 = 1.0;
/// 斤量（レース内相対）項の重み（#135）。backtest（main との before/after・両符号, 2026-03-28〜05-31
/// / 144R）で「重い→加点」採用時に 0.25 で連対 +4.1pt・複勝 +4.1pt・回収 +6.8pt・単勝 LogLoss
/// 0.3144→0.2486 と全面改善を確認したため採用（ADR 0009 追補）。recent_form と同値の保守値。
pub(crate) const WEIGHT_CARRIED_WEIGHT: f64 = 0.25;
/// 前走フォーム項の重み。#30 バックテストで検証して決定（ADR 0009）。
pub(crate) const FORM_WEIGHT: f64 = 0.25;

/// ベイズ縮約（#75）の母集団 prior レート。出走頭数の代表値（≒14 頭）から導く解析的な基準率
/// （win=1/14, place=2/14, show=3/14）で、「平均的な 1 頭が 1 着/2 着内/3 着内に入る確率」に相当する。
/// 実績の薄い factor のレートをこの prior へ寄せる。クエリ不要でリークが無い最小実装。将来は
/// results 全体の実測ベースレートへ差し替え可能（backtest で要否を再検証）。
pub(crate) const PRIOR_RATE: RateTriple = RateTriple {
    win: 1.0 / 14.0,
    place: 2.0 / 14.0,
    show: 3.0 / 14.0,
};

/// 馬体重変化がこの kg を超えると不安定として最低評価（0）にする。
pub(crate) const WEIGHT_CHANGE_CAP: f64 = 20.0;
/// 前走の人気順位と着順の差 1 つあたりのスコア寄与。
pub(crate) const POP_GAP_K: f64 = 0.08;
/// 前走着差（馬身）がこの値以上で競争力差を最大とみなすクランプ点（大差勝ち・大敗の上限, #76）。
/// 暫定値。backtest（main との before/after 比較）で寄与を確認して調整する。
pub(crate) const MARGIN_CAP_LENGTHS: f64 = 5.0;
/// 斤量シグナル（#135）の飽和上限[kg]。field 平均斤量からの差がこの kg で signal が 0/1 に飽和する。
/// レース内の斤量差は数 kg に収まるため小さめに置く。暫定値で backtest（main との before/after）で調整する。
pub(crate) const WEIGHT_CARRIED_CAP_KG: f64 = 3.0;
/// 前走タイムの相対速度 signal（#76）の飽和上限。標準タイムからの相対偏差
/// `(standard - prev) / standard` がこの割合（例 0.05 = ±5%）で signal が 0/1 に飽和する。
/// レース内のタイム差は数 % に収まるため小さめに置く。暫定値で backtest（main との before/after）
/// で寄与を確認して調整する。
pub(crate) const TIME_DEV_CAP: f64 = 0.05;
