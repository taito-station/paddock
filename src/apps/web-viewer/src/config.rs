use std::path::PathBuf;

/// 既定の pad ディレクトリ（iCloud Obsidian vault）。
/// 環境変数 `PADDOCK_PAD_DIR` で上書きできる。
const DEFAULT_PAD_DIR: &str =
    "/Users/ito-taito/Library/Mobile Documents/iCloud~md~obsidian/Documents/default/pad";
const DEFAULT_PORT: u16 = 8787;

/// pad ビューアの設定。DB 用 `paddock-config` とは無関係なファイルシステム専用設定なので、
/// 共有 Config には足さずローカルに環境変数を読む。
#[derive(Clone)]
pub struct PadConfig {
    pub pad_dir: PathBuf,
    pub port: u16,
}

impl PadConfig {
    pub fn from_env() -> Self {
        let pad_dir = std::env::var("PADDOCK_PAD_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_PAD_DIR));
        let port = std::env::var("PAD_WEB_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        Self { pad_dir, port }
    }
}
