use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-simulate",
    about = "買い目ポートフォリオの収支シミュレータ（全着順を列挙して払戻・収支を集計）",
    version
)]
pub struct Cli {
    /// 買い目定義 JSON のパス。省略時は標準入力から読む。
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// 本線の着順（例: `5-1-3`）。指定すると JSON の `main` を上書きする。
    #[arg(long)]
    pub main: Option<String>,
}
