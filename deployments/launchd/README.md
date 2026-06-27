# 締切前 live オッズ自動 prefetch（#237）

発走 N 分前のレースの最新オッズを定期取得し、`race_odds_snapshots`（#232）に締切前 live
スナップショットを蓄積する。これが回ると #218（live オッズで α 再校正）の入力が貯まる。

- 本体: [`scripts/predict-check/prefetch_odds.sh`](../../scripts/predict-check/prefetch_odds.sh)
- レース選択: [`scripts/predict-check/upcoming_races_db.py`](../../scripts/predict-check/upcoming_races_db.py)
  （#235 の `race_cards.post_time` を DB 参照。netkeiba 都度スクレイプ無し）

## 前提

- 当日の出馬表（`post_time` 入り）が朝の `paddock-fetch-card` 運用で DB に投入済みであること。
  未投入なら対象 0 件で no-op（安全）。
- release バイナリをビルド済みであること:
  `cargo build --release --bin paddock-fetch-card`

## macOS（launchd, 推奨）

```sh
# 有効化（~/Library/LaunchAgents に配置して load。__REPO_ROOT__ は実パスに置換される）
deployments/launchd/install.sh

# 状態確認 / ログ
launchctl list | grep com.paddock.prefetch-odds
tail -f "$TMPDIR/paddock-prefetch/logs/prefetch.log"

# 無効化
deployments/launchd/uninstall.sh
```

`StartInterval=300`（5 分間隔）。開催日だけ走らせたい場合は開催日朝に install、夜に uninstall
する運用でよい（常時 load でも対象 0 件なら no-op）。

## 手動・検証

```sh
# DB の post_time で対象レースを確認（fetch しない・ネットワーク不要）
scripts/predict-check/prefetch_odds.sh --dry-run
scripts/predict-check/prefetch_odds.sh --at 15:10 --window-min 30 --dry-run

# 1 回だけ実走（実際に netkeiba から取得し snapshots に積む）
scripts/predict-check/prefetch_odds.sh
```

## cron 代替（任意）

launchd を使わない場合の例（5 分間隔、開催日のみ等は運用で調整）:

```cron
*/5 * * * * cd /path/to/paddock && scripts/predict-check/prefetch_odds.sh >> /tmp/paddock-prefetch.cron.log 2>&1
```

多重起動は `prefetch_odds.sh` 側の `flock`（あれば）で抑止する。
