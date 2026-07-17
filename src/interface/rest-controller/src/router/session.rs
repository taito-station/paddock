use actix_web::web;

use paddock_use_case::OddsScraper;
use paddock_use_case::pdf_fetcher::PdfFetcher;
use paddock_use_case::pdf_parser::PdfParser;
use paddock_use_case::repository::Repository;
use paddock_use_case::result_page_fetcher::ResultPageFetcher;

use crate::handler;

/// `/sessions` 配下の write 系ルートを登録する（呼び出し側が `/api` スコープにマウントする）。
///
/// ジェネリクスは多いが役割が分かれる:
/// - `R/P/F`: メイン `Interactor`（作成・サマリ・outcome・odds:refresh のセッション存在チェック）
/// - `O`: `OddsInteractor<O, R>`（odds:refresh のライブ取得）
/// - `S`: `ResultsInteractor<S, R>`（results:refresh の同日取り込み＋自動精算。#381 でエイリアス委譲）
pub fn configure<R, P, F, O, S>(cfg: &mut web::ServiceConfig)
where
    R: Repository + 'static,
    P: PdfParser + Send + Sync + 'static,
    F: PdfFetcher + Send + Sync + 'static,
    O: OddsScraper + Send + Sync + 'static,
    S: ResultPageFetcher + Send + Sync + 'static,
{
    cfg.service(
        web::scope("/sessions")
            .route(
                "/{date}",
                web::post().to(handler::session::create_session::<R, P, F>),
            )
            .route(
                "/{date}",
                web::get().to(handler::session::get_session_summary::<R, P, F>),
            )
            .route(
                "/{date}/races/{race_id}/outcome",
                web::post().to(handler::session::record_outcome::<R, P, F>),
            )
            .route(
                "/{date}/races/{race_id}/odds:refresh",
                web::post().to(handler::session::odds_refresh::<R, P, F, O>),
            )
            .route(
                "/{date}/results:refresh",
                web::post().to(handler::results::session_alias_refresh::<S, R>),
            ),
    );
}
