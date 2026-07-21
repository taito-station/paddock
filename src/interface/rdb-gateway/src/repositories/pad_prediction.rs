//! 予想（印・短評・買い目・結果）の永続化。`(date, venue, race_num)` で upsert し、
//! 馬・買い目の子行は delete→insert で冪等にする。`predict_session.rs` のトランザクション
//! 流儀に倣う。

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{
    Mark, PadPrediction, PredictionBet, PredictionHorse, PredictionResult, Surface, Venue,
};
use paddock_use_case::repository::{
    MarkStatRow, MarkStatsFilter, PredictionFilter, PredictionSearchResult, PredictionSummaryRow,
};
use sqlx::{PgPool, Row};

use crate::error::{Error, Result};
use crate::repositories::sql::escape_like;

fn date_key(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// `races` / `race_cards` を `(date, venue, race_num)` で照合し race_id を解決する。
/// 見つからなければ `None`（未確定・未取込レース）。venue は日本語名で格納されている。
async fn resolve_race_id(
    pool: &PgPool,
    date_str: &str,
    venue_jp: &str,
    race_num: u32,
) -> Result<Option<String>> {
    // 成績由来の races を優先し、無ければ出馬表由来の race_cards を引く。
    // pri で優先度を付けて並べ替えることで、両テーブルに別 race_id があっても決定的に races を採る。
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT race_id FROM (
            SELECT race_id, 0 AS pri FROM races
            WHERE date = $1 AND venue = $2 AND race_num = $3
            UNION ALL
            SELECT race_id, 1 AS pri FROM race_cards
            WHERE date = $1 AND venue = $2 AND race_num = $3
        )
        ORDER BY pri ASC
        LIMIT 1
        "#,
    )
    .bind(date_str)
    .bind(venue_jp)
    .bind(race_num as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn save_pad_prediction(
    pool: &PgPool,
    prediction: &PadPrediction,
    now: DateTime<Utc>,
) -> Result<()> {
    let date_str = date_key(prediction.date);
    let venue_jp = prediction.venue.as_jp().to_string();
    let race_id = resolve_race_id(pool, &date_str, &venue_jp, prediction.race_num).await?;
    let ts = now.to_rfc3339();

    let result = prediction.result.clone().unwrap_or_default();

    let mut tx = pool.begin().await?;

    // ヘッダを upsert。created_at は ON CONFLICT 対象外で初回値を保持する。
    sqlx::query(
        r#"
        INSERT INTO predictions
            (date, venue, race_num, race_id, title, budget, strategy_note, commentary,
             finish_1, finish_2, finish_3, recovery_rate, pnl, result_note,
             created_at, updated_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$15)
        ON CONFLICT(date, venue, race_num) DO UPDATE SET
            -- 一度解決した race_id は、後で未解決（NULL）で再取込しても巻き戻さない。
            race_id       = COALESCE(excluded.race_id, predictions.race_id),
            title         = excluded.title,
            budget        = excluded.budget,
            strategy_note = excluded.strategy_note,
            commentary    = excluded.commentary,
            finish_1      = excluded.finish_1,
            finish_2      = excluded.finish_2,
            finish_3      = excluded.finish_3,
            recovery_rate = excluded.recovery_rate,
            pnl           = excluded.pnl,
            result_note   = excluded.result_note,
            updated_at    = excluded.updated_at
        "#,
    )
    .bind(&date_str)
    .bind(&venue_jp)
    .bind(prediction.race_num as i64)
    .bind(race_id.as_deref())
    .bind(prediction.title.as_deref())
    .bind(prediction.budget.map(|b| b as i64))
    .bind(prediction.strategy_note.as_deref())
    .bind(prediction.commentary.as_deref())
    .bind(result.finish[0].map(|n| n as i64))
    .bind(result.finish[1].map(|n| n as i64))
    .bind(result.finish[2].map(|n| n as i64))
    .bind(result.recovery_rate)
    .bind(result.pnl)
    .bind(result.note.as_deref())
    .bind(&ts)
    .execute(&mut *tx)
    .await?;

    let prediction_id: i64 = sqlx::query_scalar(
        "SELECT prediction_id FROM predictions WHERE date = $1 AND venue = $2 AND race_num = $3",
    )
    .bind(&date_str)
    .bind(&venue_jp)
    .bind(prediction.race_num as i64)
    .fetch_one(&mut *tx)
    .await?;

    // 子行は入れ替え（冪等）。
    sqlx::query("DELETE FROM prediction_horses WHERE prediction_id = $1")
        .bind(prediction_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM prediction_bets WHERE prediction_id = $1")
        .bind(prediction_id)
        .execute(&mut *tx)
        .await?;

    for h in &prediction.horses {
        sqlx::query(
            r#"
            INSERT INTO prediction_horses
                (prediction_id, horse_num, horse_name, jockey, mark,
                 win_odds, popularity, win_prob, place_prob, show_prob, comment)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
            "#,
        )
        .bind(prediction_id)
        .bind(h.horse_num as i64)
        .bind(&h.horse_name)
        .bind(h.jockey.as_deref())
        .bind(h.mark.map(|m| m.as_slug()))
        .bind(h.win_odds)
        .bind(h.popularity.map(|p| p as i64))
        .bind(h.win_prob)
        .bind(h.place_prob)
        .bind(h.show_prob)
        .bind(h.comment.as_deref())
        .execute(&mut *tx)
        .await?;
    }

    for (i, b) in prediction.bets.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO prediction_bets
                (prediction_id, ordinal, bet_type, combination, amount)
            VALUES ($1,$2,$3,$4,$5)
            "#,
        )
        .bind(prediction_id)
        .bind(i as i64)
        .bind(&b.bet_type)
        .bind(&b.combination)
        .bind(b.amount as i64)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// `predictions` の 1 行。
#[derive(sqlx::FromRow)]
struct PredictionHeaderRow {
    prediction_id: i64,
    date: String,
    venue: String,
    race_num: i64,
    title: Option<String>,
    budget: Option<i64>,
    strategy_note: Option<String>,
    commentary: Option<String>,
    finish_1: Option<i64>,
    finish_2: Option<i64>,
    finish_3: Option<i64>,
    recovery_rate: Option<f64>,
    pnl: Option<i64>,
    result_note: Option<String>,
}

const HEADER_COLUMNS: &str = "prediction_id, date, venue, race_num, title, budget, \
    strategy_note, commentary, finish_1, finish_2, finish_3, recovery_rate, pnl, result_note";

/// `prediction_horses` の 1 行を `PredictionHorse` に変換する（find / list 共通）。
/// `prediction_id` 列の有無に依存しないので、単一・一括どちらの SELECT でも使える。
fn row_to_horse(row: &sqlx::postgres::PgRow) -> Result<PredictionHorse> {
    let mark: Option<String> = row.try_get("mark")?;
    Ok(PredictionHorse {
        horse_num: row.try_get::<i64, _>("horse_num")? as u32,
        horse_name: row.try_get("horse_name")?,
        jockey: row.try_get("jockey")?,
        mark: mark.and_then(|s| Mark::from_slug(&s)),
        win_odds: row.try_get("win_odds")?,
        popularity: row
            .try_get::<Option<i64>, _>("popularity")?
            .map(|p| p as u32),
        win_prob: row.try_get("win_prob")?,
        place_prob: row.try_get("place_prob")?,
        show_prob: row.try_get("show_prob")?,
        comment: row.try_get("comment")?,
    })
}

async fn load_horses(pool: &PgPool, prediction_id: i64) -> Result<Vec<PredictionHorse>> {
    let rows = sqlx::query(
        r#"
        SELECT horse_num, horse_name, jockey, mark, win_odds, popularity,
               win_prob, place_prob, show_prob, comment
        FROM prediction_horses
        WHERE prediction_id = $1
        ORDER BY horse_num ASC
        "#,
    )
    .bind(prediction_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_horse).collect()
}

async fn load_bets(pool: &PgPool, prediction_id: i64) -> Result<Vec<PredictionBet>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT bet_type, combination, amount
        FROM prediction_bets
        WHERE prediction_id = $1
        ORDER BY ordinal ASC
        "#,
    )
    .bind(prediction_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(bet_type, combination, amount)| PredictionBet {
            bet_type,
            combination,
            amount: amount as u64,
        })
        .collect())
}

/// ヘッダ行＋子行から `PadPrediction` を組み立てる（クエリは呼び出し側で実施）。
fn build_prediction(
    h: PredictionHeaderRow,
    horses: Vec<PredictionHorse>,
    bets: Vec<PredictionBet>,
) -> Result<PadPrediction> {
    let has_result = h.finish_1.is_some()
        || h.finish_2.is_some()
        || h.finish_3.is_some()
        || h.recovery_rate.is_some()
        || h.pnl.is_some()
        || h.result_note.is_some();
    let result = has_result.then(|| PredictionResult {
        finish: [
            h.finish_1.map(|n| n as u32),
            h.finish_2.map(|n| n as u32),
            h.finish_3.map(|n| n as u32),
        ],
        recovery_rate: h.recovery_rate,
        pnl: h.pnl,
        note: h.result_note,
    });

    Ok(PadPrediction {
        date: NaiveDate::parse_from_str(&h.date, "%Y-%m-%d")
            .map_err(|e| Error::Data(format!("predictions.date {:?}: {e}", h.date)))?,
        venue: Venue::try_from(h.venue.as_str())?,
        race_num: h.race_num as u32,
        title: h.title,
        budget: h.budget.map(|b| b as u64),
        strategy_note: h.strategy_note,
        commentary: h.commentary,
        horses,
        bets,
        result,
    })
}

pub async fn find_pad_prediction(
    pool: &PgPool,
    date: NaiveDate,
    venue: Venue,
    race_num: u32,
) -> Result<Option<PadPrediction>> {
    let header: Option<PredictionHeaderRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HEADER_COLUMNS} FROM predictions WHERE date = $1 AND venue = $2 AND race_num = $3"
    )))
    .bind(date_key(date))
    .bind(venue.as_jp())
    .bind(race_num as i64)
    .fetch_optional(pool)
    .await?;

    match header {
        Some(h) => {
            let horses = load_horses(pool, h.prediction_id).await?;
            let bets = load_bets(pool, h.prediction_id).await?;
            Ok(Some(build_prediction(h, horses, bets)?))
        }
        None => Ok(None),
    }
}

/// 主キー（`prediction_id`）で予想 1 件を取得する（#145・個別予想ビュー）。
pub async fn find_pad_prediction_by_id(
    pool: &PgPool,
    prediction_id: i64,
) -> Result<Option<PadPrediction>> {
    let header: Option<PredictionHeaderRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HEADER_COLUMNS} FROM predictions WHERE prediction_id = $1"
    )))
    .bind(prediction_id)
    .fetch_optional(pool)
    .await?;

    match header {
        Some(h) => {
            let horses = load_horses(pool, h.prediction_id).await?;
            let bets = load_bets(pool, h.prediction_id).await?;
            Ok(Some(build_prediction(h, horses, bets)?))
        }
        None => Ok(None),
    }
}

/// 動的 WHERE のバインド値。静的な句フラグメントのみ `format!` に埋め、値はこの型で `.bind()` する
/// （SQL インジェクション防止, #145）。
#[derive(Clone)]
enum Bind {
    Text(String),
    Int(i64),
}

/// `sql`（静的フラグメントのみで組んだ文字列）に `binds` を順に `$1..` でバインドした `QueryAs` を返す。
fn bind_all<'q, O>(
    sql: String,
    binds: &'q [Bind],
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>
where
    O: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
{
    let mut q = sqlx::query_as::<_, O>(sqlx::AssertSqlSafe(sql));
    for b in binds {
        q = match b {
            Bind::Text(s) => q.bind(s.as_str()),
            Bind::Int(i) => q.bind(*i),
        };
    }
    q
}

/// 検索フィルタから WHERE 句（`WHERE ...` を含む。条件なしなら空文字）とバインド列を組み立てる。
/// `predictions` を `p`、`races` を `r`（呼び出し側で `LEFT JOIN`）とする。
fn build_search_where(filter: &PredictionFilter) -> (String, Vec<Bind>) {
    let mut conds: Vec<String> = Vec::new();
    let mut binds: Vec<Bind> = Vec::new();
    let mut n: u32 = 0;

    if let Some(d) = filter.date_from {
        n += 1;
        conds.push(format!("p.date >= ${n}"));
        binds.push(Bind::Text(date_key(d)));
    }
    if let Some(d) = filter.date_to {
        n += 1;
        conds.push(format!("p.date <= ${n}"));
        binds.push(Bind::Text(date_key(d)));
    }
    if let Some(v) = filter.venue {
        n += 1;
        conds.push(format!("p.venue = ${n}"));
        binds.push(Bind::Text(v.as_jp().to_string()));
    }
    // 距離・芝ダは `races` 結合列の述語。`races` は常時 LEFT JOIN だが、これらの述語を置くと
    // 未照合（race_id NULL → r.* が NULL）の予想は条件不成立で脱落する＝距離/芝ダ指定時のみ
    // 実質 INNER 相当になる（venue/date のみのフィルタでは未照合予想も残る）。
    if let Some(dmin) = filter.distance_min {
        n += 1;
        conds.push(format!("r.distance >= ${n}"));
        binds.push(Bind::Int(dmin as i64));
    }
    if let Some(dmax) = filter.distance_max {
        n += 1;
        conds.push(format!("r.distance <= ${n}"));
        binds.push(Bind::Int(dmax as i64));
    }
    if let Some(s) = filter.surface {
        n += 1;
        conds.push(format!("r.surface = ${n}"));
        binds.push(Bind::Text(s.as_str().to_string()));
    }
    // 馬名 × 印。両方指定時は「同一馬が両条件を満たす」（単一 EXISTS 内で AND）。
    match (filter.horse_name.as_deref(), filter.mark) {
        (Some(name), Some(mark)) => {
            n += 1;
            let name_ph = n;
            n += 1;
            let mark_ph = n;
            conds.push(format!(
                "EXISTS (SELECT 1 FROM prediction_horses h WHERE h.prediction_id = p.prediction_id \
                 AND h.horse_name LIKE '%' || ${name_ph} || '%' ESCAPE '\\' AND h.mark = ${mark_ph})"
            ));
            binds.push(Bind::Text(escape_like(name)));
            binds.push(Bind::Text(mark.as_slug().to_string()));
        }
        (Some(name), None) => {
            n += 1;
            conds.push(format!(
                "EXISTS (SELECT 1 FROM prediction_horses h WHERE h.prediction_id = p.prediction_id \
                 AND h.horse_name LIKE '%' || ${n} || '%' ESCAPE '\\')"
            ));
            binds.push(Bind::Text(escape_like(name)));
        }
        (None, Some(mark)) => {
            n += 1;
            conds.push(format!(
                "EXISTS (SELECT 1 FROM prediction_horses h WHERE h.prediction_id = p.prediction_id \
                 AND h.mark = ${n})"
            ));
            binds.push(Bind::Text(mark.as_slug().to_string()));
        }
        (None, None) => {}
    }
    // 的中フィルタ（値は無いので静的フラグメントのみ）。`summary_from_row` の hit 算出と
    // 同じ集合になるよう、不的中は払戻 <= 0（回収率は 0 以上だが負値が混じっても表示と一致させる）。
    match filter.hit {
        Some(true) => conds.push("p.recovery_rate > 0".to_string()),
        Some(false) => {
            conds.push("p.finish_1 IS NOT NULL AND COALESCE(p.recovery_rate, 0) <= 0".to_string())
        }
        None => {}
    }

    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    (where_sql, binds)
}

/// 検索一覧の 1 行（サマリ）。`distance` / `surface` は `races` 結合で得る（未照合なら NULL）。
#[derive(sqlx::FromRow)]
struct SummaryRow {
    prediction_id: i64,
    date: String,
    venue: String,
    race_num: i64,
    race_id: Option<String>,
    title: Option<String>,
    distance: Option<i64>,
    surface: Option<String>,
    honmei_horse: Option<String>,
    finish_1: Option<i64>,
    finish_2: Option<i64>,
    finish_3: Option<i64>,
    recovery_rate: Option<f64>,
    pnl: Option<i64>,
}

fn summary_from_row(r: SummaryRow) -> Result<PredictionSummaryRow> {
    let has_finish = r.finish_1.is_some() || r.finish_2.is_some() || r.finish_3.is_some();
    let finish = has_finish.then(|| {
        [
            r.finish_1.map(|n| n as u32),
            r.finish_2.map(|n| n as u32),
            r.finish_3.map(|n| n as u32),
        ]
    });
    // 的中。`build_search_where` の hit フィルタと同じ集合になるよう判定を完全に揃える。
    // 結果記録済みの正準シグナルは finish_1（母集団 `prediction_mark_stats` も finish_1 基準）:
    //   recovery_rate > 0           → 的中 (true)
    //   finish_1 あり且つ払戻 0 以下 → 不的中 (false)
    //   それ以外（finish_1 なし）    → 結果未記録 (None)
    let hit = if r.recovery_rate.unwrap_or(0.0) > 0.0 {
        Some(true)
    } else if r.finish_1.is_some() {
        Some(false)
    } else {
        None
    };

    Ok(PredictionSummaryRow {
        prediction_id: r.prediction_id,
        date: NaiveDate::parse_from_str(&r.date, "%Y-%m-%d")
            .map_err(|e| Error::Data(format!("predictions.date {:?}: {e}", r.date)))?,
        venue: Venue::try_from(r.venue.as_str())?,
        race_num: r.race_num as u32,
        race_id: r.race_id,
        title: r.title,
        distance: r.distance.map(|d| d as u32),
        surface: r.surface.as_deref().map(Surface::try_from).transpose()?,
        honmei_horse: r.honmei_horse,
        finish,
        recovery_rate: r.recovery_rate,
        pnl: r.pnl,
        hit,
    })
}

/// 予想を横断検索する（#145）。`races` は表示用に常時 LEFT JOIN し、距離・芝ダのフィルタ指定時は
/// WHERE で絞る（未照合 race_id の予想は NULL 述語で脱落＝実質 INNER）。
pub async fn search_predictions(
    pool: &PgPool,
    filter: &PredictionFilter,
) -> Result<PredictionSearchResult> {
    let (where_sql, where_binds) = build_search_where(filter);

    // COUNT と SELECT で同一の FROM/JOIN を使う（JOIN 条件の変更で片方だけ直す事故を防ぐ）。
    const FROM_JOIN: &str = "FROM predictions p LEFT JOIN races r ON r.race_id = p.race_id";

    let count_sql = format!("SELECT COUNT(*) {FROM_JOIN} {where_sql}");
    let (total,): (i64,) = bind_all::<(i64,)>(count_sql, &where_binds)
        .fetch_one(pool)
        .await?;

    let limit_ph = where_binds.len() + 1;
    let offset_ph = where_binds.len() + 2;
    let select_sql = format!(
        "SELECT p.prediction_id, p.date, p.venue, p.race_num, p.race_id, p.title, \
         r.distance, r.surface, \
         (SELECT h.horse_name FROM prediction_horses h \
          WHERE h.prediction_id = p.prediction_id AND h.mark = 'honmei' \
          ORDER BY h.horse_num ASC LIMIT 1) AS honmei_horse, \
         p.finish_1, p.finish_2, p.finish_3, p.recovery_rate, p.pnl \
         {FROM_JOIN} \
         {where_sql} \
         ORDER BY p.date DESC, p.venue ASC, p.race_num ASC \
         LIMIT ${limit_ph} OFFSET ${offset_ph}"
    );
    let mut select_binds = where_binds.clone();
    select_binds.push(Bind::Int(filter.limit as i64));
    select_binds.push(Bind::Int(filter.offset as i64));

    let rows: Vec<SummaryRow> = bind_all::<SummaryRow>(select_sql, &select_binds)
        .fetch_all(pool)
        .await?;
    let summaries = rows
        .into_iter()
        .map(summary_from_row)
        .collect::<Result<Vec<_>>>()?;

    Ok(PredictionSearchResult {
        total_count: total as u64,
        summaries,
    })
}

/// 印別集計の 1 行（slug + 延べ数 + 1 着 / 複勝圏数）。
#[derive(sqlx::FromRow)]
struct MarkStatSqlRow {
    mark: String,
    count: i64,
    win: i64,
    show: i64,
}

/// 印の正準順（◎○▲△☆注）。レスポンスの並びを安定させる。
fn mark_order(m: Mark) -> u8 {
    match m {
        Mark::Honmei => 0,
        Mark::Taikou => 1,
        Mark::Tanana => 2,
        Mark::Renge => 3,
        Mark::Hoshi => 4,
        Mark::Chui => 5,
    }
}

/// 印別の的中率集計（#145）。母集団は結果記録済み（`finish_1 IS NOT NULL`）かつ `mark IS NOT NULL`
/// の `prediction_horses` 延べ数。`win`=1 着、`show`=複勝圏（finish_1/2/3、NULL は不一致扱い）。
pub async fn prediction_mark_stats(
    pool: &PgPool,
    filter: &MarkStatsFilter,
) -> Result<Vec<MarkStatRow>> {
    let mut conds: Vec<String> = vec![
        "p.finish_1 IS NOT NULL".to_string(),
        "h.mark IS NOT NULL".to_string(),
    ];
    let mut binds: Vec<Bind> = Vec::new();
    let mut n: u32 = 0;

    if let Some(d) = filter.date_from {
        n += 1;
        conds.push(format!("p.date >= ${n}"));
        binds.push(Bind::Text(date_key(d)));
    }
    if let Some(d) = filter.date_to {
        n += 1;
        conds.push(format!("p.date <= ${n}"));
        binds.push(Bind::Text(date_key(d)));
    }
    if let Some(v) = filter.venue {
        n += 1;
        conds.push(format!("p.venue = ${n}"));
        binds.push(Bind::Text(v.as_jp().to_string()));
    }

    let sql = format!(
        "SELECT h.mark AS mark, COUNT(*) AS count, \
         COALESCE(SUM(CASE WHEN h.horse_num = p.finish_1 THEN 1 ELSE 0 END), 0) AS win, \
         COALESCE(SUM(CASE WHEN h.horse_num IN (p.finish_1, p.finish_2, p.finish_3) \
                          THEN 1 ELSE 0 END), 0) AS show \
         FROM prediction_horses h \
         INNER JOIN predictions p ON p.prediction_id = h.prediction_id \
         WHERE {} \
         GROUP BY h.mark",
        conds.join(" AND ")
    );

    let rows: Vec<MarkStatSqlRow> = bind_all::<MarkStatSqlRow>(sql, &binds)
        .fetch_all(pool)
        .await?;

    let mut out: Vec<MarkStatRow> = rows
        .iter()
        .filter_map(|r| {
            Mark::from_slug(&r.mark).map(|m| MarkStatRow {
                mark: m,
                count: r.count as u32,
                win: r.win as u32,
                show: r.show as u32,
            })
        })
        .collect();
    out.sort_by_key(|s| mark_order(s.mark));
    Ok(out)
}
