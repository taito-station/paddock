use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-ingest-predictions",
    about = "予想（印・短評・買い目・結果）の JSON を DB に保存し、DB から pad の MD を生成する",
    version
)]
pub struct Cli {
    /// 取り込む予想 JSON のパス。省略時は標準入力から読む（ingest 時のみ）。
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// パース・検証のみ行い DB に保存しない（ingest 時）。
    #[arg(long)]
    pub dry_run: bool,

    /// DB の全予想を pad の MD に出力する（取り込みではなく生成モード）。
    #[arg(long)]
    pub render: bool,

    /// 生成先 pad ルート。省略時は環境変数 `PADDOCK_PAD_DIR`、無ければ既定の vault パス。
    #[arg(long)]
    pub pad_dir: Option<PathBuf>,
}
