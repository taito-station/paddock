/// netkeiba からの近走取り込み結果サマリ。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FetchHorseHistoryResponse {
    /// 近走を取得できた馬の数。
    pub horses_fetched: usize,
    /// 取得に失敗してスキップした馬の数。
    pub horses_failed: usize,
    /// upsert した（合成）レース数。
    pub races_saved: usize,
    /// upsert した近走（馬×レース）の行数。
    pub results_saved: usize,
}
