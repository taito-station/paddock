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
            let display = name.trim_end_matches(".md").to_string();
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
