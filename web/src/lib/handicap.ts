// 手動ハンデ精査の材料（#628）の表示用純関数群。
//
// 現時点で実在が確認できているエッジは「手動のハンデ精査」と「執行の規律（軸ロック＋ズレ増額）」の
// 2 つだけ（ADR 0055 / 0060 / 0076）。盤は前者が使う**事実**を出すだけで、閾値で go/no-go は出さない
// （ADR 0079 と同じ理由——バッジが go シグナルとして誤読される事故を作らない）。
// 確率・買い目には一切影響しない（ADR 0055 の「確率と買い方の分離」）。

import type { components } from "../api/schema";
import { SURFACE_JP, VENUE_JP } from "./format";

export type HandicapNote = components["schemas"]["HandicapNoteSchema"];
export type ConditionRun = components["schemas"]["ConditionRunSchema"];

// 完全一致が 0 走のときの明示表記。**空欄と区別する**——空欄は「まだ引いていない」に見える。
export const NO_RECORD = "該当なし";

// 「人気馬の前提が壊れているサイン」(2) を出す市場人気の上限。全頭に出すとノイズになるため
// 上位人気だけに絞る（issue #628 の要件）。詳細パネルは全頭で出す（絞るのはカードの密度のため）。
export const PREMISE_POPULARITY_MAX = 3;

// 今回条件のラベル（例 `新潟芝1000m`）。venue は API のスラッグ。
export function conditionLabel(
  venue: string,
  surface: string,
  distance: number,
): string {
  const v = VENUE_JP[venue] ?? venue;
  const s = SURFACE_JP[surface] ?? surface;
  return `${v}${s}${distance}m`;
}

// 場グループのラベル（例 `洋芝(札幌/函館)芝2000m`）。`groupVenues` が空＝グループが当場のみで
// 完全一致と同じ集合なので `null`（2 行目を出さない）。多場グループは洋芝（札幌・函館）だけで、
// これはサーバ側 `Venue::turf_group` が決める（web は表記だけを持つ）。
export function groupConditionLabel(
  groupVenues: readonly string[],
  surface: string,
  distance: number,
): string | null {
  if (groupVenues.length === 0) return null;
  const names = groupVenues.map((v) => VENUE_JP[v] ?? v).join("/");
  const s = SURFACE_JP[surface] ?? surface;
  return `洋芝(${names})${s}${distance}m`;
}

// カードに並べる着順の最大数。実測（2026-08-16 新潟6R）で 7 走・9 走の馬が出ると
// 狭幅カラムで折り返し、カード高さが揃わず横並び比較がしづらくなる。走数そのものは常に出すので
// 「何走したか」は失われず、省略されたぶんは書評パネルが全件持つ。
export const CARD_MAX_POSITIONS = 5;

// 条件別実績の 1 行要約（例 `2走 3,3着`）。0 走は `該当なし` を返す（空文字にしない）。
// 着順は新しい順に最大 `CARD_MAX_POSITIONS` 件で、省略があれば末尾に `…` を付ける。
export function conditionRecordSummary(runs: readonly ConditionRun[]): string {
  if (runs.length === 0) return NO_RECORD;
  const shown = runs.slice(0, CARD_MAX_POSITIONS);
  const positions = shown.map((r) => r.finishing_position).join(",");
  const ellipsis = runs.length > shown.length ? "…" : "";
  return `${runs.length}走 ${positions}着${ellipsis}`;
}

// 洋芝グループ行を出すか。**完全一致より件数が増えているときだけ**出す
// （同数なら同じ集合を 2 回書くだけで情報が増えない）。
export function showsGroupRecord(note: HandicapNote): boolean {
  return note.group_runs.length > note.course_runs.length;
}

// 「純モデル vs 市場」の差[pt]。市場 implied 未取得（単勝未発売）は `null`。
// 正＝モデルが市場より高く見ている＝妙味候補。ただし近走欠損馬はモデルがベースライン近くに
// 置かれるだけの**偽の妙味**なので、この値は `no_past_runs` と必ず並べて読む。
export function modelEdgePt(
  pureWinProb: number,
  marketImplied: number | null | undefined,
): number | null {
  if (marketImplied == null) return null;
  return (pureWinProb - marketImplied) * 100;
}

// 差pt の表記（例 `+9.4pt`）。`null` は `-`。
export function edgePtLabel(pt: number | null): string {
  if (pt == null) return "-";
  return `${pt >= 0 ? "+" : ""}${pt.toFixed(1)}pt`;
}

// 休養明けの表記（例 `休養43日`）。**日数を出すだけで「久々」等の判定はしない**
// ——10ヶ月半の休養明けと 4ヶ月半の王道ローテは質が違い、その読み分けは人間がやる。
export function layoffLabel(days: number | null | undefined): string | null {
  if (days == null) return null;
  return `休養${days}日`;
}

// カードに出す「前提が壊れているサイン」(2) のバッジ群。上位人気に限って出す。
// 事実だけを並べ、go/no-go は出さない。
export function premiseBadges(
  note: HandicapNote,
  popularity: number | null | undefined,
): string[] {
  if (popularity == null || popularity > PREMISE_POPULARITY_MAX) return [];
  const out: string[] = [];
  const layoff = layoffLabel(note.layoff_days);
  if (layoff) out.push(layoff);
  if (note.distance_untried) out.push("距離初");
  if (note.surface_untried) out.push("芝ダ初");
  return out;
}

// 過去走 1 走の詳細行（例 `2026-08-02 3着 テストS`）。レース名は netkeiba 近走のみが持つ。
export function conditionRunLine(run: ConditionRun): string {
  const name = run.race_name ? ` ${run.race_name}` : "";
  return `${run.date} ${run.finishing_position}着${name}`;
}
