import { pct } from "../../lib/format";
import {
  type BoardHorse,
  heatColor,
  markSymbol,
  placeOddsLabel,
} from "../../lib/board";

// 全頭横並び盤の 1 馬カラム（#411 で RaceBoard から抽出）。数値密度を保ちつつ、書評のある馬は
// クリック / Enter / Space で詳細パネルを開閉できる。開閉状態（selectedHorse）と trigger 要素の
// フォーカス管理は親（RaceBoard）が持ち、カードは onSelect で馬番と trigger 要素だけ通知する。
export function HorseCard({
  horse: h,
  maxWin,
  showModel,
  isSelected,
  onSelect,
}: {
  horse: BoardHorse;
  maxWin: number;
  showModel: boolean;
  isSelected: boolean;
  onSelect: (horseNum: number, trigger: HTMLElement) => void;
}) {
  // detail_lines はスキーマ上必須（string[]）。comment または根拠行があれば展開可。
  const hasDetail = !!h.comment || h.detail_lines.length > 0;
  return (
    <div
      className={
        "horse-col" +
        (h.is_overlay ? " is-overlay" : "") +
        (h.is_value ? " is-value" : "") +
        (hasDetail ? " has-detail" : "") +
        (isSelected ? " is-selected" : "")
      }
      role={hasDetail ? "button" : undefined}
      tabIndex={hasDetail ? 0 : undefined}
      aria-label={
        hasDetail ? `${h.horse_num} ${h.horse_name} の書評を開く` : undefined
      }
      aria-expanded={hasDetail ? isSelected : undefined}
      aria-controls={
        hasDetail && isSelected ? "horse-detail-panel" : undefined
      }
      title={hasDetail ? "クリック / Enter / Space で書評を表示" : undefined}
      onClick={
        hasDetail ? (e) => onSelect(h.horse_num, e.currentTarget) : undefined
      }
      onKeyDown={
        hasDetail
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onSelect(h.horse_num, e.currentTarget);
              }
            }
          : undefined
      }
    >
      <div
        className="heat"
        style={{ background: heatColor(h.win_prob, maxWin) }}
        title={`ブレンド勝率 ${pct(h.win_prob)}`}
      >
        <span className="rank">{h.model_rank}</span>
      </div>
      <div className="num-mark">
        <span className="num">{h.horse_num}</span>
        <span className="mark">{markSymbol(h.mark)}</span>
        {/* 確定着順（#381。results 由来。除外/中止・未確定は null で非表示）。 */}
        {h.finishing_position != null && (
          <span className="finish-pos" title={`確定 ${h.finishing_position} 着`}>
            {h.finishing_position}着
          </span>
        )}
      </div>
      <div className="hname" title={h.horse_name}>
        {h.horse_name}
      </div>
      <div className="jockey">{h.jockey ?? "-"}</div>
      {/* 確率は出所ごとに 2 文字ラベルで明示（#373）: ブ=ブレンド(本番α=0.2)・
          モ=モデル(純α=1.0)・市=市場implied。狭幅カラムに合わせ full 名は title に退避。
          市場は単勝オッズ由来のため勝率のみ（連対/複勝の市場 implied は出さない）。 */}
      <dl className="stats">
        <div title="ブレンド勝率＝本番 α=0.2（市場ブレンド）で 1 着になる確率">
          <dt>ブ勝</dt>
          <dd>{pct(h.win_prob)}</dd>
        </div>
        <div title="ブレンド連対率＝本番 α=0.2 で 2 着以内に入る確率">
          <dt>ブ連</dt>
          <dd>{pct(h.place_prob)}</dd>
        </div>
        <div title="ブレンド複勝率＝本番 α=0.2 で 3 着以内に入る確率">
          <dt>ブ複</dt>
          <dd>{pct(h.show_prob)}</dd>
        </div>
        {showModel && (
          <>
            <div title="モデル勝率＝純モデル α=1.0（市場非依存）で 1 着になる確率">
              <dt>モ勝</dt>
              <dd>{pct(h.pure_win_prob)}</dd>
            </div>
            <div title="モデル連対率＝純モデル α=1.0 で 2 着以内に入る確率">
              <dt>モ連</dt>
              <dd>{pct(h.pure_place_prob)}</dd>
            </div>
            <div title="モデル複勝率＝純モデル α=1.0 で 3 着以内に入る確率">
              <dt>モ複</dt>
              <dd>{pct(h.pure_show_prob)}</dd>
            </div>
          </>
        )}
        <div title="市場勝率＝単勝オッズから逆算した市場推定の勝率（胴元の控除を抜いた実力評価）。モデル/ブレンド勝率と比べて乖離＝妙味">
          <dt>市勝</dt>
          <dd>{h.market_implied == null ? "-" : pct(h.market_implied)}</dd>
        </div>
        <div>
          <dt>単勝</dt>
          <dd>{h.win_odds == null ? "-" : h.win_odds.toFixed(1)}</dd>
        </div>
        <div>
          <dt>複勝</dt>
          <dd>{placeOddsLabel(h.place_odds_low, h.place_odds_high)}</dd>
        </div>
        <div>
          <dt>人気</dt>
          <dd>{h.popularity ?? "-"}</dd>
        </div>
      </dl>
      <div className="flags">
        {h.is_overlay && (
          <span
            className="chip chip-overlay"
            title="ブレンド勝率1位×人気1位＝ほぼ複勝圏"
          >
            複勝圏
          </span>
        )}
        {h.is_value && (
          <span
            className="chip chip-value"
            title="ブレンド上位×市場人気低＝妙味・ワイドボックス候補"
          >
            妙味
          </span>
        )}
        {hasDetail && <span className="chip chip-note">書評</span>}
      </div>
    </div>
  );
}
