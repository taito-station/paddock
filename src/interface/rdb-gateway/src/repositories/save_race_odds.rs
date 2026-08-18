use paddock_domain::{BetType, OddsValue};
use paddock_use_case::RaceOddsRecord;
use sqlx::PgPool;

use crate::error::Result;

/// 1 行のオッズ値がなぜ弾かれたか（#621）。値域条件を手書きで複製せず `OddsValue` の不変条件
/// （finite・>= 1.0・その券種の番兵でない）に委譲するので、読み取り側 `find_race_odds` の skip 判定と
/// 境界が必ず一致する。`odds` と `odds_high` を **1 回ずつだけ**評価して
/// 分類に使う（判定のたびに `try_from` を呼び直すと 1 行で最大 4 回・毎回 `format!` が走る）。
#[derive(Debug, PartialEq)]
enum RowVerdict {
    /// 全成分が有効。
    Ok,
    /// 弾くが**異常ではない**——未発売の番兵だけが理由。1 レースに数百件出るのでログは debug。
    UnpricedOnly,
    /// 値域違反を含む。**番兵と混在していても warn**——本来見るべき残骸を埋もれさせない。
    ///
    /// ある値が番兵か否かは**券種別**（#630。ワイドの `9999.9` は番兵・三連複の `9999.9` は正当）
    /// だが、Invalid / UnpricedOnly の分岐そのものは**成分の内訳**で決まる。
    /// **実地のワイド未発売行 `(9999.9, Some(0.0))` がここに落ちる**のは、相方 `0.0` が値域違反
    /// だから。弾かれた成分が全部番兵なら band でも [`RowVerdict::UnpricedOnly`]（debug）になる。
    Invalid,
}

fn classify_row(bet_type: BetType, odds: f64, odds_high: Option<f64>) -> RowVerdict {
    let mut sentinel_seen = false;
    for v in std::iter::once(odds).chain(odds_high) {
        match OddsValue::try_from((bet_type, v)) {
            Ok(_) => {}
            Err(paddock_domain::Error::UnpricedSentinel(_)) => sentinel_seen = true,
            Err(_) => return RowVerdict::Invalid,
        }
    }
    if sentinel_seen {
        RowVerdict::UnpricedOnly
    } else {
        RowVerdict::Ok
    }
}

/// 1 レース分のオッズを 1 トランザクションで upsert する。
/// 主キー `(race_id, bet_type, combination_key)` で衝突した行は最新値で上書きする。
///
/// 併せて `race_odds_snapshots`（append-only 履歴, #232）にも同じ行を追記する。`race_odds` は
/// 最新値の単一行キャッシュなので締切前 live を取っても後続/事後フェッチ（確定オッズ）で上書きされ
/// 消えるが、snapshots は `fetched_at` を PK に含めるため別時刻の取得を別行として積み、live が
/// 消えない。#218（live オッズで α 再校正）の入力を蓄積する。両 INSERT は同一 tx で原子的に行う。
///
/// 値域違反行（odds < 1.0・非有限。netkeiba の未公開組合せ 0 埋めなど）は warn を残して INSERT
/// しない。`race_odds` に無効値を入れない DB 境界のガードで、読み取り側(find_race_odds)の skip と
/// 二重で predict セッションの全停止を防ぐ(#114)。netkeiba 経路は生 f64 を渡すためここで一元的に弾く。
/// ガードの内側で両テーブルへ書くため、無効行は snapshots にも入らない。
///
/// **未発売の番兵（#621）も同じく INSERT しないが、ログは debug**。「まだ売れていない」という
/// 正常な状態で 1 レースに数百件出るため、warn だと本来の値域違反が埋もれる。値域違反と番兵が
/// 1 行に混在した場合は warn 側を優先する（[`classify_row`]）。
///
/// **ワイドの未発売行は debug にならない**。netkeiba は `["9999.9", "0.0", "--"]` の形で返すため
/// （`docs/qa/QA-odds-sentinel-621.md` Q2）相方 `0.0` が値域違反になり、混在扱いで warn 側へ落ちる
/// （[`RowVerdict::Invalid`]）。**券種で分かれるのではなく成分の内訳で分かれる**ので、弾かれた成分が
/// 全部番兵なら band でも debug。読み出し経路は成分ごとに判定するので番兵側は debug になる。
///
/// ここで弾くのは値域違反と番兵のみ。band（複勝・ワイド）の構造的不整合（odds_high NULL・low>high）は
/// 保存側バグの早期検知のため意図的にガードせず、読み取り側で `Error` として顕在化させる
/// （find_race_odds::parse_band 参照。「保存できるが読めない」のは検知すべき不正状態のため許容）。
pub async fn save_race_odds(pool: &PgPool, record: &RaceOddsRecord) -> Result<()> {
    let mut tx = pool.begin().await?;

    let fetched_at = record.fetched_at.to_rfc3339();
    for row in &record.rows {
        // 番兵判定は券種別（#630）なので、分類の前に bet_type ラベルをドメイン型へ解決する。
        // 行は ingest が `BetType`(Display=snake_case) から生成するため未知ラベルは書き手のバグ
        // ——番兵ガードを通せない行を黙って保存せず、warn を残して skip する（読み出し側の
        // 「未知は読み飛ばす」と対で、保存側は「未知は書かない」）。
        let Ok(bet_type) = BetType::try_from(row.bet_type.as_str()) else {
            tracing::warn!(
                race_id = record.race_id.value(),
                bet_type = row.bet_type,
                key = row.combination_key,
                "race_odds の未知 bet_type 行を保存せずスキップした"
            );
            continue;
        };
        match classify_row(bet_type, row.odds, row.odds_high) {
            RowVerdict::Ok => {}
            RowVerdict::UnpricedOnly => {
                tracing::debug!(
                    race_id = record.race_id.value(),
                    bet_type = row.bet_type,
                    key = row.combination_key,
                    odds = row.odds,
                    odds_high = row.odds_high,
                    "未発売の組み合わせを保存せずスキップした"
                );
                continue;
            }
            RowVerdict::Invalid => {
                tracing::warn!(
                    race_id = record.race_id.value(),
                    bet_type = row.bet_type,
                    key = row.combination_key,
                    odds = row.odds,
                    odds_high = row.odds_high,
                    "race_odds の不正オッズ行を保存せずスキップした"
                );
                continue;
            }
        }
        sqlx::query(
            r#"
            INSERT INTO race_odds
                (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(race_id, bet_type, combination_key) DO UPDATE SET
                odds       = excluded.odds,
                odds_high  = excluded.odds_high,
                -- 人気はスクレイプ経路(predict)では取れず NULL になる。既存の人気付き値を
                -- NULL で潰さないよう、新値が NULL のときは既存値を残す（odds は常に最新で上書き）。
                popularity = COALESCE(excluded.popularity, race_odds.popularity),
                fetched_at = excluded.fetched_at
            "#,
        )
        .bind(record.race_id.value())
        .bind(&row.bet_type)
        .bind(&row.combination_key)
        .bind(row.odds)
        .bind(row.odds_high)
        .bind(row.popularity.map(|p| p as i64))
        .bind(&fetched_at)
        .execute(&mut *tx)
        .await?;

        // append-only 履歴(#232)。fetched_at が PK に入るため別時刻の取得は別行として残り、
        // 後続/事後フェッチで race_odds が上書きされても live スナップショットは消えない。
        // 同一 fetched_at の再保存は冪等にしたいので衝突時は何もしない（point-in-time をそのまま記録）。
        // 上の race_odds INSERT と列構成（odds/odds_high/popularity/fetched_at）を共有するため、
        // 列を増減するときは両 INSERT を同時に更新すること（片方だけの更新は履歴欠落を生む）。
        sqlx::query(
            r#"
            INSERT INTO race_odds_snapshots
                (race_id, bet_type, combination_key, odds, odds_high, popularity, fetched_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(race_id, bet_type, combination_key, fetched_at) DO NOTHING
            "#,
        )
        .bind(record.race_id.value())
        .bind(&row.bet_type)
        .bind(&row.combination_key)
        .bind(row.odds)
        .bind(row.odds_high)
        .bind(row.popularity.map(|p| p as i64))
        .bind(&fetched_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BetType, RowVerdict, classify_row};

    /// warn / debug の出し分けは `classify_row` の戻り値がすべてなので、ここで固定する。
    /// 統合テスト（`tests/test_race_odds_persistence.rs`）は「保存されないこと」しか見ておらず、
    /// 分類を取り違えても緑のままだった（#621 3 巡目）。DB 不要の純関数なのでここが正しい置き場。
    #[test]
    fn classify_row_separates_unpriced_sentinel_from_out_of_range() {
        assert_eq!(classify_row(BetType::Win, 3.5, None), RowVerdict::Ok);
        assert_eq!(classify_row(BetType::Place, 1.5, Some(2.0)), RowVerdict::Ok);
        // 番兵単独（馬連・三連複など band でない券種）は debug 側。
        assert_eq!(
            classify_row(BetType::Quinella, 99_999.9, None),
            RowVerdict::UnpricedOnly
        );
        assert_eq!(
            classify_row(BetType::Trifecta, 999_999.9, None),
            RowVerdict::UnpricedOnly
        );
        // 値域違反が 1 成分でもあれば warn 側。**評価順に依らない**ことを両向きで固定する。
        assert_eq!(
            classify_row(BetType::Trio, 0.0, Some(99_999.9)),
            RowVerdict::Invalid
        );
        assert_eq!(
            classify_row(BetType::Trio, 99_999.9, Some(0.0)),
            RowVerdict::Invalid
        );
        // 実地のワイド未発売行はこの形（QA-odds-sentinel-621.md Q2）。番兵だが相方 0.0 で warn 側。
        assert_eq!(
            classify_row(BetType::Wide, 9_999.9, Some(0.0)),
            RowVerdict::Invalid
        );
        // 分岐は券種でなく成分の内訳。番兵だけが 2 成分に揃えば band でも debug 側
        // （現行 netkeiba では出ないが契約として固定）。
        assert_eq!(
            classify_row(BetType::Wide, 9_999.9, Some(9_999.9)),
            RowVerdict::UnpricedOnly
        );
        // 非有限は #114 の枝。番兵と取り違えず warn 側へ（ここを張らないと skip 変異が生き残る）。
        assert_eq!(
            classify_row(BetType::Win, f64::NAN, None),
            RowVerdict::Invalid
        );
        assert_eq!(
            classify_row(BetType::Place, 3.5, Some(f64::INFINITY)),
            RowVerdict::Invalid
        );
    }

    /// 番兵は券種スコープ（#630/#634）。同じ値でも券種が違えば正当なオッズとして保存される。
    /// 核は「三連複の `9999.9` は Ok」——フラット判定に退行すると正当な高配当が黙って消える。
    #[test]
    fn classify_row_scopes_sentinels_to_the_bet_type() {
        // ワイドの 9999.9 は番兵、三連複の 9999.9 は正当（9000〜11000 帯に実在する配当）。
        assert_eq!(
            classify_row(BetType::Wide, 9_999.9, Some(9_999.9)),
            RowVerdict::UnpricedOnly
        );
        assert_eq!(classify_row(BetType::Trio, 9_999.9, None), RowVerdict::Ok);
        assert_eq!(
            classify_row(BetType::Trio, 99_999.9, None),
            RowVerdict::UnpricedOnly
        );
        // 三連単は 99999.9 が正当（実在の高配当帯）で 999999.9 だけが番兵。
        assert_eq!(
            classify_row(BetType::Trifecta, 99_999.9, None),
            RowVerdict::Ok
        );
        // 単勝・複勝に番兵は無い（#634 実測 0 行）。他券種の番兵値でも保存される。
        assert_eq!(classify_row(BetType::Win, 99_999.9, None), RowVerdict::Ok);
        assert_eq!(
            classify_row(BetType::Place, 9_999.9, Some(9_999.9)),
            RowVerdict::Ok
        );
    }
}
