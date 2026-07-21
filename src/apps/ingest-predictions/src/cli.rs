use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-ingest-predictions",
    about = "予想（印・短評・買い目・結果）の JSON を DB に保存する",
    version
)]
pub struct Cli {
    /// 取り込む予想 JSON のパス。省略時は標準入力から読む。
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// パース・検証のみ行い DB に保存しない。
    #[arg(long)]
    pub dry_run: bool,
}
