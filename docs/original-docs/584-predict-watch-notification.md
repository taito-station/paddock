# 584 — predict-watch の判定がログに埋もれた実測と、既存通知経路の所見（一次資料）

`paddock-predict-watch` がゲート判定を人に届ける経路を持たず、開催日の判定がログに埋もれたまま
終わっていた件（[#584](https://github.com/taito-station/paddock/issues/584)）の生素材。
**書き換えない**（蒸留は [docs/knowledge/monitor-loop-sleep-resilience.md](../knowledge/monitor-loop-sleep-resilience.md) 側）。

コード所見はいずれも `42a4ff6`（2026-08-23 時点の main）で確認。

## 1. 開催日ごとの実測（`~/Library/Logs/paddock-predict-watch-YYYYMMDD.log`）

判定行は 3 種（`  🔶 ` / `  🔍 ` / `  ・ ` で始まる行）。ヘッダ行の 🔶 表記は集計から除いてある。

| 開催日 | ログ行数 | スイープ数 | 🔶 | 🔍 | ・ | 途切れ警告 |
|---|---:|---:|---:|---:|---:|---:|
| 2026-08-09 | 20,744 | 82 | 0 | 7 | 185 | 0 |
| 2026-08-15 | 17,797 | 75 | 0 | 0 | 160 | 0 |
| 2026-08-16 | 8,351 | 35 | 0 | 0 | 74 | 0 |
| 2026-08-22 | 13,492 | 54 | 0 | 0 | 121 | 0 |

読み取れること:

- **プロセスとしては完璧に動いている**。4 日とも途切れ警告 0 件（#568 のスリープ耐性は効いている）。
  2026-08-09 は 09:43 に開始し 18:33 まで 82 スイープを完走している。
- **判定行はログの 1% 未満**。8/9 は 20,744 行中 192 行（0.93%）。残りは確率テーブル・買い目・
  固定状況の出力で、端末を見ていない限り判定に辿り着けない。
- **ゲート通過（🔶）は 4 日 246 スイープで 0 件**。ADR 0076 の 182R / 839 スイープの実測と整合する。

## 2. 2026-08-09 新潟10R 17:20 — 同一レースが連続スイープで通過し続けた実例

その日唯一の通過帯（🔍・≥70%）は 1 レースに集中していた。7 スイープぶんの参考ROI の推移:

```
73.9% → 80.3% → 80.2% → 79.9% → 79.1% → 79.1% → 76.1%
```

同じレースが窓 40 分 / 間隔 5 分の間に 7 回 Due に入るため、**抑制が無ければこの 1 レースだけで
7 連投**になる。ROI の振れ幅は最大でも 6.4pt（73.9 → 80.3）で、通知に値する新情報を伴っていない。

## 3. 既存の macOS 通知経路（Rust 側にはゼロ）

`Command::new("osascript")` も `notify-rust` 等のクレートも Rust ワークスペースに存在しない。
通知はすべて shell 側にあり、4 本が同一形の `notify()` を個別にコピーしている。

| ファイル | 定義行 | title |
|---|---|---|
| `scripts/predict-check/snapshot_coverage_check.sh` | `:58-61` | `paddock snapshot` |
| `scripts/backup-db.sh` | `:32-36` | `paddock backup` |
| `scripts/backup-staleness-check.sh` | `:23-26` | `paddock backup` |
| `scripts/verify-backup-restore.sh` | `:52-55` | `paddock verify-backup-restore` |

実体（`snapshot_coverage_check.sh:58-61`）:

```bash
notify() {
  # メッセージは argv 経由で AppleScript に渡す（" や \ で壊れない。backup-staleness と同方式）。
  osascript -e 'on run {msg}' -e 'display notification msg with title "paddock snapshot"' -e 'end run' -- "$1" >/dev/null 2>&1 || true
}
```

共通点（4 本とも）:

- 本文は **argv 経由**（`on run {msg}`）。AppleScript の文字列補間をしない
- 失敗は `|| true` で握り潰し、本処理を止めない
- title は `paddock <機能>` 命名
- 連投防止は marker ファイル（`~/Library/Logs/.snapshot-coverage-done.<DATE>` 等）で、
  ROI のような連続値に対する抑制の前例は無い

運用注記は `deployments/launchd/README.md:29`:
「osascript 通知は表示セッション依存でベストエフォート。ログの `GAP` マーカーが一次情報」。

## 4. predict-watch 側のコード所見（`42a4ff6`）

- ゲート判定は純関数 `mark_for(roi, notify_gate, buy_gate)`（`src/apps/predict-watch/src/watch.rs:54-66`）。
  🔶 / 🔍 / ・ の 3 値はすべて**同じ 1 行の println**（`:562-564`）で出る。
  **ゲート通過だけを分岐する if は存在しない**——マーク文字が変わるだけ。
- `evaluate_race`（`:443-612`）は `()` を返し、`SweepCtx`（`:192-202`）は `Copy`。
  `Sweeper::sweep` は `impl Future<Output=()> + Send`（`src/interface/monitor-loop/src/driver.rs:150-155`）
  なので、評価関数へ `&RefCell<_>` を差す形は `Send` を満たさずコンパイルできない。
  **`&mut self` を持てるのは `WatchSweeper::sweep`（`:682`）だけ**。
- スイープを跨いでレース単位の状態を保持する構造は存在しない。近い前例は
  `WatchSweeper.overrides_checked`（`:644`）の「1 度だけ警告する」フラグのみ。
- `Slot`（`:335-340`）は `race`／`post_time`／`race_class` の 3 つで、**競走名を持たない**。
  日付一括で引く API は既にある（`Interactor::race_names_by_date`・
  `src/use-case/src/interactor/race/races_by_date.rs:26`。REST の
  `src/interface/rest-controller/src/handler/race.rs:111` が先行利用している）。
- `--notify-gate`（`src/apps/predict-watch/src/cli.rs:33-39`）は **🔍 の表示閾値**であって
  macOS 通知ではない。名前だけを見ると通知フラグに読める。

## 5. 実装後の実測（2026-08-23）

- `osascript` の終了ステータスは成否を弁別する: 壊れたスクリプトで `1`、正しい呼び出しで `0`。
  ＝ 呼び出し側の `status.success()` による失敗検知は機能する。
- 本文に `"` と `\` を混ぜた通知（`函館10R 15:35 テスト"賞\  ・ 参考ROI 123% ・ 軸7`）が
  終了ステータス 0 で通る（argv 渡しの検査を兼ねた `sends_real_notification` 手動テスト）。
- `--once` の起動注記 3 パターンを実バイナリ + 実 DB で確認（20:05・全レース発走済みのため対象 0 レース）:

```
── macOS 通知: 有効・参考ROI ≥ 100%（--roi-gate と同値）。この閾値は 182R / 839 スイープで通過 0 件（ADR 0076）＝既定では鳴りません。実地検証は --notify-roi を下げてください（例 --notify-roi 0.5）。
── macOS 通知: 有効・参考ROI ≥ 20%（--notify-roi 指定・--roi-gate 100% より下げた実地検証設定）。
── macOS 通知: 無効（--no-notify）。ゲート通過はログの 🔔 行にも出ません。
```

- `--notify-roi nan` は起動時に弾かれる:
  `Error: --notify-roi（NaN）は 0 以上の有限値で指定してください（…）`

**未実施**: 実際にゲートを通過したレースで通知バナーが出るところまでの end-to-end は、
2026-08-23 20:05 時点で当日の全レースが発走済みのため確認できていない。次の開催日に
`--notify-roi` を下げて `🔔` 行とバナーの一致を確認する。
