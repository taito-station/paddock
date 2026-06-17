use sqlx::PgPool;

use crate::error::Result;

/// pdf 成績行（`results.horse_id IS NULL`）に、`horses` マスタへ馬名がちょうど 1 件一致する
/// horse_id を backfill する。set-based な単一 UPDATE。埋めた行数を返す。
///
/// - `COUNT(*) = 1` 条件で**一意一致のみ**採用（同名別馬＝複数一致は NULL 据え置き）。
/// - `horse_id IS NULL` 限定なので冪等（既存値は上書きしない）。
/// - `results` は #59 以降 pdf 確定成績専用だが、その前提が崩れても netkeiba 由来行へ
///   誤って backfill しないよう `source = 'pdf'` を明示する（防御）。
pub async fn backfill_results_horse_ids(pool: &PgPool) -> Result<u64> {
    let affected = sqlx::query(
        r#"
        UPDATE results
        SET horse_id = (
            SELECT h.horse_id FROM horses h WHERE h.horse_name = results.horse_name
        )
        WHERE results.horse_id IS NULL
          AND results.source = 'pdf'
          AND (
              SELECT COUNT(*) FROM horses h WHERE h.horse_name = results.horse_name
          ) = 1
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(affected)
}
