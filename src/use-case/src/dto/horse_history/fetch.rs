/// netkeiba からの近走取り込み結果サマリ。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FetchHorseHistoryResponse {
    /// 近走を取得できた馬の数。
    pub horses_fetched: usize,
    /// `horses_fetched` のうち、JRA 平地の近走が 0 走だった馬の数（未出走など。失敗ではない・#725）。
    pub horses_no_runs: usize,
    /// 取得に失敗してスキップした馬の数。
    pub horses_failed: usize,
    /// 出馬表取得に失敗してスキップした race_id 数（その出走馬は丸ごと欠落する）。
    pub shutuba_failed: usize,
    /// upsert した近走（馬×過去レース）の行数。
    pub runs_saved: usize,
}
