import { type RefObject } from "react";
import { type BoardHorse, markSymbol } from "../../lib/board";
import {
  NO_RECORD,
  conditionRunLine,
  layoffLabel,
  showsGroupRecord,
} from "../../lib/handicap";

// 馬書評（クリックで展開する詳細パネル。#411 で RaceBoard から抽出）。
// horse は親が selectedHorse から horses.find で解決した値（見つからなければ undefined）。
// undefined のときは何も描画しない（元の IIFE の早期 return 相当を props 側で表現）。
//
// #628 で手動ハンデ精査の材料を追加した。カードが出すのは要点（N走 着順列・上位人気のバッジ）で、
// ここは内訳（どのレースか・洋芝グループ・全頭ぶんの休養日数）を持つ。
export function HorseDetailPanel({
  horse: h,
  conditionLabel,
  groupConditionLabel,
  onClose,
  closeBtnRef,
}: {
  horse: BoardHorse | undefined;
  /** 今回条件のラベル（例 `新潟芝1000m`）。 */
  conditionLabel: string;
  /** 場グループのラベル（例 `洋芝(札幌/函館)芝2000m`）。洋芝場でのみ非 null。 */
  groupConditionLabel: string | null;
  onClose: () => void;
  closeBtnRef: RefObject<HTMLButtonElement | null>;
}) {
  if (!h) return null;
  const hc = h.handicap;
  const layoff = layoffLabel(hc.layoff_days);
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
      {/* パネルは hasDetail(=comment もしくは detail_lines あり)でのみ開くため、
          detail_lines 空のとき comment は必ず存在する（lead 表示済み・追加表示は不要）。 */}
      {h.detail_lines.length > 0 && (
        <ul className="horse-detail-lines">
          {h.detail_lines.map((line, i) => (
            <li key={`${i}-${line}`}>{line}</li>
          ))}
        </ul>
      )}

      {/* 手動ハンデ精査の材料（#628）。事実だけを並べ、良し悪しの判定は書かない。 */}
      <dl className="handicap-detail">
        <div>
          <dt>{conditionLabel}</dt>
          <dd>
            {hc.course_runs.length === 0 ? (
              <span className="muted">{NO_RECORD}</span>
            ) : (
              <ul>
                {hc.course_runs.map((r) => (
                  <li key={`${r.date}-${r.finishing_position}`}>
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
            <dt>{groupConditionLabel}</dt>
            <dd>
              <ul>
                {hc.group_runs.map((r) => (
                  <li key={`g-${r.date}-${r.finishing_position}`}>
                    {conditionRunLine(r)}
                  </li>
                ))}
              </ul>
            </dd>
          </div>
        )}
        <div>
          <dt>前走からの間隔</dt>
          <dd>{layoff ?? <span className="muted">近走なし</span>}</dd>
        </div>
        <div>
          <dt>今回条件の経験</dt>
          <dd>
            {hc.no_past_runs ? (
              // 近走 0 件は「未経験」ではなく「データが無い」。モデル確率がベースライン近くに
              // 置かれる＝差pt が偽の妙味として出る、という読み方を明示する。
              <span className="warn">
                近走データ 0 件（モデル確率はベースライン近く＝差ptは妙味の根拠にならない）
              </span>
            ) : (
              [
                hc.distance_untried ? "今回距離は未経験" : "今回距離の経験あり",
                hc.surface_untried ? "今回の芝ダは未経験" : "今回の芝ダの経験あり",
              ].join(" / ")
            )}
          </dd>
        </div>
      </dl>
    </div>
  );
}
