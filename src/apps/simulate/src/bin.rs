mod cli;
mod input;

use std::io::Read;

use anyhow::{Context, Result};
use clap::Parser;
use paddock_domain::{Finish, SimInput, SimReport, simulate};

fn main() -> Result<()> {
    let args = cli::Cli::parse();

    let raw = match &args.input {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("入力 JSON を読めません: {}", path.display()))?,
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("標準入力を読めません")?;
            buf
        }
    };

    let json: input::InputJson = serde_json::from_str(&raw).context("入力 JSON の解釈に失敗")?;
    let sim = input::to_sim_input(json, args.main.as_deref())?;
    let report = simulate(&sim)?;
    print_report(&sim, &report);
    Ok(())
}

fn format_signed(v: i128) -> String {
    if v >= 0 {
        format!("+¥{v}")
    } else {
        format!("-¥{}", -v)
    }
}

fn fmt_finish(f: Finish) -> String {
    format!("{}-{}-{}", f.0.value(), f.1.value(), f.2.value())
}

fn print_report(sim: &SimInput, r: &SimReport) {
    println!("=== 買い目ポートフォリオ収支シミュレーション ===");
    println!(
        "出走 {} 頭 / 買い目 {} 点 / 総賭け金 ¥{}",
        sim.field.len(),
        sim.bets.len(),
        r.total_stake
    );

    println!();
    println!("[買い目]");
    for b in &sim.bets {
        println!(
            "  {} {}  ¥{} @ {:.1}倍",
            b.combination.bet_type().as_ja(),
            b.combination.combination_code(),
            b.stake,
            b.odds
        );
    }

    println!();
    println!("[結果]");
    println!(
        "ベストケース: {} 着  払戻 ¥{}  収支 {}",
        fmt_finish(r.best.finish),
        r.best.payout,
        format_signed(r.best.pnl)
    );

    match &r.worst_hit {
        Some(w) if w.pnl < 0 => println!(
            "当たっても赤字: あり  最小払戻 ¥{} ({} 着)  収支 {}",
            w.payout,
            fmt_finish(w.finish),
            format_signed(w.pnl)
        ),
        Some(w) => println!("当たっても赤字: なし（的中時の最小払戻 ¥{}）", w.payout),
        None => println!("当たる着順: なし（どの着順でも 1 点も的中しません）"),
    }

    println!("的中する着順: {} / {} 通り", r.hit_count, r.total_count);

    if let Some(m) = &r.main {
        println!(
            "本線 {} 着: 払戻 ¥{}  収支 {}",
            fmt_finish(m.finish),
            m.payout,
            format_signed(m.pnl)
        );
    }

    if let Some(ev) = &r.ev {
        println!();
        println!("[期待値（勝率入力より）]");
        println!("期待払戻: ¥{:.0}", ev.ev);
        println!("期待回収率: {:.1}%", ev.roi * 100.0);
        println!("的中確率: {:.1}%", ev.hit_prob * 100.0);
    }
}
