use std::sync::LazyLock;

use chrono::{NaiveDate, NaiveTime};
use paddock_domain::{
    GateNum, HorseId, HorseName, HorseNum, JockeyName, RaceClass, Surface, TrainerName,
};
use paddock_use_case::netkeiba_scraper::{FetchedCard, FetchedEntry};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};

use super::shutuba::extract_horse_id;
use super::{cell_text, round_day_racenum, venue_from_race_id};
use crate::error::{Error, Result};

/// `芝1600m` 等から馬場と距離を取る正規表現（呼び出しごとの再コンパイルを避け static 化）。
static SURFACE_DISTANCE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([芝ダ障])\s*(\d{3,4})m").expect("valid surface/distance regex"));

/// `YYYY年M月D日` の開催日表記を取る正規表現。
static DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4})年(\d{1,2})月(\d{1,2})日").expect("valid date regex"));

/// `15:40発走` 等の発走時刻表記を取る正規表現（#235）。
static POST_TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2}):(\d{2})発走").expect("valid post_time regex"));

/// 出馬表 (`race/shutuba.html`) のHTMLから当日のレースカード（メタ + 全出走馬）を抽出する。
///
/// venue/round/day/race_num は 12 桁 race_id から導出し、surface/distance は
/// `div.RaceData01` のテキスト、開催日は HTML 全体の `YYYY年M月D日` 表記から取る。
/// 各行（`tr.HorseList`）から枠番・馬番・馬名・騎手を読む（馬名は静的 HTML に含まれる）。
pub fn parse_card(html: &str, netkeiba_race_id: &str) -> Result<FetchedCard> {
    let venue = venue_from_race_id(netkeiba_race_id)
        .ok_or_else(|| Error::Parse(format!("JRA 外もしくは不正な race_id: {netkeiba_race_id}")))?;
    let (round, day, race_num) = round_day_racenum(netkeiba_race_id).ok_or_else(|| {
        Error::Parse(format!(
            "race_id から回/日/R を読めません: {netkeiba_race_id}"
        ))
    })?;

    // race_id 先頭 4 桁が開催年。開催日の照合に使う。
    let year: i32 = netkeiba_race_id
        .get(0..4)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::Parse(format!("race_id から年を読めません: {netkeiba_race_id}")))?;

    let doc = Html::parse_document(html);

    let (surface, distance) = extract_surface_distance(&doc)?;
    let date = extract_date(&doc, year)?;
    let post_time = extract_post_time(&doc);
    let race_class = extract_race_class(&doc);
    let entries = extract_entries(&doc)?;

    Ok(FetchedCard {
        date,
        post_time,
        venue,
        round,
        day,
        race_num,
        surface,
        distance,
        race_class,
        entries,
    })
}

/// `<title>` のグレード表記（例「安田記念(G1)」）と `div.RaceData02` の条件表記
/// （オープン/n勝クラス/未勝利/新馬）を結合し、レースクラスを正規化する（#345）。
///
/// グレードは `<title>` にのみ現れ（`h1.RaceName` は「安田記念」でグレードを含まない）、
/// 条件は RaceData02 に現れるため両方を結合したラベルを `RaceClass::from_label` に渡す。
/// 判定できなければ `None`（best-effort：クラスはカード取得の必須項目ではなく、欠けても
/// カード保存を止めない）。
fn extract_race_class(doc: &Html) -> Option<RaceClass> {
    let title = sel("title")
        .ok()
        .and_then(|s| doc.select(&s).next().map(|t| t.text().collect::<String>()))
        .unwrap_or_default();
    let condition = sel("div.RaceData02")
        .ok()
        .and_then(|s| doc.select(&s).next().map(|d| d.text().collect::<String>()))
        .unwrap_or_default();
    let label = format!("{title} {condition}");
    let class = RaceClass::from_label(&label);
    if class.is_none() {
        // title も RaceData02 も取れているのにクラスを判定できないのは netkeiba の
        // 表記変更の兆候になりうる。best-effort で None に落とすが debug で可視化する。
        tracing::debug!("title/RaceData02 からレースクラスを判定できませんでした");
    }
    class
}

/// `div.RaceData01` の「HH:MM発走」から発走時刻を読む（#235）。surface/distance と同じ
/// RaceData01 テキストにあるため追加リクエストは不要。取れない場合は `None`（best-effort：
/// 発走時刻はカード取得の必須項目ではなく、欠けてもカード保存を止めない）。
fn extract_post_time(doc: &Html) -> Option<NaiveTime> {
    let data_sel = sel("div.RaceData01").ok()?;
    let text = doc.select(&data_sel).next()?.text().collect::<String>();
    let Some(caps) = POST_TIME_RE.captures(&text) else {
        // RaceData01 自体は取れている（surface/distance は読めている）のに発走時刻だけ
        // 読めないのは netkeiba のレイアウト変更の兆候になりうる。best-effort で None に
        // 落とすが、将来の取得失敗を切り分けられるよう debug で可視化する。
        tracing::debug!("RaceData01 から発走時刻(HH:MM発走)を読めませんでした");
        return None;
    };
    let hour: u32 = caps[1].parse().ok()?;
    let min: u32 = caps[2].parse().ok()?;
    NaiveTime::from_hms_opt(hour, min, 0)
}

/// `div.RaceData01` のテキストから `芝1600m` 等を読み、Surface と距離(m)に変換する。
/// 障害(障)は対応外として parse error。
fn extract_surface_distance(doc: &Html) -> Result<(Surface, u32)> {
    let data_sel = sel("div.RaceData01")?;
    let text = doc
        .select(&data_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .ok_or_else(|| Error::Parse("RaceData01 が見つかりません".to_string()))?;

    let caps = SURFACE_DISTANCE_RE
        .captures(&text)
        .ok_or_else(|| Error::Parse(format!("芝/ダ/距離を読めません: {text:?}")))?;

    let surface = match &caps[1] {
        "芝" => Surface::Turf,
        "ダ" => Surface::Dirt,
        "障" => {
            return Err(Error::Parse("障害レースは対応外です".to_string()));
        }
        other => return Err(Error::Parse(format!("unknown surface marker: {other}"))),
    };
    let distance: u32 = caps[2]
        .parse()
        .map_err(|_| Error::Parse(format!("invalid distance: {}", &caps[2])))?;
    Ok((surface, distance))
}

/// `YYYY年M月D日` の開催日を読み取る。
///
/// `<title>`（"… | YYYY年M月D日 <場>NNR …" 形式のレース正規情報）を最優先で見る。
/// 本文の近走欄・広告に同型の日付が混在し得るため、信頼できる title へスコープを絞る。
/// title から取れない場合のみ文書全体テキストへフォールバックし、いずれも
/// `expected_year` 一致を優先する（同年が無ければ最初の妥当な日付）。
fn extract_date(doc: &Html, expected_year: i32) -> Result<NaiveDate> {
    let title = sel("title")
        .ok()
        .and_then(|s| doc.select(&s).next().map(|t| t.text().collect::<String>()));
    if let Some(date) = title.as_deref().and_then(|t| date_in(t, expected_year)) {
        return Ok(date);
    }
    // title から取れなかった = 想定レイアウト外。本文には近走等の別日付が混ざり得るため、
    // フォールバック採用を warn で可視化する（誤抽出の調査の手掛かりにする）。
    tracing::warn!("開催日を <title> から取得できず本文テキストへフォールバックします");
    let body = doc.root_element().text().collect::<String>();
    date_in(&body, expected_year)
        .ok_or_else(|| Error::Parse("開催日(YYYY年M月D日)が見つかりません".to_string()))
}

/// テキストから `YYYY年M月D日` を拾う。`expected_year` 一致を優先し、無ければ最初の妥当な日付。
fn date_in(text: &str, expected_year: i32) -> Option<NaiveDate> {
    let mut first: Option<NaiveDate> = None;
    for caps in DATE_RE.captures_iter(text) {
        let (Ok(y), Ok(m), Ok(d)) = (
            caps[1].parse::<i32>(),
            caps[2].parse::<u32>(),
            caps[3].parse::<u32>(),
        ) else {
            continue;
        };
        let Some(date) = NaiveDate::from_ymd_opt(y, m, d) else {
            continue;
        };
        if y == expected_year {
            return Some(date);
        }
        first.get_or_insert(date);
    }
    first
}

/// 出馬表テーブルの各行から枠番・馬番・馬名・騎手を抽出する。
fn extract_entries(doc: &Html) -> Result<Vec<FetchedEntry>> {
    let row_sel = sel("table.Shutuba_Table tr.HorseList")?;
    let waku_sel = sel("td[class^=\"Waku\"] span")?;
    let umaban_sel = sel("td[class^=\"Umaban\"]")?;
    let horse_link_sel = sel("td.HorseInfo a[href*=\"/horse/\"]")?;
    let jockey_sel = sel("td.Jockey a")?;
    let trainer_sel = sel("td.Trainer a")?;
    // 斤量は性齢セル(td.Barei)の直後の td（class="Txt_C"）。td.Txt_C は枠/馬番/人気にも付くため
    // 隣接セレクタで斤量列だけを特定する（#135）。
    let weight_sel = sel("td.Barei + td")?;

    let mut entries = Vec::new();
    for row in doc.select(&row_sel) {
        let Some(gate_num) = row
            .select(&waku_sel)
            .next()
            .and_then(|c| cell_text(&c.text().collect::<String>()))
            .and_then(|t| t.parse::<u32>().ok())
            .and_then(|n| GateNum::try_from(n).ok())
        else {
            continue;
        };
        let Some(horse_num) = row
            .select(&umaban_sel)
            .next()
            .and_then(|c| cell_text(&c.text().collect::<String>()))
            .and_then(|t| t.parse::<u32>().ok())
            .and_then(|n| HorseNum::try_from(n).ok())
        else {
            continue;
        };
        // 馬名と horse_id は同じ `/horse/<id>` リンク（title=馬名, href=ID）から取る。
        // 馬名が無い行（th のみ等）はスキップ。horse_id は近走取り込み（#103）の再利用キーで、
        // 出馬表保存の必須項目ではないため、欠けても entry は落とさず Option として持つ。
        let Some(horse_link) = row.select(&horse_link_sel).next() else {
            continue;
        };
        let Some(horse_name) = horse_link
            .value()
            .attr("title")
            .and_then(cell_text)
            .and_then(|n| HorseName::try_from(n).ok())
        else {
            continue;
        };
        let horse_id = horse_link
            .value()
            .attr("href")
            .and_then(extract_horse_id)
            .and_then(|id| HorseId::try_from(id).ok());
        let jockey = extract_jockey(&row, &jockey_sel);
        let trainer = extract_trainer(&row, &trainer_sel);
        let weight_carried = extract_weight(&row, &weight_sel);

        entries.push(FetchedEntry {
            gate_num,
            horse_num,
            horse_name,
            horse_id,
            jockey,
            trainer,
            weight_carried,
        });
    }

    if entries.is_empty() {
        return Err(Error::Parse(
            "出馬表テーブルから出走馬を抽出できませんでした".to_string(),
        ));
    }
    Ok(entries)
}

/// `td.Jockey a` の `title` 属性から騎手名を取る。取れなければ None。
fn extract_jockey(row: &ElementRef, sel: &Selector) -> Option<JockeyName> {
    let link = row.select(sel).next()?;
    link.value()
        .attr("title")
        .and_then(cell_text)
        .and_then(|n| JockeyName::try_from(n).ok())
}

/// `td.Trainer a` の `title` 属性から調教師名を取る（#74）。取れなければ None。
fn extract_trainer(row: &ElementRef, sel: &Selector) -> Option<TrainerName> {
    let link = row.select(sel).next()?;
    link.value()
        .attr("title")
        .and_then(cell_text)
        .and_then(|n| TrainerName::try_from(n).ok())
}

/// 性齢セル直後の斤量セル（`td.Barei + td`）から負担重量[kg]を取る（#135）。空・非数値は None。
/// `td.Barei + td` は DOM 順依存（性齢列の直後が斤量）のため、レイアウト変更で別カラムを掴んでも
/// 不正値が確率推定へ流れないよう、現実的な斤量レンジ（40〜70kg）外は `None` に落とす防御を入れる。
fn extract_weight(row: &ElementRef, sel: &Selector) -> Option<f64> {
    let cell = row.select(sel).next()?;
    cell_text(&cell.text().collect::<String>())?
        .parse::<f64>()
        .ok()
        .filter(|w| (40.0..=70.0).contains(w))
}

fn sel(s: &str) -> Result<Selector> {
    Selector::parse(s).map_err(|e| Error::Parse(format!("invalid selector {s}: {e}")))
}
