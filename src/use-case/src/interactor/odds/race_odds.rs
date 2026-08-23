use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use paddock_domain::{BetType, RaceId, RaceOdds};

use crate::error::Result;
use crate::interactor::odds::OddsInteractor;
use crate::odds_scraper::{OddsScraper, ScrapedOdds};
use crate::repository::{OddsRepository, OddsRow, RaceOddsRecord, UnpricedObservation};

/// 「未発売と確認できた」観測を信用する期間（#632）。
///
/// これを過ぎた観測は無視して取り直す＝**発売開始に気づくまでの最大遅れ**でもある。15 分は
/// オッズ時系列コレクタ（`paddock-odds-collect`）の収集間隔と同じ刻み。前日プリフェッチ帯で
/// 再スクレイプが 1 レースあたり最大 4 回/時に収まり、かつ当日の発売開始には十分速く追従する。
///
/// 発走直前の鮮度をこの TTL が損なうことはない。`predict-watch` は read-through を通らず
/// 毎回再スクレイプする（#257）ため、判断に使うオッズは常にフレッシュ。
const UNPRICED_OBSERVATION_TTL: Duration = Duration::minutes(15);

/// 欠落券種が「未発売と確認できた（かつ観測がまだ新しい）」券種にすべて収まるかを判定する
/// 純関数（#632）。true なら再スクレイプせず保存済みを使ってよい。
///
/// **観測が無い欠落は cache-miss のまま**にするのが要点で、exotic の一過性取得失敗は
/// 従来どおり次回すぐ取り直される（#294 の自己修復を鈍らせない）。
///
/// 安全側の絞り込みが 2 つある。どちらも「判断に迷ったら取り直す」向きに倒してある:
/// - **単勝の観測は無視する**。本番の書き込み経路は組合せ 5 券種しか記録しないが、
///   仮に `win` の観測行が入ると単勝の欠落を免除してしまい、`race_odds()` が win 空の
///   スナップショットを cache-hit で返す（fetch-card が degraded 分岐で明示的に避けている
///   「オッズ有り・win 無し」の再現）。DB の CHECK は語彙統一のため 7 値を許しているので、
///   ここで読み側の防御を効かせる。**複勝は除外しない**——`missing_bet_types()` が `Place` を
///   返さないため（`is_complete()` と同じ券種集合）、`fresh` 側に混ざっても
///   `missing.is_subset(&fresh)` の結果を変えず、除外しても実効が無いから。
/// - **未来時刻の観測は stale 扱いにする**。時計のズレやダンプ復元で `observed_at` が未来に
///   なると、単純な差分比較では無条件に fresh と判定されて再取得が止まる。gateway 側が
///   「壊れた `observed_at` は読み飛ばす＝取り直すほうが安全」としているのと向きを揃える。
fn is_cache_fresh(
    missing: &BTreeSet<BetType>,
    unpriced: &[UnpricedObservation],
    now: DateTime<Utc>,
) -> bool {
    if missing.is_empty() {
        return true;
    }
    let fresh: BTreeSet<BetType> = unpriced
        .iter()
        .filter(|o| o.bet_type != BetType::Win)
        .filter(|o| {
            let age = now.signed_duration_since(o.observed_at);
            age >= Duration::zero() && age < UNPRICED_OBSERVATION_TTL
        })
        .map(|o| o.bet_type)
        .collect();
    missing.is_subset(&fresh)
}

impl<O: OddsScraper, R: OddsRepository> OddsInteractor<O, R> {
    /// race_id のオッズを read-through で取得する（#51, ADR 0010 / #294）。
    ///
    /// 1. `race_odds` に保存済みが complete（win + 組合せ 5 券種）なら、再スクレイプせずそれを返す。
    ///    **欠けている券種が TTL 15 分以内の「未発売と確認できた」観測にすべて収まる場合も
    ///    cache-hit にする**（#632。券種まるごと未発売の時間帯で毎回フルスクレイプが走るのを止める）。
    /// 2. どちらでもなければライブスクレイプし、取得した全券種(#38)を保存してフルのオッズを返す。
    ///    保存はその回の買い目には影響させない（exotic も含めて返す）。
    ///    スクレイプできた回は未発売の観測も更新する（#632）。
    ///
    /// 取得できれば `Some(odds)`、未取得なら `None`。「未取得」は次の 2 つを束ねる:
    /// - スクレイプ失敗（サイト改変・開催日外・ネットワーク等）→ warn ログを出して `None`
    /// - 取得成功だが全馬券種が空（オッズ未公開）→ `None`
    ///
    /// いずれも予想フロー側ではスキップ扱いになり、1 レースの取得失敗でセッション全体を
    /// 止めない（`select_bets` を呼ばず安全に次レースへ進める設計、predict-session.md 参照）。
    pub async fn race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        // 1. 保存済みが complete なら再スクレイプせずに返す。
        //    cache-hit 判定は「win + 組合せ 5 券種が揃った complete スナップショット」(#294)。
        //    win あり・組合せ券種一部欠落の部分スナップショット（exotic の一過性取得失敗で生じる）は
        //    `!is_empty()` を満たすため旧判定では cache-hit して当日ずっと欠落が恒久化していた。
        //    `is_complete()` 基準にすると不完全なスナップショットは cache-miss として再スクレイプする。
        //    `race_odds` は (race_id,bet_type,combination_key) 単一行 UPSERT（save_race_odds）で、
        //    再スクレイプは欠けていた券種の行を追加するだけで既存行を消さないため、保存済みの券種集合は
        //    取得済み券種の和集合として単調に埋まり、complete に収束する（persist 側は変更不要・自己修復）。
        //    place は判定に含めない（ADR 0010 の複勝未公開時 win-only 許容を維持、is_complete 参照）。
        //    ただし is_complete だけを見ると、**券種がまるごと未発売の時間帯（前日プリフェッチ）は
        //    永久に false** になり、read-through を呼ぶ度に 6 GET のフルスクレイプが走る（#632）。
        //    netkeiba 経路には RateGate が無く（ADR 0049）、規律は「無駄打ちを構造的に止める」型
        //    （ADR 0068 の debounce）なので、欠落のうち **未発売と確認できた券種**（#621/ADR 0086 の
        //    番兵、または取得成功で 0 行）を差し引いて判定する。観測の無い欠落＝一過性の取得失敗は
        //    従来どおり cache-miss にして即取り直す（#294 の自己修復を保つ）。
        if let Some(saved) = self.repository.find_race_odds(race_id, None).await? {
            // 欠落集合は 1 回だけ計算して使い回す。欠落が無ければ従来どおりの complete
            // cache-hit なので、**観測表を引かずに返す**（開催日の大半のレースがこの経路を
            // 通るため、無駄な SELECT を 1 本増やさない）。
            let missing = saved.missing_bet_types();
            let unpriced = if missing.is_empty() {
                Vec::new()
            } else {
                self.repository.find_unpriced_bet_types(race_id).await?
            };
            if is_cache_fresh(&missing, &unpriced, Utc::now()) {
                // 欠落が空なら従来どおりの complete cache-hit、非空なら「欠落はすべて未発売と
                // 確認済み」で通した場合。どちらだったかがログから読めるようにしておく。
                tracing::debug!(
                    race_id = %race_id,
                    missing = ?missing,
                    "保存済み race_odds を参照（再スクレイプなし）"
                );
                return Ok(Some(saved));
            }
        }

        // 2. cache-fresh でなければ（未保存 / 部分スナップショット / 観測が無いか古い）ライブスクレイプ。
        //    部分スナップショットの取り直しが #294 の中核ケース。空/失敗は従来どおりスキップ(None)。
        //    scrape は async（#458）。api-server 経路では実装が spawn_blocking へ逃がすため
        //    ここで await しても actix worker を同期ブロッキングで塞がない。
        match self.scraper.scrape(race_id).await {
            Ok(scraped) if scraped.odds.is_empty() => {
                // 取得は成功したが全馬券種が空（未公開）。スクレイプ失敗（warn）と
                // 区別できるよう debug で記録し、運用時に原因を切り分けられるようにする。
                // 未発売の観測も記録しない——win すら無い状態は「まだ何も公開されていない」で
                // あって、券種ごとの発売有無を確認できたわけではない。
                tracing::debug!(race_id = %race_id, "オッズ取得成功だが全馬券種が空（未公開）、スキップ");
                Ok(None)
            }
            Ok(scraped) => {
                // 取得できた全券種を永続化（#38）。保存失敗は予想を止めず warn のみ。
                let saved = self.persist_all(race_id, &scraped.odds).await;
                self.record_unpriced(race_id, &scraped, saved).await;
                // フルのオッズはその回の買い目にそのまま使う。
                Ok(Some(scraped.odds))
            }
            Err(e) => {
                tracing::warn!(race_id = %race_id, error = %e, "オッズ取得に失敗、スキップ");
                Ok(None)
            }
        }
    }

    /// race_id のオッズを**キャッシュのみ**で返す（再スクレイプしない）。
    ///
    /// `race_odds()` と異なり completeness チェックを行わないため、保存済みが一部券種のみの
    /// 部分スナップショットでもそのまま返す。過去日（`MeetingPhase::Over`）の --overview で
    /// read-through を抑制するために使う（#624）。過去日に切り替わった時点で自己修復パス
    /// （read-through による再スクレイプ）は使われなくなるため、一過性の取得失敗で保存された
    /// 不完全なスナップショットはそのまま固定表示される——この片道性は意図的な割り切り。
    pub async fn race_odds_cached(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        self.repository.find_race_odds(race_id, None).await
    }

    /// race_id のオッズを**必ず再スクレイプ**して新スナップショットを保存し、フルのオッズを返す（#257）。
    ///
    /// `race_odds()` の read-through はキャッシュ優先で再取得しないため、発走直前の
    /// フレッシュなオッズで EV/ROI を再計算したい監視用途には使えない。本メソッドは
    /// 保存済みの有無に関わらず常にライブスクレイプし、`persist_all` で新スナップショットを
    /// 追記する（`find_race_odds(.., None)` が最新を返すため、後続の予想はフレッシュ値を見る）。
    ///
    /// 戻り値の意味は `race_odds()` と揃える: 取得できれば `Some(odds)`、未取得（スクレイプ
    /// 失敗・全券種空＝未公開）は `None`。監視 1 レースの取得失敗で全体を止めない。
    pub async fn refresh_race_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        match self.scraper.scrape(race_id).await {
            Ok(scraped) if scraped.odds.is_empty() => {
                tracing::debug!(race_id = %race_id, "オッズ再取得成功だが全馬券種が空（未公開）、スキップ");
                Ok(None)
            }
            Ok(scraped) => {
                // 監視経路は cache を見ないが、観測は同じように記録する（#632）。発売開始を
                // 最初に観測するのは 5 分毎に回る predict-watch であることが多く、ここで
                // マークを消しておくと read-through 側も次回すぐ取り直せる。
                let saved = self.persist_all(race_id, &scraped.odds).await;
                self.record_unpriced(race_id, &scraped, saved).await;
                Ok(Some(scraped.odds))
            }
            Err(e) => {
                tracing::warn!(race_id = %race_id, error = %e, "オッズ再取得に失敗、スキップ");
                Ok(None)
            }
        }
    }

    /// race_id の**単複のみ**を必ず再スクレイプして新スナップショットを保存する（#odds-collect）。
    ///
    /// オッズ時系列コレクタ用の軽量版 `refresh_race_odds`。組合せ 5 券種を打たず win/place
    /// （type=1・1 GET）だけを取り、`persist_all` で `race_odds`（最新・key 単位 UPSERT なので
    /// exotic 行は破壊しない）＋ `race_odds_snapshots`（append）へ保存する。全レースを終日高頻度で
    /// 貯めるため netkeiba への負荷を最小化する。戻り値の意味は `refresh_race_odds` と揃える
    /// （取得できれば `Some`、未公開/失敗は `None`・1 レースの失敗で収集ループを止めない）。
    pub async fn refresh_win_place_odds(&self, race_id: &RaceId) -> Result<Option<RaceOdds>> {
        match self.scraper.scrape_win_place(race_id).await {
            Ok(odds) if odds.is_empty() => {
                tracing::debug!(race_id = %race_id, "単複再取得成功だが空（未公開）、スキップ");
                Ok(None)
            }
            Ok(odds) => {
                // 単複のみの経路は組合せ券種を観測しないので、保存の成否で分岐する必要は無い。
                let _ = self.persist_all(race_id, &odds).await;
                Ok(Some(odds))
            }
            Err(e) => {
                tracing::warn!(race_id = %race_id, error = %e, "単複再取得に失敗、スキップ");
                Ok(None)
            }
        }
    }

    /// スクレイプで得た全券種のオッズを `race_odds` に保存する。複勝・ワイドは幅 odds
    /// （下限=odds, 上限=odds_high）。スクレイプ由来は人気を持たないため popularity は None。
    /// 保存失敗は予想フローを止めず warn ログのみ（次回参照時に取り直せる）。
    ///
    /// **保存できたかどうかを返す**（#632）。失敗した回に未発売の観測だけを記録すると、
    /// 「古いスナップショット ＋ 新しいマーク」で TTL のあいだ古い値を cache-hit で返し続ける
    /// ——本 PR 自身の「迷ったら取り直す」原則に反するので、呼び出し側で観測記録を見送る。
    /// 保存する行が 1 つも無い場合も `false`（保存していないので観測も残さない）。
    async fn persist_all(&self, race_id: &RaceId, odds: &RaceOdds) -> bool {
        let capacity = odds.win.len()
            + odds.place.len()
            + odds.quinella.len()
            + odds.wide.len()
            + odds.exacta.len()
            + odds.trio.len()
            + odds.trifecta.len();
        let mut rows: Vec<OddsRow> = Vec::with_capacity(capacity);
        for (horse, ov) in &odds.win {
            rows.push(OddsRow::win(horse.value(), ov.value(), None));
        }
        for (horse, place) in &odds.place {
            rows.push(OddsRow::place(
                horse.value(),
                place.low.value(),
                place.high.value(),
                None,
            ));
        }
        for (pair, ov) in &odds.quinella {
            rows.push(OddsRow::quinella(*pair, ov.value()));
        }
        for (pair, band) in &odds.wide {
            rows.push(OddsRow::wide(*pair, band.low.value(), band.high.value()));
        }
        for (pair, ov) in &odds.exacta {
            rows.push(OddsRow::exacta(*pair, ov.value()));
        }
        for (triple, ov) in &odds.trio {
            rows.push(OddsRow::trio(*triple, ov.value()));
        }
        for (triple, ov) in &odds.trifecta {
            rows.push(OddsRow::trifecta(*triple, ov.value()));
        }
        if rows.is_empty() {
            return false;
        }
        let record = RaceOddsRecord {
            race_id: race_id.clone(),
            fetched_at: Utc::now(),
            rows,
        };
        if let Err(e) = self.repository.save_race_odds(&record).await {
            tracing::warn!(race_id = %race_id, error = %e, "race_odds の保存に失敗（予想は継続）");
            return false;
        }
        true
    }

    /// 1 回のスクレイプ観測から「未発売と確認できた券種」を記録する（#632）。
    ///
    /// priced が取れた券種は同時にマークを消す（発売開始の反映）。`persist_all` と同じく
    /// **保存失敗は予想フローを止めず warn のみ**——マークが無い状態は「毎回取り直す」という
    /// 修正前の挙動に戻るだけで、判断を誤らせる方向には倒れない。
    ///
    /// `odds_saved` はオッズの保存に成功したか。**false のときは未発売マークを新しく立てない**
    /// ——古いスナップショットに新しいマークが付くと TTL のあいだ古い値を cache-hit で返し続ける。
    /// 一方 **priced 券種のマーク削除は保存の成否によらず行う**: 削除は「次回取り直す」方向に
    /// しか働かないので、見送ると発売開始を検知できないまま古い判断が TTL 分残る（どちらの
    /// 分岐も「迷ったら取り直す」に倒す）。
    async fn record_unpriced(&self, race_id: &RaceId, scraped: &ScrapedOdds, odds_saved: bool) {
        // priced 側は「今回オッズが取れた券種」。unpriced と重ならないことは assemble 側の
        // 判定（priced 0 件のときだけ unpriced）が保証する。
        let priced: BTreeSet<BetType> = [
            (BetType::Quinella, !scraped.odds.quinella.is_empty()),
            (BetType::Wide, !scraped.odds.wide.is_empty()),
            (BetType::Exacta, !scraped.odds.exacta.is_empty()),
            (BetType::Trio, !scraped.odds.trio.is_empty()),
            (BetType::Trifecta, !scraped.odds.trifecta.is_empty()),
        ]
        .into_iter()
        .filter_map(|(bet_type, has_rows)| has_rows.then_some(bet_type))
        .collect();
        // 保存できなかった回は新しいマークを立てない（削除だけ通す）。
        let unpriced = if odds_saved {
            scraped.unpriced.clone()
        } else {
            BTreeSet::new()
        };
        if unpriced.is_empty() && priced.is_empty() {
            return;
        }
        if let Err(e) = self
            .repository
            .record_unpriced_bet_types(race_id, &unpriced, &priced, Utc::now())
            .await
        {
            tracing::warn!(
                race_id = %race_id,
                error = %e,
                "未発売券種の観測記録に失敗（予想は継続・次回は再スクレイプになる）"
            );
            return;
        }
        // 何を未発売と判断したかを残す。判定が「priced が 0 件か」である以上、netkeiba の
        // JSON 形式変更でパースが全滅しても「未発売」に見える——その場合ここに毎回全券種が
        // 並ぶので、真の未発売（発売開始後に消える）と切り分けられる。ただし既定のログ
        // フィルタは info なので、切り分けには `PADDOCK_LOG=debug` が要る。
        if !unpriced.is_empty() {
            tracing::debug!(
                race_id = %race_id,
                unpriced = ?unpriced,
                priced = ?priced,
                "未発売と確認できた券種を記録（TTL 内は再スクレイプしない）"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::NaiveDate;
    use paddock_domain::{
        BetType, HorseNum, OddsValue, OrderedPair, OrderedTriple, Pair, PlaceOdds, RaceId,
        RaceOdds, Triple,
    };

    use super::{BTreeSet, DateTime, Duration, UNPRICED_OBSERVATION_TTL, Utc, is_cache_fresh};

    use crate::error::{Error, Result};
    use crate::interactor::odds::OddsInteractor;
    use crate::odds_scraper::{OddsScraper, ScrapedOdds};
    use crate::repository::{OddsRepository, RaceOddsRecord, UnpricedObservation};

    /// テスト用の OddsScraper。scrape の戻り値を差し替えつつ呼び出し回数を数える。
    struct FakeScraper {
        result: fn(&RaceId) -> Result<RaceOdds>,
        /// この観測で「未発売と確認できた」券種（#632）。
        unpriced: BTreeSet<BetType>,
        calls: Mutex<usize>,
    }

    impl FakeScraper {
        fn new(result: fn(&RaceId) -> Result<RaceOdds>) -> Self {
            Self {
                result,
                unpriced: BTreeSet::new(),
                calls: Mutex::new(0),
            }
        }

        /// 未発売と確認できた券種を伴う観測を返すスクレイパ。
        fn with_unpriced(result: fn(&RaceId) -> Result<RaceOdds>, unpriced: &[BetType]) -> Self {
            Self {
                unpriced: unpriced.iter().copied().collect(),
                ..Self::new(result)
            }
        }
    }

    impl OddsScraper for FakeScraper {
        async fn scrape(&self, race_id: &RaceId) -> Result<ScrapedOdds> {
            *self.calls.lock().unwrap() += 1;
            Ok(ScrapedOdds {
                odds: (self.result)(race_id)?,
                unpriced: self.unpriced.clone(),
            })
        }
    }

    /// 保存済みオッズの有無と save 呼び出しを記録するだけの Repository フェイク。
    ///
    /// 未発売観測（#632）は `unpriced` が実 DB と同じ意味論（`(race_id, bet_type)` の UPSERT ＋
    /// priced 券種の DELETE）で保持する。read-through を複数回呼んだときの挙動を見るテストが
    /// あるため、記録は呼び出しをまたいで残る。
    #[derive(Default)]
    struct FakeRepo {
        preset: Option<RaceOdds>,
        saved: Mutex<Vec<RaceOddsRecord>>,
        unpriced: Mutex<Vec<UnpricedObservation>>,
        /// true なら `save_race_odds` が失敗する（保存失敗時に観測を残さないことの検証用）。
        save_fails: bool,
    }

    impl FakeRepo {
        /// 保存済みオッズと、`age` だけ過去に観測された未発売マークを持つ Repo。
        fn with_marks(preset: RaceOdds, marks: &[BetType], age: Duration) -> Self {
            let observed_at = Utc::now() - age;
            Self {
                preset: Some(preset),
                unpriced: Mutex::new(
                    marks
                        .iter()
                        .map(|&bet_type| UnpricedObservation {
                            bet_type,
                            observed_at,
                        })
                        .collect(),
                ),
                ..Default::default()
            }
        }
    }

    impl OddsRepository for FakeRepo {
        async fn find_race_odds(
            &self,
            _race_id: &RaceId,
            _as_of: Option<NaiveDate>,
        ) -> Result<Option<RaceOdds>> {
            Ok(self.preset.clone())
        }
        async fn save_race_odds(&self, record: &RaceOddsRecord) -> Result<()> {
            if self.save_fails {
                return Err(Error::Internal("save failed".into()));
            }
            self.saved.lock().unwrap().push(record.clone());
            Ok(())
        }
        async fn find_race_odds_morning(
            &self,
            _race_id: &RaceId,
        ) -> Result<Option<crate::repository::MorningRaceOdds>> {
            Ok(None)
        }
        async fn purge_race_odds_snapshots(&self, _before: NaiveDate) -> Result<u64> {
            Ok(0)
        }
        async fn count_race_odds_snapshots_before(&self, _before: NaiveDate) -> Result<u64> {
            Ok(0)
        }
        async fn find_unpriced_bet_types(
            &self,
            _race_id: &RaceId,
        ) -> Result<Vec<UnpricedObservation>> {
            Ok(self.unpriced.lock().unwrap().clone())
        }
        async fn record_unpriced_bet_types(
            &self,
            _race_id: &RaceId,
            unpriced: &BTreeSet<BetType>,
            priced: &BTreeSet<BetType>,
            observed_at: DateTime<Utc>,
        ) -> Result<()> {
            let mut marks = self.unpriced.lock().unwrap();
            marks.retain(|m| !priced.contains(&m.bet_type));
            for &bet_type in unpriced {
                match marks.iter_mut().find(|m| m.bet_type == bet_type) {
                    Some(existing) => existing.observed_at = observed_at,
                    None => marks.push(UnpricedObservation {
                        bet_type,
                        observed_at,
                    }),
                }
            }
            Ok(())
        }
    }

    fn race_id() -> RaceId {
        RaceId::try_from("2026-3-nakayama-8-1R").unwrap()
    }

    fn odds_with_win(race_id: RaceId) -> RaceOdds {
        let mut odds = RaceOdds::empty(race_id);
        odds.win.insert(
            HorseNum::try_from(1).unwrap(),
            OddsValue::try_from((BetType::Win, 3.5)).unwrap(),
        );
        odds
    }

    fn odds_win_place(race_id: RaceId) -> RaceOdds {
        let mut odds = odds_with_win(race_id);
        odds.place.insert(
            HorseNum::try_from(1).unwrap(),
            PlaceOdds::try_from((
                OddsValue::try_from((BetType::Place, 1.5)).unwrap(),
                OddsValue::try_from((BetType::Place, 2.0)).unwrap(),
            ))
            .unwrap(),
        );
        odds
    }

    /// 前日プリフェッチ帯の形: 単勝と一部の組合せ券種は取れるが、三連複・三連単はまるごと未発売。
    fn odds_without_trio_trifecta(race_id: RaceId) -> RaceOdds {
        let mut odds = odds_all_types(race_id);
        odds.trio.clear();
        odds.trifecta.clear();
        odds
    }

    // --- #632: 券種まるごと未発売の再スクレイプ抑止 ---------------------------

    #[tokio::test]
    async fn unpriced_bet_types_are_not_rescraped_within_ttl() {
        // **本 issue の回帰テスト（再取得回数の計測）**。三連複・三連単がまるごと未発売の
        // レースで read-through を 5 回呼ぶ。修正前は is_complete が永久 false なので
        // スクレイプが 5 回（＝呼んだ回数だけ 6 GET が飛ぶ）走っていた。
        // 修正後は初回だけ取りに行き、以降は未発売の観測で cache-hit する。
        let scraper = FakeScraper::with_unpriced(
            |rid| Ok(odds_without_trio_trifecta(rid.clone())),
            &[BetType::Trio, BetType::Trifecta],
        );
        let repo = FakeRepo {
            preset: Some(odds_without_trio_trifecta(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        for _ in 0..5 {
            let got = interactor.race_odds(&race_id()).await.unwrap();
            assert!(got.is_some(), "未発売の券種があってもオッズは返る");
        }

        assert_eq!(
            *interactor.scraper.calls.lock().unwrap(),
            1,
            "未発売と確認できた券種は TTL 内で取り直さない（修正前は 5）"
        );
        assert_eq!(
            interactor
                .repository
                .unpriced
                .lock()
                .unwrap()
                .iter()
                .map(|m| m.bet_type)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([BetType::Trio, BetType::Trifecta]),
            "未発売の観測が記録される"
        );
    }

    #[tokio::test]
    async fn rescrapes_once_unpriced_mark_is_stale() {
        // TTL を過ぎた観測は信用しない＝発売開始に気づける。これが無いと「未発売」の判断が
        // 永久に固定され、当日オッズが出ても取りに行かなくなる。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let repo = FakeRepo::with_marks(
            odds_without_trio_trifecta(race_id()),
            &[BetType::Trio, BetType::Trifecta],
            UNPRICED_OBSERVATION_TTL + Duration::minutes(1),
        );
        let interactor = OddsInteractor::new(scraper, repo);

        interactor.race_odds(&race_id()).await.unwrap();

        assert_eq!(
            *interactor.scraper.calls.lock().unwrap(),
            1,
            "TTL 切れの観測は cache-miss として取り直す"
        );
        assert!(
            interactor.repository.unpriced.lock().unwrap().is_empty(),
            "発売開始（priced が取れた）ら未発売マークは消える"
        );
    }

    #[tokio::test]
    async fn missing_without_mark_still_rescrapes_every_call() {
        // **#294 の自己修復が鈍っていないことの固定**。未発売の観測が無い欠落（＝exotic の
        // 一過性取得失敗）は、修正前と同じく毎回 cache-miss として取り直す。ここを
        // 「レース単位の一律 debounce」にすると、この性質が失われる。
        let scraper = FakeScraper::new(|rid| Ok(odds_without_trio_trifecta(rid.clone())));
        let repo = FakeRepo {
            preset: Some(odds_without_trio_trifecta(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        for _ in 0..3 {
            interactor.race_odds(&race_id()).await.unwrap();
        }

        assert_eq!(
            *interactor.scraper.calls.lock().unwrap(),
            3,
            "観測の無い欠落は毎回取り直す（#294）"
        );
    }

    #[tokio::test]
    async fn all_empty_scrape_records_no_observation() {
        // ADR 0089「影響」の残存ケース。単勝すら未公開のレースは「まだ何も公開されていない」で
        // あって券種ごとの発売有無を確認できたわけではないので、観測を残してはいけない。
        // ここで捨てないと、全 5 券種に未発売マークが立つ＝決定 4 が「最も危険」と名指しした解釈。
        // 分岐を通るテストが無いと、将来 record_unpriced をこの分岐の前へ動かす整理で静かに壊れる。
        let scraper = FakeScraper::with_unpriced(
            |rid| Ok(RaceOdds::empty(rid.clone())),
            &[
                BetType::Quinella,
                BetType::Wide,
                BetType::Exacta,
                BetType::Trio,
                BetType::Trifecta,
            ],
        );
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        assert!(interactor.race_odds(&race_id()).await.unwrap().is_none());
        assert!(
            interactor.repository.unpriced.lock().unwrap().is_empty(),
            "全券種が空のスクレイプでは観測を残さない"
        );

        assert!(
            interactor
                .refresh_race_odds(&race_id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            interactor.repository.unpriced.lock().unwrap().is_empty(),
            "監視経路でも同じ"
        );
    }

    #[tokio::test]
    async fn failed_persist_skips_observation_recording() {
        // 保存に失敗した回にマークだけ立てると、「古いスナップショット ＋ 新しいマーク」で
        // TTL のあいだ古い値を cache-hit で返し続ける。「迷ったら取り直す」に倒す。
        let scraper = FakeScraper::with_unpriced(
            |rid| Ok(odds_without_trio_trifecta(rid.clone())),
            &[BetType::Trio, BetType::Trifecta],
        );
        let repo = FakeRepo {
            save_fails: true,
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        interactor.race_odds(&race_id()).await.unwrap();

        assert!(
            interactor.repository.unpriced.lock().unwrap().is_empty(),
            "保存に失敗した回は未発売マークを新しく立てない"
        );
    }

    #[tokio::test]
    async fn failed_persist_still_clears_marks_for_priced_bet_types() {
        // マーク削除は「次回取り直す」方向にしか働かないので、保存の成否によらず通す。
        // 見送ると、発売開始したのに古いマークが残って TTL のあいだ古い判断を引きずる。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let repo = FakeRepo {
            save_fails: true,
            // マークが fresh だと cache-hit してスクレイプに到達しないので、TTL 切れにして
            // 「取り直したら発売開始していた」状況を作る。
            ..FakeRepo::with_marks(
                odds_without_trio_trifecta(race_id()),
                &[BetType::Trio, BetType::Trifecta],
                UNPRICED_OBSERVATION_TTL + Duration::minutes(1),
            )
        };
        let interactor = OddsInteractor::new(scraper, repo);

        interactor.race_odds(&race_id()).await.unwrap();

        assert_eq!(
            *interactor.scraper.calls.lock().unwrap(),
            1,
            "前提: TTL 切れなので取り直している"
        );
        assert!(
            interactor.repository.unpriced.lock().unwrap().is_empty(),
            "保存に失敗しても priced になった券種のマークは消す"
        );
    }

    #[tokio::test]
    async fn refresh_records_unpriced_observations() {
        // ADR 0089 決定 9: 監視経路（predict-watch）でも観測を記録する。発売開始を最初に
        // 観測するのは 5 分毎に回る監視であることが多く、ここでマークを更新しないと
        // read-through 側が TTL いっぱい古い判断を引きずる。
        let scraper = FakeScraper::with_unpriced(
            |rid| Ok(odds_without_trio_trifecta(rid.clone())),
            &[BetType::Trio, BetType::Trifecta],
        );
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        interactor.refresh_race_odds(&race_id()).await.unwrap();

        assert_eq!(
            interactor
                .repository
                .unpriced
                .lock()
                .unwrap()
                .iter()
                .map(|m| m.bet_type)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([BetType::Trio, BetType::Trifecta]),
            "監視経路でも未発売の観測が記録される"
        );
    }

    #[tokio::test]
    async fn refresh_clears_marks_when_bet_type_goes_on_sale() {
        // 監視が発売開始を観測したらマークが消える（＝read-through も次回すぐ取り直せる）。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let repo = FakeRepo::with_marks(
            odds_without_trio_trifecta(race_id()),
            &[BetType::Trio, BetType::Trifecta],
            Duration::minutes(1),
        );
        let interactor = OddsInteractor::new(scraper, repo);

        interactor.refresh_race_odds(&race_id()).await.unwrap();

        assert!(
            interactor.repository.unpriced.lock().unwrap().is_empty(),
            "priced が取れた券種のマークは監視経路でも消える"
        );
    }

    #[test]
    fn is_cache_fresh_ignores_win_observations() {
        // 単勝の観測は本番経路では書かれないが、DB の CHECK は語彙統一のため 7 値を
        // 許している。仮に win の観測行が入っても単勝の欠落を免除しないことの防御テスト
        // （免除すると win 空のスナップショットを cache-hit で返してしまう）。
        let mut saved = odds_all_types(race_id());
        saved.win.clear();
        let now = Utc::now();

        assert!(!is_cache_fresh(
            &saved.missing_bet_types(),
            &[UnpricedObservation {
                bet_type: BetType::Win,
                observed_at: now,
            }],
            now
        ));
    }

    #[test]
    fn is_cache_fresh_treats_future_observations_as_stale() {
        // 時計のズレやダンプ復元で observed_at が未来になったとき、単純な差分比較では
        // 無条件に fresh と判定されて再取得が止まる。安全側（取り直す）に倒す。
        let saved = odds_without_trio_trifecta(race_id());
        let now = Utc::now();
        let future = |bet_type| UnpricedObservation {
            bet_type,
            observed_at: now + Duration::hours(1),
        };

        assert!(!is_cache_fresh(
            &saved.missing_bet_types(),
            &[future(BetType::Trio), future(BetType::Trifecta)],
            now
        ));
    }

    #[test]
    fn is_cache_fresh_requires_every_missing_bet_type_to_be_observed() {
        // 欠落の一部しか観測が無いなら cache-miss。部分的な観測で取り直しを止めると、
        // 観測されていない券種の欠落（＝取得失敗）が恒久化する。
        let saved = odds_without_trio_trifecta(race_id());
        let now = Utc::now();
        let mark = |bet_type| UnpricedObservation {
            bet_type,
            observed_at: now,
        };

        assert!(
            !is_cache_fresh(&saved.missing_bet_types(), &[], now),
            "観測なし → 再スクレイプ"
        );
        assert!(
            !is_cache_fresh(&saved.missing_bet_types(), &[mark(BetType::Trio)], now),
            "三連単の観測が無いので再スクレイプ"
        );
        assert!(
            is_cache_fresh(
                &saved.missing_bet_types(),
                &[mark(BetType::Trio), mark(BetType::Trifecta)],
                now
            ),
            "欠落がすべて未発売と確認済みなら cache-hit"
        );
        assert!(
            is_cache_fresh(&odds_all_types(race_id()).missing_bet_types(), &[], now),
            "そもそも欠落が無ければ観測は不要"
        );
    }

    #[test]
    fn is_cache_fresh_ignores_observations_older_than_ttl() {
        let saved = odds_without_trio_trifecta(race_id());
        let now = Utc::now();
        let aged = |bet_type, age| UnpricedObservation {
            bet_type,
            observed_at: now - age,
        };
        let just_inside = UNPRICED_OBSERVATION_TTL - Duration::seconds(1);
        let just_outside = UNPRICED_OBSERVATION_TTL + Duration::seconds(1);

        assert!(is_cache_fresh(
            &saved.missing_bet_types(),
            &[
                aged(BetType::Trio, just_inside),
                aged(BetType::Trifecta, just_inside)
            ],
            now
        ));
        assert!(!is_cache_fresh(
            &saved.missing_bet_types(),
            &[
                aged(BetType::Trio, just_inside),
                aged(BetType::Trifecta, just_outside)
            ],
            now
        ));
    }

    #[tokio::test]
    async fn returns_saved_without_scraping() {
        // 保存済みが complete（win + 組合せ 5 券種）なら scrape を呼ばずにそれを返す（#294）。
        let scraper = FakeScraper::new(|_| panic!("scrape は呼ばれてはならない"));
        let repo = FakeRepo {
            preset: Some(odds_all_types(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| o.is_complete()));
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 0);
        assert!(interactor.repository.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rescrapes_when_saved_snapshot_incomplete() {
        // #294: 保存済みが部分スナップショット（win+place のみ・組合せ券種欠落）の場合は
        // cache-miss として再スクレイプし、complete を取り直して persist する。
        // exotic の一過性取得失敗で生じた欠落が当日恒久化するのを防ぐ。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let repo = FakeRepo {
            preset: Some(odds_win_place(race_id())), // 組合せ券種が無い＝is_complete()=false
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(
            got.is_some_and(|o| o.is_complete()),
            "再スクレイプで complete を返す"
        );
        assert_eq!(
            *interactor.scraper.calls.lock().unwrap(),
            1,
            "部分スナップショットは cache-miss として再スクレイプする"
        );
        assert_eq!(
            interactor.repository.saved.lock().unwrap().len(),
            1,
            "再取得した complete スナップショットを追記保存する"
        );
    }

    #[tokio::test]
    async fn scrapes_and_persists_when_not_saved() {
        // 未保存ならスクレイプし、単勝・複勝を保存してフルのオッズを返す。
        let scraper = FakeScraper::new(|rid| Ok(odds_win_place(rid.clone())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| !o.is_empty()));
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 1);

        let saved = interactor.repository.saved.lock().unwrap();
        assert_eq!(saved.len(), 1, "単勝・複勝を 1 レコードで保存");
        let rows = &saved[0].rows;
        assert_eq!(rows.iter().filter(|r| r.bet_type == "win").count(), 1);
        let place: Vec<_> = rows.iter().filter(|r| r.bet_type == "place").collect();
        assert_eq!(place.len(), 1);
        assert!((place[0].odds - 1.5).abs() < 1e-9);
        assert_eq!(place[0].odds_high, Some(2.0));
    }

    fn odds_all_types(race_id: RaceId) -> RaceOdds {
        let mut odds = odds_win_place(race_id);
        let h = |n: u32| HorseNum::try_from(n).unwrap();
        // テスト値はどの券種の番兵とも重ならない正当オッズなので、ヘルパは Win 固定で包む（#630）。
        let ov = |v: f64| OddsValue::try_from((BetType::Win, v)).unwrap();
        odds.quinella
            .insert(Pair::try_from((h(1), h(2))).unwrap(), ov(12.4));
        odds.wide.insert(
            Pair::try_from((h(1), h(2))).unwrap(),
            PlaceOdds::try_from((ov(3.1), ov(4.8))).unwrap(),
        );
        odds.exacta
            .insert(OrderedPair::try_from((h(2), h(1))).unwrap(), ov(25.0));
        odds.trio
            .insert(Triple::try_from((h(1), h(2), h(3))).unwrap(), ov(88.0));
        odds.trifecta.insert(
            OrderedTriple::try_from((h(3), h(1), h(2))).unwrap(),
            ov(410.0),
        );
        odds
    }

    #[tokio::test]
    async fn persists_all_bet_types_when_scraped() {
        // #38: スクレイプで得た組合せ券種も含め全券種を保存する。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        interactor.race_odds(&race_id()).await.unwrap();

        let saved = interactor.repository.saved.lock().unwrap();
        let rows = &saved[0].rows;
        let count = |bt: &str| rows.iter().filter(|r| r.bet_type == bt).count();
        for bt in [
            "win", "place", "quinella", "wide", "exacta", "trio", "trifecta",
        ] {
            assert_eq!(count(bt), 1, "{bt} が 1 行保存されること");
        }
        // ワイドは複勝同様に幅 odds（odds_high 付き）で保存される。
        let wide = rows.iter().find(|r| r.bet_type == "wide").unwrap();
        assert_eq!(wide.combination_key, "1-2");
        assert_eq!(wide.odds_high, Some(4.8));
        // 馬単はキーの順序を保持（2>1）。
        let exacta = rows.iter().find(|r| r.bet_type == "exacta").unwrap();
        assert_eq!(exacta.combination_key, "2>1");
    }

    #[tokio::test]
    async fn returns_none_when_odds_empty() {
        // 取得成功だが未公開（全馬券種が空）→ スキップ扱いの None。保存もしない。
        let scraper = FakeScraper::new(|rid| Ok(RaceOdds::empty(rid.clone())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_none());
        assert!(interactor.repository.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn returns_none_on_scrape_error() {
        // スクレイプ失敗はセッションを止めず None で安全にスキップ。
        let scraper = FakeScraper::new(|_| Err(Error::Internal("navigation failed".into())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.race_odds(&race_id()).await.unwrap();
        assert!(got.is_none());
        assert!(interactor.repository.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refresh_scrapes_and_persists_even_when_saved() {
        // #257: refresh は read-through と違い、保存済みがあっても必ず再スクレイプし
        // 新スナップショットを保存する（発走直前のフレッシュなオッズを得るため）。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let repo = FakeRepo {
            preset: Some(odds_with_win(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.refresh_race_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| !o.is_empty()));
        // 保存済みがあっても scrape は必ず 1 回呼ばれる（read-through との決定的な違い）。
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 1);
        // 新スナップショットが 1 レコード追記される。
        assert_eq!(interactor.repository.saved.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refresh_win_place_persists_only_win_place() {
        // #odds-collect: 単複限定 refresh は win/place だけを保存する（組合せ券種は打たない）。
        // FakeScraper は scrape() で全券種を返すが、trait 既定の scrape_win_place が win/place に絞る。
        let scraper = FakeScraper::new(|rid| Ok(odds_all_types(rid.clone())));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.refresh_win_place_odds(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| !o.is_empty()));
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 1);

        let saved = interactor.repository.saved.lock().unwrap();
        assert_eq!(saved.len(), 1, "新スナップショットを 1 レコード追記");
        let rows = &saved[0].rows;
        let count = |bt: &str| rows.iter().filter(|r| r.bet_type == bt).count();
        assert_eq!(count("win"), 1, "win を保存");
        assert_eq!(count("place"), 1, "place を保存");
        for bt in ["quinella", "wide", "exacta", "trio", "trifecta"] {
            assert_eq!(count(bt), 0, "{bt} は単複限定なので保存しない");
        }
    }

    #[tokio::test]
    async fn refresh_win_place_returns_none_when_empty_or_error() {
        // 未公開（空）も失敗も None・保存なし（refresh_race_odds と挙動を揃える）。
        let empty = OddsInteractor::new(
            FakeScraper::new(|rid| Ok(RaceOdds::empty(rid.clone()))),
            FakeRepo::default(),
        );
        assert!(
            empty
                .refresh_win_place_odds(&race_id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(empty.repository.saved.lock().unwrap().is_empty());

        let errored = OddsInteractor::new(
            FakeScraper::new(|_| Err(Error::Internal("nav failed".into()))),
            FakeRepo::default(),
        );
        assert!(
            errored
                .refresh_win_place_odds(&race_id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(errored.repository.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refresh_returns_none_when_empty_or_error() {
        // 未公開（全券種空）も失敗も None。保存はしない（race_odds() と挙動を揃える）。
        let empty = OddsInteractor::new(
            FakeScraper::new(|rid| Ok(RaceOdds::empty(rid.clone()))),
            FakeRepo::default(),
        );
        assert!(empty.refresh_race_odds(&race_id()).await.unwrap().is_none());
        assert!(empty.repository.saved.lock().unwrap().is_empty());

        let errored = OddsInteractor::new(
            FakeScraper::new(|_| Err(Error::Internal("nav failed".into()))),
            FakeRepo::default(),
        );
        assert!(
            errored
                .refresh_race_odds(&race_id())
                .await
                .unwrap()
                .is_none()
        );
        assert!(errored.repository.saved.lock().unwrap().is_empty());
    }

    // --- #624: race_odds_cached（キャッシュのみ・スクレイプなし）-----------------

    #[tokio::test]
    async fn cached_returns_preset_without_scraping() {
        let scraper = FakeScraper::new(|_| panic!("scrape は呼ばれてはならない"));
        let repo = FakeRepo {
            preset: Some(odds_all_types(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.race_odds_cached(&race_id()).await.unwrap();
        assert!(got.is_some_and(|o| o.is_complete()));
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn cached_returns_none_when_not_saved() {
        let scraper = FakeScraper::new(|_| panic!("scrape は呼ばれてはならない"));
        let interactor = OddsInteractor::new(scraper, FakeRepo::default());

        let got = interactor.race_odds_cached(&race_id()).await.unwrap();
        assert!(got.is_none());
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn cached_returns_partial_snapshot_as_is() {
        let scraper = FakeScraper::new(|_| panic!("scrape は呼ばれてはならない"));
        let repo = FakeRepo {
            preset: Some(odds_win_place(race_id())),
            ..Default::default()
        };
        let interactor = OddsInteractor::new(scraper, repo);

        let got = interactor.race_odds_cached(&race_id()).await.unwrap();
        assert!(
            got.is_some_and(|o| !o.is_complete()),
            "部分スナップショットでも再スクレイプせずそのまま返す"
        );
        assert_eq!(*interactor.scraper.calls.lock().unwrap(), 0);
    }
}
