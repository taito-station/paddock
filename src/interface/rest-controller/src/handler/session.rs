use actix_web::{HttpResponse, web};
use chrono::{DateTime, NaiveDate};

use paddock_domain::RaceId;
use paddock_use_case::repository::{BetFixSource, PredictBetRecord, Repository};
use paddock_use_case::{Interactor, OddsInteractor, OddsScraper};

use crate::error::{Error, Result};
use crate::schema::session::{
    CreateSessionRequest, FixSourceInput, OddsRefreshResponse, RecordOutcomeRequest,
    SessionSummaryResponse,
};

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| Error::BadRequest(format!("invalid date '{s}' (YYYY-MM-DD): {e}")))
}

/// `fix_source.odds_fetched_at` の長さの上限（#795）。RFC3339 の UTC 時刻は小数秒 9 桁でも 35 文字程度に
/// 収まる。巨大な文字列を DB に書かせない・パースさせないための門で、正当な値を弾かない余裕を持たせた値。
const MAX_ODDS_FETCHED_AT_LEN: usize = 64;

/// エラー文に埋める入力値の先頭の長さ。巨大な値をそのままレスポンスに載せないよう、長さだけ切り詰める
/// （制御文字は JSON 化でエスケープされるので、ここでは扱わない）。
const ERROR_ECHO_LEN: usize = 40;

fn echo_head(s: &str) -> String {
    s.chars().take(ERROR_ECHO_LEN).collect()
}

/// 記録の出所（#795）を検証して use-case の型にする。見るのは kind が 3 値のどれかと、kind と時刻の組み合わせ
/// （`morning` だけ時刻必須）、時刻が長さの上限内・RFC3339 として読める・UTC（`Z` / `+00:00`）であること。
/// 「買い目ありで出所なし」は use-case が `InvalidArgument`（400）で弾くので、ここでは判定しない。
///
/// 時刻の文字列は正規化せずそのまま保存する。盤が返す `morning_selections.odds_fetched_at` と文字列で
/// 突き合わせられるようにするため（正規化すると `Z` と `+00:00` の違いで一致しなくなる）。
fn parse_fix_source(input: Option<FixSourceInput>) -> Result<Option<BetFixSource>> {
    let Some(FixSourceInput {
        kind,
        odds_fetched_at,
    }) = input
    else {
        return Ok(None);
    };
    let source = match (kind.as_str(), odds_fetched_at) {
        ("morning", Some(at)) => {
            if at.len() > MAX_ODDS_FETCHED_AT_LEN {
                return Err(Error::BadRequest(format!(
                    "fix_source.odds_fetched_at is too long (max {MAX_ODDS_FETCHED_AT_LEN} bytes)"
                )));
            }
            let parsed = DateTime::parse_from_rfc3339(&at).map_err(|e| {
                Error::BadRequest(format!(
                    "invalid fix_source.odds_fetched_at '{}' (RFC3339): {e}",
                    echo_head(&at)
                ))
            })?;
            // chrono の parse_rfc3339 は日付と時刻の区切りに `t`・空白、UTC に `z`・`-00:00`・U+2212 も受け付ける。
            // 時刻は正規化せず保存し、盤の `morning_selections.odds_fetched_at` と文字列で突き合わせる前提なので、
            // 盤が返す形（区切りは `T`、末尾は `Z` か `+00:00`）以外は弾く。
            let canonical_form = at.as_bytes().get(10) == Some(&b'T')
                && (at.ends_with('Z') || at.ends_with("+00:00"));
            if parsed.offset().local_minus_utc() != 0 || !canonical_form {
                return Err(Error::BadRequest(format!(
                    "fix_source.odds_fetched_at '{}' must be UTC with 'T' separator (ending in Z or +00:00)",
                    echo_head(&at)
                )));
            }
            BetFixSource::Morning {
                odds_fetched_at: at,
            }
        }
        ("morning", None) => {
            return Err(Error::BadRequest(
                "fix_source.odds_fetched_at is required for kind 'morning'".to_string(),
            ));
        }
        ("recorded_axis" | "none", Some(_)) => {
            return Err(Error::BadRequest(format!(
                "fix_source.odds_fetched_at must be absent for kind '{kind}'"
            )));
        }
        ("recorded_axis", None) => BetFixSource::RecordedAxis,
        ("none", None) => BetFixSource::None,
        _ => {
            return Err(Error::BadRequest(format!(
                "invalid fix_source.kind '{}' (morning / recorded_axis / none)",
                echo_head(&kind)
            )));
        }
    };
    Ok(Some(source))
}

/// セッション新規作成。
#[utoipa::path(
    post,
    path = "/api/sessions/{date}",
    params(("date" = String, Path, description = "開催日 YYYY-MM-DD")),
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "作成したセッションのサマリ", body = SessionSummaryResponse),
        (status = 400, description = "budget 不正・日付不正", body = crate::error::ErrorBody),
        (status = 409, description = "同一開催日のセッションが既に存在", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn create_session<R>(
    interactor: web::Data<Interactor<R>>,
    path: web::Path<String>,
    body: web::Json<CreateSessionRequest>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
{
    let date = parse_date(&path.into_inner())?;
    let session = interactor.create_predict_session(date, body.budget).await?;
    // 新規作成直後は買い目も見送りも空。
    Ok(HttpResponse::Created().json(SessionSummaryResponse::new(&session, &[], &[])))
}

/// セッション収支サマリ + 明細。
#[utoipa::path(
    get,
    path = "/api/sessions/{date}",
    params(("date" = String, Path, description = "開催日 YYYY-MM-DD")),
    responses(
        (status = 200, description = "セッション収支 + 明細", body = SessionSummaryResponse),
        (status = 400, description = "日付不正", body = crate::error::ErrorBody),
        (status = 404, description = "未作成のセッション", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn get_session_summary<R>(
    interactor: web::Data<Interactor<R>>,
    path: web::Path<String>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
{
    let date = parse_date(&path.into_inner())?;
    let (session, bets) = interactor.session_summary(date).await?;
    let skips = interactor.find_predict_race_skips(date).await?;
    Ok(HttpResponse::Ok().json(SessionSummaryResponse::new(&session, &bets, &skips)))
}

/// 1 レース分の賭け金・払戻を記録（残高ガード・1 トランザクション）。
#[utoipa::path(
    post,
    path = "/api/sessions/{date}/races/{race_id}/outcome",
    params(
        ("date" = String, Path, description = "開催日 YYYY-MM-DD"),
        ("race_id" = String, Path, description = "レース ID"),
    ),
    request_body = RecordOutcomeRequest,
    responses(
        (status = 200, description = "更新後のセッションサマリ", body = SessionSummaryResponse),
        (status = 400, description = "残高超過・日付/ID 不正・買い目ありで fix_source なし・fix_source 不正", body = crate::error::ErrorBody),
        (status = 404, description = "未作成のセッション", body = crate::error::ErrorBody),
        (status = 409, description = "当該レースの outcome が既に記録済み", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn record_outcome<R>(
    interactor: web::Data<Interactor<R>>,
    path: web::Path<(String, String)>,
    body: web::Json<RecordOutcomeRequest>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
{
    let (date_str, race_id_str) = path.into_inner();
    let date = parse_date(&date_str)?;
    let race_id = RaceId::try_from(race_id_str)?;

    let body = body.into_inner();
    let fix_source = parse_fix_source(body.fix_source)?;
    let bets: Vec<PredictBetRecord> = body
        .bets
        .into_iter()
        .map(|b| PredictBetRecord {
            race_id: race_id.clone(),
            bet_type: b.bet_type,
            combination: b.combination,
            stake: b.stake,
            payout: b.payout,
            ev: b.ev,
        })
        .collect();

    // record_race_outcome が更新後 session を返すので再取得せず、明細 bets と見送り痕跡のみ取り直す。
    let session = interactor
        .record_race_outcome(date, &race_id, bets, fix_source)
        .await?;
    let all_bets = interactor.find_predict_bets(date).await?;
    let skips = interactor.find_predict_race_skips(date).await?;
    Ok(HttpResponse::Ok().json(SessionSummaryResponse::new(&session, &all_bets, &skips)))
}

/// オッズをライブ取得して保存（#51, read-through）。
#[utoipa::path(
    post,
    path = "/api/sessions/{date}/races/{race_id}/odds:refresh",
    params(
        ("date" = String, Path, description = "開催日 YYYY-MM-DD"),
        ("race_id" = String, Path, description = "レース ID"),
    ),
    responses(
        (status = 200, description = "取得結果（未取得は fetched=false・HTTP は 200）", body = OddsRefreshResponse),
        (status = 400, description = "日付/ID 不正", body = crate::error::ErrorBody),
        (status = 404, description = "未作成のセッション", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー", body = crate::error::ErrorBody),
    ),
    tag = "sessions",
)]
pub async fn odds_refresh<R, O>(
    interactor: web::Data<Interactor<R>>,
    odds: web::Data<OddsInteractor<O, R>>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    O: OddsScraper + Send + Sync + 'static,
{
    let (date_str, race_id_str) = path.into_inner();
    let date = parse_date(&date_str)?;
    let race_id = RaceId::try_from(race_id_str)?;

    // セッション文脈下の操作として存在を要求する（race_odds 自体はセッション非依存だが SPA の一貫性のため）。
    if interactor.find_predict_session(date).await?.is_none() {
        return Err(Error::NotFound(format!("session for {date} not found")));
    }

    let fetched = odds.race_odds(&race_id).await?.is_some();
    Ok(HttpResponse::Ok().json(OddsRefreshResponse {
        race_id: race_id.value().to_string(),
        fetched,
    }))
}
