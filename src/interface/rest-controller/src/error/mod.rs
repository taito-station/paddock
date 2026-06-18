use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;

use paddock_use_case::Error as UseCaseError;

/// HTTP 層のエラー。use-case / domain のエラーを HTTP ステータスへ写像する。
#[derive(Debug)]
pub enum Error {
    /// 400: 不正な入力（クエリ・パス・ドメイン値変換失敗）。
    BadRequest(String),
    /// 404: リソースが存在しない。
    NotFound(String),
    /// 409: 既存リソースの再作成（セッション二重作成など）。
    Conflict(String),
    /// 500: 内部エラー（DB 等）。
    Internal(String),
}

impl Error {
    fn code(&self) -> &'static str {
        match self {
            Error::BadRequest(_) => "bad_request",
            Error::NotFound(_) => "not_found",
            Error::Conflict(_) => "conflict",
            Error::Internal(_) => "internal",
        }
    }

    fn message(&self) -> &str {
        match self {
            Error::BadRequest(m) | Error::NotFound(m) | Error::Conflict(m) | Error::Internal(m) => {
                m
            }
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for Error {}

/// エラーレスポンス本文。`{ "error": { "code": ..., "message": ... } }`。
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorDetail {
    /// 機械可読なエラーコード（`bad_request` / `not_found` / `conflict` / `internal`）。
    pub code: String,
    /// 人間可読なエラーメッセージ。
    pub message: String,
}

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        match self {
            Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        // 500（内部エラー）は DB エラー文字列などの内部情報を含みうる（rdb-gateway は
        // `sqlx::Error` を `use_case::Error::Internal(詳細)` に畳む）。詳細はサーバログにのみ
        // 出し、クライアントには固定文言を返して情報漏洩を防ぐ。
        let message = match self {
            Error::Internal(detail) => {
                tracing::error!(error = %detail, "internal error");
                "internal server error".to_string()
            }
            Error::BadRequest(m) | Error::NotFound(m) | Error::Conflict(m) => m.clone(),
        };
        HttpResponse::build(self.status_code()).json(ErrorBody {
            error: ErrorDetail {
                code: self.code().to_string(),
                message,
            },
        })
    }
}

impl From<UseCaseError> for Error {
    fn from(e: UseCaseError) -> Self {
        match e {
            UseCaseError::InvalidArgument(m) => Error::BadRequest(m),
            UseCaseError::NotFound(m) => Error::NotFound(m),
            UseCaseError::Conflict(m) => Error::Conflict(m),
            UseCaseError::Internal(m) => Error::Internal(m),
            // read API では外部 fetch を行わないため通常は発生しないが、網羅のため 500 に倒す
            // （詳細は error_response 側でログにのみ出す）。
            UseCaseError::Fetch(m) | UseCaseError::Timeout(m) => Error::Internal(m),
        }
    }
}

impl From<paddock_domain::Error> for Error {
    fn from(e: paddock_domain::Error) -> Self {
        Error::BadRequest(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
