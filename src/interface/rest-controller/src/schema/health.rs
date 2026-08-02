use serde::Serialize;
use utoipa::ToSchema;

/// `GET /api/health` のレスポンス（#570）。稼働中プロセスの世代を自己申告する。
/// `git_sha` を現在の checkout（`git rev-parse --short HEAD`）と突き合わせれば陳腐化を機械検知できる。
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    /// 常に `"ok"`（プロセスが応答できている＝liveness）。
    pub status: String,
    /// ビルド元の git sha（短縮）。未コミット変更ありのビルドは `-dirty` 付き。`.git` 不在時は `unknown`。
    pub git_sha: String,
    /// ビルド時刻（UTC rfc3339, 秒精度）。
    pub build_time: String,
}
