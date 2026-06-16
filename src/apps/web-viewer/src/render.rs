use std::path::Path;

use pulldown_cmark::{Options, Parser, html};

#[derive(Debug)]
pub enum RenderError {
    /// 不正なパス（traversal・拡張子違い等）。
    Invalid,
    /// 該当ファイルが無い・読めない。
    NotFound,
    /// pad_dir 自体が解決できない等のサーバ側問題。
    Server,
}

/// pad_dir からの相対パスで指定された MD を読み、HTML 断片にレンダリングする。
///
/// パストラバーサル対策: `..` を含むパスを拒否し、canonicalize 後に pad_dir 配下かつ
/// `.md` 拡張子であることを検証する。pad/ 配下のファイルは Claude 自身が書いた信頼済み
/// コンテンツなので、MD 内の生 HTML はそのまま通す（ローカル単一ユーザー）。
pub fn render_doc(pad_dir: &Path, rel: &str) -> Result<String, RenderError> {
    if rel.is_empty() || rel.contains("..") {
        return Err(RenderError::Invalid);
    }
    let candidate = pad_dir.join(rel);
    if candidate.extension().and_then(|e| e.to_str()) != Some("md") {
        return Err(RenderError::Invalid);
    }
    let root_canon = pad_dir.canonicalize().map_err(|_| RenderError::Server)?;
    let canon = candidate
        .canonicalize()
        .map_err(|_| RenderError::NotFound)?;
    if !canon.starts_with(&root_canon) {
        return Err(RenderError::Invalid);
    }

    let md = std::fs::read_to_string(&canon).map_err(|_| RenderError::NotFound)?;

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(&md, options);
    let mut html_out = String::new();
    html::push_html(&mut html_out, parser);
    Ok(html_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad_with_md() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("20260613/hanshin")).unwrap();
        std::fs::write(
            tmp.path().join("20260613/hanshin/4R.md"),
            "# 見出し\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        )
        .unwrap();
        tmp
    }

    #[test]
    fn renders_table_to_html() {
        let tmp = pad_with_md();
        let html = render_doc(tmp.path(), "20260613/hanshin/4R.md").unwrap();
        assert!(html.contains("<table>"));
        assert!(html.contains("<h1>見出し</h1>"));
    }

    #[test]
    fn rejects_parent_traversal() {
        let tmp = pad_with_md();
        assert!(matches!(
            render_doc(tmp.path(), "../escape.md"),
            Err(RenderError::Invalid)
        ));
    }

    #[test]
    fn rejects_non_md_extension() {
        let tmp = pad_with_md();
        assert!(matches!(
            render_doc(tmp.path(), "20260613/hanshin/4R.txt"),
            Err(RenderError::Invalid)
        ));
    }

    #[test]
    fn rejects_empty_path() {
        let tmp = pad_with_md();
        assert!(matches!(
            render_doc(tmp.path(), ""),
            Err(RenderError::Invalid)
        ));
    }

    #[test]
    fn missing_file_is_not_found() {
        let tmp = pad_with_md();
        assert!(matches!(
            render_doc(tmp.path(), "20260613/hanshin/99R.md"),
            Err(RenderError::NotFound)
        ));
    }

    #[test]
    fn rejects_absolute_symlink_escape() {
        // pad 配下に pad 外を指す symlink を置いても、canonicalize 後の
        // starts_with 判定で root 外として弾かれること。
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.md"), "# secret").unwrap();
        let tmp = pad_with_md();
        let link = tmp.path().join("link.md");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path().join("secret.md"), &link).unwrap();
            assert!(matches!(
                render_doc(tmp.path(), "link.md"),
                Err(RenderError::Invalid)
            ));
        }
    }
}
