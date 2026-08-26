import { memo } from "react";
import { pct } from "../../lib/format";
import {
  type BoardHorse,
  heatColor,
  markSymbol,
  winOddsMove,
} from "../../lib/board";
import { placeBand } from "../../lib/live";
import {
  EMPTY_HANDICAP_NOTE,
  NO_MATERIAL,
  type HandicapNote,
  conditionRecordSummary,
  edgePtLabel,
  hasHandicapMaterial,
  modelEdgePt,
  premiseBadges,
} from "../../lib/handicap";

// 全頭横並び盤の 1 馬カラム（#411 で RaceBoard から抽出）。数値密度を保ちつつ、書評のある馬は
// クリック / Enter / Space で詳細パネルを開閉できる。開閉状態（selectedHorse）と trigger 要素の
// フォーカス管理は親（RaceBoard）が持ち、カードは onSelect で馬番と trigger 要素だけ通知する。
//
// React.memo 化（#475）: オッズ自動ポーリング＋予算入力の毎キーストロークで親が再描画されるため、
// props（horse は query データ由来で参照安定・showModel/showMorning/isSelected は primitive・onSelect は
// 親で useCallback 済み）が変わらない限り再描画をスキップし、全 18 頭カードの無駄再描画を防ぐ。
function HorseCardImpl({
  horse: h,
  maxWin,
  showModel,
  showMorning,
  conditionLabel,
  isSelected,
  onSelect,
}: {
  horse: BoardHorse;
  maxWin: number;
  showModel: boolean;
  showMorning: boolean;
  /** 今回条件の短縮ラベル（例 `新潟芝1000m`）。条件別実績行の title に使う（#628）。 */
  conditionLabel: string;
  isSelected: boolean;
  onSelect: (horseNum: number, trigger: HTMLElement) => void;
}) {
  // 手動ハンデ精査の材料（#628）。いずれも事実の表示であって go/no-go 判定ではない。
  // api-server が古い成果物を配信し続ける事故（#570）ではこのフィールドが欠けうるので、
  // 盤全体を落とさないよう既定値へ縮退させる（型は必須のまま＝正常系は素通り）。
  const handicap: HandicapNote = h.handicap ?? EMPTY_HANDICAP_NOTE;
  // 材料が引けているか。**引けていないとき「該当なし」と書かない**——それは
  // 「走っていない」という断定になり、本 issue が塞ごうとしている取り違えそのもの。
  const hasMaterial = hasHandicapMaterial(h.handicap);
  // detail_lines はスキーマ上必須（string[]）。comment・根拠行・ハンデ材料のいずれかがあれば展開可。
  const hasDetail = !!h.comment || h.detail_lines.length > 0 || hasMaterial;
  // 「書評」チップは書評があることの信号として残す（hasDetail に材料を足したことで
  // ほぼ全頭が展開可になったため、チップまで全頭に出すと信号の意味が消える）。
  const hasCommentary = !!h.comment || h.detail_lines.length > 0;
  // 朝↔現の単勝変動（#448）。朝 snapshot が無い馬は null（矢印を出さない）。
  const oddsMove = winOddsMove(h.morning_win_odds, h.win_odds);
  const edgePt = modelEdgePt(h.pure_win_prob, h.market_implied);
  const badges = premiseBadges(handicap, h.popularity);
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
        hasDetail
          ? `${h.horse_num} ${h.horse_name} の詳細（条件別実績・書評）を開く`
          : undefined
      }
      aria-expanded={hasDetail ? isSelected : undefined}
      aria-controls={hasDetail && isSelected ? "horse-detail-panel" : undefined}
      title={hasDetail ? "クリック / Enter / Space で詳細を表示" : undefined}
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
          <span
            className="finish-pos"
            title={`確定 ${h.finishing_position} 着`}
          >
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
            <div
              className="group-sep"
              title="モデル勝率＝純モデル α=1.0（市場非依存）で 1 着になる確率"
            >
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
        <div
          className="group-sep"
          title="市場勝率＝単勝オッズから逆算した市場推定の勝率（胴元の控除を抜いた実力評価）。モデル/ブレンド勝率と比べて乖離＝妙味"
        >
          <dt>市勝</dt>
          <dd>{h.market_implied == null ? "-" : pct(h.market_implied)}</dd>
        </div>
        {/* 純モデル−市場の差[pt]（#628）。被減数の「モ勝」と同じ showModel トグル配下に置く
            ——モデル列を畳んだ状態で差だけ残ると、値の出所が画面から消える。
            過去走 0 件の馬はモデルがベースライン近くに置かれるだけなので、欠損フラグを
            必ず同じ行に並べる（差pt だけ見て買い材料と誤読しないため）。 */}
        {showModel && (
          <div
            className="edge-row"
            title="モデル勝率（純 α=1.0）− 市場implied勝率[pt]。過去走データが無い馬はモデルがベースライン近くに置かれるため、この差は妙味ではなく欠損の影"
          >
            <dt>差</dt>
            <dd>{edgePtLabel(edgePt)}</dd>
          </div>
        )}
        <div className="group-sep">
          <dt>単勝</dt>
          <dd>{h.win_odds == null ? "-" : h.win_odds.toFixed(1)}</dd>
        </div>
        {/* 朝時点の単勝オッズ＋朝→現の変動矢印（#448）。朝比較 ON かつ朝 snapshot がある時のみ。
            ▲＝オッズ下落＝人気化（妙味減）／△＝上昇＝過小人気化（妙味）。 */}
        {showMorning && h.morning_win_odds != null && (
          <div title="朝時点（最初にフル盤成立した snapshot）の単勝オッズと、朝→現の変動。▲人気化（妙味減）／△妙味（過小人気化）">
            <dt>朝単</dt>
            <dd>
              {h.morning_win_odds.toFixed(1)}
              {oddsMove && (
                <span
                  className={`odds-move ${oddsMove.cls}`}
                  title={oddsMove.label}
                >
                  {oddsMove.symbol}
                </span>
              )}
            </dd>
          </div>
        )}
        <div>
          <dt>複勝</dt>
          <dd>{placeBand(h.place_odds_low, h.place_odds_high)}</dd>
        </div>
        <div>
          <dt>人気</dt>
          <dd>{h.popularity ?? "-"}</dd>
        </div>
      </dl>
      {/* 条件別実績（#628・最優先）。今回と同じ 場×芝ダ×距離 の「N走 着順列」を全頭に常時出す
          ——2・3番人気が当該条件で大敗している、のような事実は横並びで比べて初めて意味を持つ。
          0 走は空欄でなく「該当なし」（空欄は「まだ引いていない」に見える）。
          材料そのものが引けていないときは「該当なし」と断定せず `—` に落とす。 */}
      <div
        className="cond-record"
        title={
          hasMaterial
            ? `${conditionLabel} での過去成績（着順は新しい順）`
            : "条件別実績を取得できていません（走っていないという意味ではありません）"
        }
      >
        {hasMaterial ? (
          conditionRecordSummary(handicap.course_runs)
        ) : (
          <span className="muted">{NO_MATERIAL}</span>
        )}
      </div>
      {/* 過去走データ 0 件の印（#628）。**モデル列のトグルと直交する事実**なので
          差pt 行（showModel 配下）ではなくここに常時出す——モデル列を畳んだだけで
          「この馬の確率は欠損由来」という警告が消えてはいけない。 */}
      {handicap.no_past_runs && (
        <div className="premise-flags">
          <span
            className="chip chip-missing"
            title="過去走データ 0 件。モデル確率はベースライン近くの推定なので、差pt（モ勝 − 市勝）は妙味の根拠にならない"
          >
            戦績なし
          </span>
        </div>
      )}
      {/* 人気馬の前提が壊れているサイン（#628）。上位人気に限って出す（全頭だとノイズ）。
          休養日数・距離初・芝ダ初の**事実だけ**で、閾値判定はしない（読み分けは人間がやる）。 */}
      {badges.length > 0 && (
        <div className="premise-flags">
          {badges.map((b) => (
            <span key={b} className="chip chip-premise">
              {b}
            </span>
          ))}
        </div>
      )}
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
        {/* 「書評」は書評（人手短評・根拠 bullet）がある馬の信号として残す。
            hasDetail は条件別実績でもほぼ全頭 true になるので、そちらを使うと信号が消える。 */}
        {hasCommentary && <span className="chip chip-note">書評</span>}
      </div>
    </div>
  );
}

// メモ化して親（RaceBoard）のポーリング/予算入力の再描画で全頭が再レンダリングされるのを防ぐ（#475）。
export const HorseCard = memo(HorseCardImpl);
