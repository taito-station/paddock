use std::path::Path;

use pulldown_cmark::{Options, Parser, html};

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
