// UI 横断の定数（複数画面で共有するマジックナンバーの単一ソース・#417）。
// ドメインロジック固有の閾値は各 lib モジュール側（live.ts の STALE_MINUTES 等）に置き、
// ここは「画面の挙動」に属する値（ポーリング間隔・tick・既定入力値）だけを集約する。

// レース一覧のライブ EV スナップショット自動再取得の間隔（ミリ秒）。
// predict-watch のスイープ（5 分間隔）を拾うため、未発走が残る間だけこの間隔で再取得する（#372）。
export const RACE_LIST_POLL_INTERVAL_MS = 60_000;

// 発走済み判定・鮮度の相対時刻を更新する tick 間隔（ミリ秒）。
// 「現在時刻」state を刻んで再描画するだけの表示用 tick（RaceList / SessionSummary で共有）。
export const CLOCK_TICK_INTERVAL_MS = 30_000;

// セッション作成フォームの既定予算（円）。number input の文字列 state に合わせて文字列で持つ。
export const DEFAULT_SESSION_BUDGET = "10000";
