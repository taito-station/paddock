use std::path::Path;

use serde::Serialize;

/// pad ツリーの 1 ノード。`path` が `Some` なら表示可能な MD ファイル、`None` ならディレクトリ。
#[derive(Serialize)]
pub struct Node {
    /// 表示名（ファイルは拡張子を除いた名前）。
    pub name: String,
    /// pad_dir からの相対パス（ファイルのみ）。`/api/doc?path=` にそのまま渡す。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub children: Vec<Node>,
}

/// pad_dir 配下を走査して `.md` ファイルのツリーを返す。
/// トップ階層は日付ディレクトリを降順（新しい開催日が上）、それ以外（`_analysis` 等）を後ろに並べる。
pub fn build_tree(pad_dir: &Path) -> Vec<Node> {
    let mut nodes = read_dir_nodes(pad_dir, pad_dir);
    nodes.sort_by(|a, b| match (is_date(&a.name), is_date(&b.name)) {
        (true, true) => b.name.cmp(&a.name),       // 日付は降順
        (true, false) => std::cmp::Ordering::Less, // 日付を先頭側へ
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => natural_cmp(&a.name, &b.name),
    });
    nodes
}

/// ディレクトリ 1 階層を読み、子ノード列を昇順（レース番号は数値順）で返す。空ディレクトリは除外。
fn read_dir_nodes(dir: &Path, root: &Path) -> Vec<Node> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            let children = read_dir_nodes(&path, root);
            if children.is_empty() {
                continue;
            }
            out.push(Node {
                name,
                path: None,
                children,
            });
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let display = name.strip_suffix(".md").unwrap_or(&name).to_string();
            out.push(Node {
                name: display,
                path: Some(rel),
                children: Vec::new(),
            });
        }
    }
    out.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    out
}

/// 8 桁数字（`YYYYMMDD`）なら日付ディレクトリとみなす。
fn is_date(name: &str) -> bool {
    name.len() == 8 && name.bytes().all(|b| b.is_ascii_digit())
}

/// 先頭が数字同士なら数値比較（`4R` < `10R`）、それ以外は辞書順。
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (leading_num(a), leading_num(b)) {
        (Some(na), Some(nb)) => na.cmp(&nb).then_with(|| a.cmp(b)),
        _ => a.cmp(b),
    }
}

fn leading_num(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn is_date_detects_8_digits() {
        assert!(is_date("20260613"));
        assert!(!is_date("hanshin"));
        assert!(!is_date("2026061")); // 7 桁
        assert!(!is_date("2026061a")); // 数字以外混入
        assert!(!is_date("_analysis"));
    }

    #[test]
    fn natural_cmp_orders_races_numerically() {
        // 文字列順だと "10R" < "4R" になってしまうのを数値順で正す。
        assert_eq!(natural_cmp("4R", "10R"), Ordering::Less);
        assert_eq!(natural_cmp("10R", "4R"), Ordering::Greater);
        assert_eq!(natural_cmp("07R", "7R"), Ordering::Less); // 同値はゼロ埋め有無で辞書順 tie-break
        // 先頭が数字でない場合は辞書順。
        assert_eq!(natural_cmp("hanshin", "nakayama"), Ordering::Less);
    }

    #[test]
    fn build_tree_orders_dates_desc_and_races_asc() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for (date, venue, races) in [
            ("20260607", "hanshin", &["10R", "4R"][..]),
            ("20260613", "tokyo", &["1R"][..]),
        ] {
            let dir = root.join(date).join(venue);
            std::fs::create_dir_all(&dir).unwrap();
            for r in races {
                std::fs::write(dir.join(format!("{r}.md")), "# t").unwrap();
            }
        }

        let tree = build_tree(root);
        // 日付は降順: 20260613 が先頭。
        assert_eq!(tree[0].name, "20260613");
        assert_eq!(tree[1].name, "20260607");
        // レースは数値昇順: 4R が 10R より前。
        let hanshin = &tree[1].children[0];
        assert_eq!(hanshin.name, "hanshin");
        assert_eq!(hanshin.children[0].name, "4R");
        assert_eq!(hanshin.children[1].name, "10R");
        // ファイルノードは pad 相対パスを持つ。
        assert_eq!(
            hanshin.children[0].path.as_deref(),
            Some("20260607/hanshin/4R.md")
        );
    }

    #[test]
    fn build_tree_skips_empty_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("20260613/empty")).unwrap();
        assert!(build_tree(tmp.path()).is_empty());
    }
}
