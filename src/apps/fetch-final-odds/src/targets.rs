//! 対象レース一覧の読み込み。
//!
//! 一覧は 1 行 1 つの paddock race_id（先頭行 `race_id` はヘッダとして読み飛ばす）。DB から読み取り専用の
//! SQL で作り、凍結して入力にする（`find_finished_races_between` は `source='pdf'` に限るため、
//! netkeiba 由来で取り込んだレースも含められるよう DB 接続に依存しない）。

use paddock_domain::RaceId;
use paddock_use_case::{netkeiba_race_id_from_paddock, paddock_race_id_from_netkeiba};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub race_id: String,
    pub netkeiba_id: String,
}

/// 一覧の本文を読む。空行は無視。変換できない race_id・重複はエラー（黙って飛ばさない）。
pub fn parse_targets(text: &str) -> anyhow::Result<Vec<Target>> {
    let mut out: Vec<Target> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, line) in text.lines().enumerate() {
        let id = line.trim();
        if id.is_empty() || (i == 0 && id == "race_id") {
            continue;
        }
        let race_id = RaceId::try_from(id)
            .map_err(|e| anyhow::anyhow!("{} 行目: race_id が不正です: {id}（{e}）", i + 1))?;
        let netkeiba_id = netkeiba_race_id_from_paddock(&race_id).map_err(|e| {
            anyhow::anyhow!(
                "{} 行目: netkeiba race_id に変換できません: {id}（{e}）",
                i + 1
            )
        })?;
        // 表記の揺れ（ゼロ埋めの有無など）で同じレースを二度叩かないよう、変換後の netkeiba ID で数える
        if !seen.insert(netkeiba_id.clone()) {
            anyhow::bail!(
                "{} 行目: 重複したレースです: {id}（netkeiba {netkeiba_id}）",
                i + 1
            );
        }
        // 保存ファイル名・TSV の race_id は正規表記（netkeiba ID から戻したもの）にそろえる。表記の違う一覧で
        // 再開しても取得済みを取り直さず、TSV を DB の race_id と突き合わせられるようにする
        let canonical = paddock_race_id_from_netkeiba(&netkeiba_id).map_err(|e| {
            anyhow::anyhow!("{} 行目: race_id を正規化できません: {id}（{e}）", i + 1)
        })?;
        out.push(Target {
            race_id: canonical.value().to_string(),
            netkeiba_id,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_paddock_ids_to_netkeiba_ids() {
        // 実測で確かめた組（issue #721）: 2025-06-01 東京 11R と 2025-01-05 中京 10R
        let t = parse_targets("race_id\n2025-2-tokyo-12-11R\n\n2025-1-chukyo-1-10R\n").unwrap();
        assert_eq!(
            t,
            vec![
                Target {
                    race_id: "2025-2-tokyo-12-11R".into(),
                    netkeiba_id: "202505021211".into()
                },
                Target {
                    race_id: "2025-1-chukyo-1-10R".into(),
                    netkeiba_id: "202507010110".into()
                },
            ]
        );
    }

    #[test]
    fn rejects_bad_and_duplicate_ids() {
        assert!(parse_targets("race_id\nnot-a-race\n").is_err());
        let dup = parse_targets("2025-2-tokyo-12-11R\n2025-2-tokyo-12-11R\n").unwrap_err();
        assert!(dup.to_string().contains("重複"), "{dup}");
        // 表記が違っても同じ netkeiba レースなら重複（回・日のゼロ埋め）
        let alias = parse_targets("2025-2-tokyo-12-11R\n2025-02-tokyo-12-11R\n").unwrap_err();
        assert!(alias.to_string().contains("重複"), "{alias}");
    }

    #[test]
    fn race_id_is_normalized() {
        let t = parse_targets("2025-02-tokyo-12-11R\n").unwrap();
        assert_eq!(t[0].race_id, "2025-2-tokyo-12-11R");
    }
}
