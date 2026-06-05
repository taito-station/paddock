use std::io::{self, Write};

use chrono::NaiveDate;
use paddock_domain::{
    BetCombination, BettingConfig, BettingRecommendation, HorseProbability, Race, Surface,
    select_bets,
};

use crate::setup::App;

/// セッション中の残高・累計（App 層のみで管理。残高ガードにより budget は常に 0 以上）。
struct SessionState {
    budget: u64,
    total_bet: u64,
    total_payout: u64,
}

impl SessionState {
    fn new(budget: u64) -> Self {
        Self {
            budget,
            total_bet: 0,
            total_payout: 0,
        }
    }
}

/// 1 日分のレースを順番に処理する対話セッション。
pub async fn run_session(app: &App, date: NaiveDate, budget: u64) -> anyhow::Result<()> {
    let races = app.interactor.races_by_date(date).await?;
    if races.is_empty() {
        println!("この日の開催はありません: {}", date.format("%Y-%m-%d"));
        return Ok(());
    }

    println!(
        "=== {} 開催 — {} レース ===",
        date.format("%Y-%m-%d"),
        races.len()
    );
    println!("初期予算: ¥{budget}");

    let mut state = SessionState::new(budget);
    for race in &races {
        run_race(app, race, &mut state).await?;
    }

    println!();
    println!("=== {} 終了 ===", date.format("%Y-%m-%d"));
    print_summary(&state);
    Ok(())
}

async fn run_race(app: &App, race: &Race, state: &mut SessionState) -> anyhow::Result<()> {
    println!();
    println!(
        "--- レース {}: {} {} {}m ---",
        race.race_num,
        race.venue.as_jp(),
        surface_jp(race.surface),
        race.distance
    );
    println!("残高: ¥{}", state.budget);

    let probs = match app.interactor.predict_race(&race.race_id).await {
        Ok(p) => p,
        Err(e) => {
            println!("確率推定をスキップします（{e}）");
            return Ok(());
        }
    };

    println!();
    print_probs(&probs);

    // オッズ未取得（None）はスキップのみ受付（select_bets は呼ばない）
    let Some(odds) = app.interactor.race_odds(&race.race_id).await? else {
        println!();
        println!("オッズ未取得 — このレースはスキップします");
        let _ = read_line("[s=スキップ] > ")?;
        return Ok(());
    };

    let recs = select_bets(&probs, &odds, &BettingConfig::default());
    let kelly_fractions: Vec<f64> = recs.iter().map(|r| r.kelly_fraction).collect();
    let suggested = recommended_amounts(state.budget, &kelly_fractions);

    println!();
    println!("【買い目推奨】");
    if recs.is_empty() {
        println!("  EV 閾値を超える買い目なし");
    }
    for (rec, amt) in recs.iter().zip(&suggested) {
        println!(
            "  {} EV={:.2} Kelly={:.0}% 推奨額=¥{}",
            format_combination(&rec.combination),
            rec.ev,
            rec.kelly_fraction * 100.0,
            amt,
        );
    }

    println!();
    let bet_amounts: Vec<u64> = match read_choice()? {
        's' => return Ok(()),
        'y' => suggested.clone(),
        'e' => read_edited_amounts(&recs, &suggested, state.budget)?,
        _ => unreachable!("read_choice returns only y/e/s"),
    };

    let bet: u64 = bet_amounts.iter().sum();
    if bet == 0 {
        println!("賭けなし — 次のレースへ");
        return Ok(());
    }
    // 残高ガード（y の比例縮小・e の入力チェックで保証されるが二重防御）
    if bet > state.budget {
        println!(
            "賭け金合計 ¥{} が残高 ¥{} を超えるためスキップします",
            bet, state.budget
        );
        return Ok(());
    }

    state.budget -= bet;
    state.total_bet += bet;

    println!();
    println!(">>> レース後 <<<");
    let payout = read_u64("実際の払い戻し額を入力 (なし: Enter のみ) > ", true)?;
    state.budget += payout;
    state.total_payout += payout;

    let pnl = payout as i64 - bet as i64;
    println!(
        "  賭け金: ¥{}  払戻: ¥{}  ({})",
        bet,
        payout,
        format_signed(pnl)
    );
    println!("残高: ¥{}", state.budget);

    Ok(())
}

/// Kelly 比例縮小方式で各買い目の推奨額を算出する。
///
/// 丸め前の実数合計 `Σ raw_i` を分母に使うことで、`floor` の単調性により
/// `Σ 推奨額 ≤ budget` を厳密に保証する（設計書 predict-session.md 参照）。
fn recommended_amounts(budget: u64, kelly_fractions: &[f64]) -> Vec<u64> {
    if kelly_fractions.is_empty() {
        return Vec::new();
    }
    let budget_f = budget as f64;
    let raws: Vec<f64> = kelly_fractions.iter().map(|k| budget_f * k).collect();
    let sum: f64 = raws.iter().sum();
    if sum <= budget_f {
        raws.iter().map(|r| r.floor() as u64).collect()
    } else {
        raws.iter()
            .map(|r| (r * budget_f / sum).floor() as u64)
            .collect()
    }
}

fn read_edited_amounts(
    recs: &[BettingRecommendation],
    suggested: &[u64],
    budget: u64,
) -> anyhow::Result<Vec<u64>> {
    loop {
        let mut amounts = Vec::with_capacity(recs.len());
        for (rec, sug) in recs.iter().zip(suggested) {
            let a = read_u64(
                &format!(
                    "  {} 推奨¥{} 入力額 > ",
                    format_combination(&rec.combination),
                    sug
                ),
                false,
            )?;
            amounts.push(a);
        }
        let total: u64 = amounts.iter().sum();
        if total > budget {
            println!(
                "合計 ¥{total} が残高 ¥{budget} を超えています。入力し直してください。"
            );
            continue;
        }
        return Ok(amounts);
    }
}

fn print_summary(state: &SessionState) {
    println!("総賭け金:  ¥{}", state.total_bet);
    println!("総払戻:    ¥{}", state.total_payout);
    println!("最終残高:  ¥{}", state.budget);
    let pnl = state.total_payout as i64 - state.total_bet as i64;
    println!("P&L:       {}", format_signed(pnl));
}

fn print_probs(probs: &[HorseProbability]) {
    println!(
        "{:<4} {:<16} {:>8} {:>8} {:>8}",
        "馬番", "馬名", "勝率", "連対率", "複勝率"
    );
    for p in probs {
        println!(
            "{:>4} {:<16} {:>7.1}% {:>7.1}% {:>7.1}%",
            p.horse_num.value(),
            p.horse_name.value(),
            p.win_prob * 100.0,
            p.place_prob * 100.0,
            p.show_prob * 100.0,
        );
    }
}

fn format_combination(c: &BetCombination) -> String {
    match c {
        BetCombination::Win(h) => format!("単勝 {}", h.value()),
        BetCombination::Place(h) => format!("複勝 {}", h.value()),
        BetCombination::Quinella(p) => {
            let (a, b) = p.as_tuple();
            format!("馬連 {}-{}", a.value(), b.value())
        }
        BetCombination::Exacta(p) => {
            let (a, b) = p.as_tuple();
            format!("馬単 {}→{}", a.value(), b.value())
        }
        BetCombination::Trio(t) => {
            let (a, b, c) = t.as_tuple();
            format!("三連複 {}-{}-{}", a.value(), b.value(), c.value())
        }
        BetCombination::Trifecta(t) => {
            let (a, b, c) = t.as_tuple();
            format!("三連単 {}→{}→{}", a.value(), b.value(), c.value())
        }
    }
}

fn surface_jp(s: Surface) -> &'static str {
    match s {
        Surface::Turf => "芝",
        Surface::Dirt => "ダート",
    }
}

fn format_signed(v: i64) -> String {
    if v >= 0 {
        format!("+¥{v}")
    } else {
        format!("-¥{}", v.abs())
    }
}

fn read_line(prompt: &str) -> io::Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// `y` / `e` / `s` のいずれかを読み取る（不正入力は再プロンプト）。
fn read_choice() -> anyhow::Result<char> {
    loop {
        let s = read_line("購入方法を選んでください [y=推奨通り / e=編集 / s=スキップ] > ")?;
        match s.as_str() {
            "y" | "Y" => return Ok('y'),
            "e" | "E" => return Ok('e'),
            "s" | "S" => return Ok('s'),
            _ => println!("y / e / s のいずれかを入力してください。"),
        }
    }
}

/// 非負整数を読み取る。`allow_empty_as_zero` が true なら空入力を 0 とみなす。
fn read_u64(prompt: &str, allow_empty_as_zero: bool) -> anyhow::Result<u64> {
    loop {
        let s = read_line(prompt)?;
        if s.is_empty() && allow_empty_as_zero {
            return Ok(0);
        }
        match s.parse::<u64>() {
            Ok(v) => return Ok(v),
            Err(_) => println!("数値を入力してください。"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::recommended_amounts;

    #[test]
    fn within_budget_keeps_floor() {
        // budget 10000, kelly 0.15/0.08/0.05 → 1500/800/500（合計 2800 ≤ 10000）
        let amounts = recommended_amounts(10000, &[0.15, 0.08, 0.05]);
        assert_eq!(amounts, vec![1500, 800, 500]);
    }

    #[test]
    fn four_quarter_kelly_exactly_fits() {
        // 0.25 × 4 = 1.0、raw 合計 = 10000 = budget（縮小不要）
        let amounts = recommended_amounts(10000, &[0.25, 0.25, 0.25, 0.25]);
        assert_eq!(amounts, vec![2500, 2500, 2500, 2500]);
        assert_eq!(amounts.iter().sum::<u64>(), 10000);
    }

    #[test]
    fn over_budget_scales_down_within_balance() {
        // 0.25 × 5 = 1.25、raw 合計 12500 > 10000 → 比例縮小で各 2000
        let amounts = recommended_amounts(10000, &[0.25, 0.25, 0.25, 0.25, 0.25]);
        let total: u64 = amounts.iter().sum();
        assert!(total <= 10000, "total {total} must be <= budget");
        assert_eq!(amounts, vec![2000, 2000, 2000, 2000, 2000]);
    }

    #[test]
    fn floor_residual_never_exceeds_budget() {
        // 丸め前合計を分母にすることで floor 残差でも budget を超えないこと
        let amounts = recommended_amounts(
            58,
            &[0.2203, 0.1163, 0.0605, 0.2041, 0.2055, 0.1673, 0.1646],
        );
        let total: u64 = amounts.iter().sum();
        assert!(total <= 58, "total {total} must be <= 58");
    }

    #[test]
    fn empty_returns_empty() {
        assert!(recommended_amounts(10000, &[]).is_empty());
    }

    #[test]
    fn zero_budget_returns_zeros() {
        let amounts = recommended_amounts(0, &[0.25, 0.1]);
        assert_eq!(amounts, vec![0, 0]);
    }
}
