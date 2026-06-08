//! stats クエリ共通の SQL 片ヘルパ。

/// `as_of` カットオフ用の `AND races.date < <placeholder>` 述語片を返す。
///
/// `cutoff` が `None`（全期間集計）なら空文字。日付値は呼び出し側がプレースホルダで
/// バインドするため、ここでは文字列連結に値を含めない（SQL インジェクション安全）。
pub(crate) fn date_lt_pred(cutoff: Option<&str>, placeholder: &str) -> String {
    if cutoff.is_some() {
        format!("AND races.date < {placeholder}")
    } else {
        String::new()
    }
}

/// 再取り込み時に、今回の出走集合 `horse_nums` に含まれない馬番の行だけを `table` から削除する。
///
/// 破壊的な全消し（`DELETE ... WHERE race_id = ?`）の代替。出走中の馬は `ON CONFLICT` 更新で
/// 残しつつ、取消・除外で今回いなくなった馬番の残骸だけを掃除する。`horse_nums` が空のときは
/// 何もしない（劣化パースで全行を消さないための防御）。
///
/// `table` はリテラル（`"results"` / `"horse_entries"`）のみを渡す前提で `format!` に埋め込む。
/// race_id・馬番は必ずプレースホルダでバインドする（SQL インジェクション安全）。
pub(crate) async fn delete_absent_horse_nums(
    conn: &mut sqlx::SqliteConnection,
    table: &str,
    race_id: &str,
    horse_nums: &[i64],
) -> Result<(), sqlx::Error> {
    // `table` は SQL に直接埋め込むため、既知のリテラルのみ許す（呼び出し側契約の二重防御）。
    debug_assert!(
        matches!(table, "results" | "horse_entries"),
        "table must be a known literal, got {table:?}"
    );
    if horse_nums.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", horse_nums.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql =
        format!("DELETE FROM {table} WHERE race_id = ? AND horse_num NOT IN ({placeholders})");
    let mut q = sqlx::query(&sql).bind(race_id);
    for n in horse_nums {
        q = q.bind(n);
    }
    q.execute(conn).await?;
    Ok(())
}
