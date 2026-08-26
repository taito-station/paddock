import { type RefObject } from "react";
import { type BoardHorse, markSymbol } from "../../lib/board";
import {
  NO_RECORD,
  conditionRunLine,
  showsGroupRecord,
} from "../../lib/handicap";

// 馬書評（クリックで展開する詳細パネル。#411 で RaceBoard から抽出）。
// horse は親が selectedHorse から horses.find で解決した値（見つからなければ undefined）。
// undefined のときは何も描画しない（元の IIFE の早期 return 相当を props 側で表現）。
//
// #628 で手動ハンデ精査の材料を追加した。カードが出すのは要点（N走 着順列・上位人気のバッジ）で、
// ここは内訳（どのレースか・洋芝グループ・全頭ぶんの間隔）を持つ。
export function HorseDetailPanel({
  horse: h,
  conditionLabel,
  groupConditionLabel,
  distanceToleranceM,
  onClose,
  closeBtnRef,
}: {
  horse: BoardHorse | undefined;
  /** 今回条件のラベル（例 `新潟芝1000m`）。 */
  conditionLabel: string;
  /** 場グループのラベル（例 `洋芝(札幌/函館)芝2000m`）。洋芝場でのみ非 null。 */
  groupConditionLabel: string | null;
  /** 「今回距離を経験済み」の許容幅[m]。判定に使った値をサーバから受け取り表示するだけ
   *  （web に同値を持つとサーバだけ変えたとき画面が定義を偽る）。 */
  distanceToleranceM: number;
  onClose: () => void;
  closeBtnRef: RefObject<HTMLButtonElement | null>;
}) {
  if (!h) return null;
  // **`null` = 未取得**（サーバが明示）。材料が無いときはブロックごと出さない
  // ——「該当なし」「過去走なし」と書くと「走っていない」という断定になる。
  const hc = h.handicap ?? null;
  return (
    <div
      className="horse-detail"
      id="horse-detail-panel"
      role="region"
      aria-label={`${h.horse_num} ${h.horse_name} の書評`}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
    >
      <div className="horse-detail-head">
        <span className="mark">{markSymbol(h.mark)}</span>
        <strong>
          {h.horse_num} {h.horse_name}
        </strong>
        <span className="muted">{h.jockey ?? "-"}</span>
        <button
          ref={closeBtnRef}
          className="detail-close"
          onClick={onClose}
          aria-label="閉じる"
        >
          ×
        </button>
      </div>
      {h.comment && <p className="horse-detail-lead">{h.comment}</p>}
      {/* comment / detail_lines は無いことがある（#628 でハンデ材料だけでも開けるようにした）。
          その場合は下の handicap-detail が本体になる。 */}
      {h.detail_lines.length > 0 && (
        <ul className="horse-detail-lines">
          {h.detail_lines.map((line, i) => (
            <li key={`${i}-${line}`}>{line}</li>
          ))}
        </ul>
      )}

      {/* 手動ハンデ精査の材料（#628）。事実だけを並べ、良し悪しの判定は書かない。 */}
      {hc && (
        <dl className="handicap-detail">
          <div>
            <dt>{conditionLabel}</dt>
            <dd>
              {hc.course_runs.length === 0 ? (
                <span className="muted">{NO_RECORD}</span>
              ) : (
                <ul>
                  {/* key に index を含める。dedup キーは (馬名, 日付, 場, R) なので
                    (日付, 着順) の一意性は保証されない（同日・同着順の 2 走がありうる）。 */}
                  {hc.course_runs.map((r, i) => (
                    <li key={`c-${i}-${r.date}-${r.finishing_position}`}>
                      {conditionRunLine(r)}
                    </li>
                  ))}
                </ul>
              )}
            </dd>
          </div>
          {/* 洋芝（札幌⇄函館）は場が違ってもコース適性が通じるので、完全一致より広い集合を
            **別ラベルで**併記する（黙って混ぜない——事実が違うものは行を分ける）。
            件数が増えていないときは同じ集合の再掲になるので出さない。 */}
          {groupConditionLabel && showsGroupRecord(hc) && (
            <div>
              {/* 上位集合なので当場の走も再掲される。件数だけ見て「別に N 走ある」と
                誤読されないよう、内訳を見出しに明示する。 */}
              <dt>
                {groupConditionLabel}
                <span className="muted">
                  （{hc.group_runs.length}走・うち当場{hc.course_runs.length}
                  走）
                </span>
              </dt>
              <dd>
                <ul>
                  {hc.group_runs.map((r, i) => (
                    <li key={`g-${i}-${r.date}-${r.finishing_position}`}>
                      {conditionRunLine(r)}
                    </li>
                  ))}
                </ul>
              </dd>
            </div>
          )}
          <div>
            {/* 見出しが「前走からの間隔」なので値は日数だけ（カードの `間隔◯日` は
              見出しが無いぶんラベルを付ける）。 */}
            <dt>前走からの間隔</dt>
            <dd>
              {hc.layoff_days != null ? (
                `${hc.layoff_days}日`
              ) : hc.no_past_runs ? (
                <span className="muted">過去走なし</span>
              ) : (
                // 過去走はあるのに間隔が出せない＝前走日が未来（データ不整合）。
                // 「過去走なし」と断定せず不明と出す。
                <span className="muted">不明</span>
              )}
            </dd>
          </div>
          <div>
            {/* 条件別実績（上のブロック）は**完全一致**、この行は**±許容幅**で判定が違う。
              「新潟芝1000m → 該当なし」と「今回距離の経験あり」が並ぶと矛盾に見えるので、
              ラベルを「近い距離（±Nm）」にして定義差を文言で示す。 */}
          <dt>今回条件の経験</dt>
            <dd>
              {hc.no_past_runs ? (
                // 過去走 0 件は「未経験」ではなく「データが無い」。モデル確率がベースライン近くに
                // 置かれる＝差pt が偽の妙味として出る、という読み方を明示する。
                <span className="warn">
                  過去走データ 0
                  件（モデル確率はベースライン近く＝差ptは妙味の根拠にならない）
                </span>
              ) : (
                [
                  hc.distance_untried
                    ? `近い距離（±${distanceToleranceM}m）は未経験`
                    : `近い距離（±${distanceToleranceM}m）の経験あり`,
                  hc.surface_untried
                    ? "今回の芝ダは未経験"
                    : "今回の芝ダの経験あり",
                ].join(" / ")
              )}
            </dd>
          </div>
        </dl>
      )}
    </div>
  );
}
