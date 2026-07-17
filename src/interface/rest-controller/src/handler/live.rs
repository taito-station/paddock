use actix_web::{HttpResponse, web};
use chrono::{NaiveDate, SecondsFormat, Utc};

use paddock_use_case::Interactor;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;

use crate::error::{Error, Result};
use crate::schema::live::LiveResponse;

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| Error::BadRequest(format!("invalid date '{s}' (YYYY-MM-DD): {e}")))
}

/// 指定開催日のライブ EV 買い目（race ごと最新サイクル＋伝票＋フリップ）。
///
/// 該当日の snapshot が無ければ races 空・summary count 0（200 空・404 にしない）。
#[utoipa::path(
    get,
    path = "/api/live/{date}",
    params(("date" = String, Path, description = "開催日 YYYY-MM-DD")),
    responses(
        (status = 200, description = "race ごと最新サイクルの判定＋伝票（無い日は races 空・200）", body = LiveResponse),
        (status = 400, description = "日付フォーマット不正", body = crate::error::ErrorBody),
        (status = 500, description = "内部エラー（伝票 JSON 復元失敗を含む）", body = crate::error::ErrorBody),
    ),
    tag = "live",
)]
pub async fn get_live<R, P, F>(
    interactor: web::Data<Interactor<R, P, F>>,
    path: web::Path<String>,
) -> Result<HttpResponse>
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
{
    let date = parse_date(&path.into_inner())?;
    let view = interactor.find_live_by_date(date).await?;
    // 結果確定フラグ（#381）: 「⚫終」を post_time 推定でなく着順確定で判定するため race_id で引けるようにする。
    let confirmed: std::collections::HashMap<String, bool> = interactor
        .result_confirmed_by_date(date)
        .await?
        .into_iter()
        .map(|(k, v)| (k.value().to_string(), v))
        .collect();
    // 鮮度較正用のサーバ現在時刻（#382）。captured_at と同じ UTC rfc3339（秒精度）で載せる。
    let server_now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    // slip 伝票 JSON の復元失敗は永続化側の不整合。詳細はログにのみ出し 500 に倒す。
    let body = LiveResponse::from_view(view, server_now, &confirmed)
        .map_err(|e| Error::Internal(format!("slip 伝票の復元に失敗しました: {e}")))?;
    Ok(HttpResponse::Ok().json(body))
}
