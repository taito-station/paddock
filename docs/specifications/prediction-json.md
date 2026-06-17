# 予想 JSON 仕様（ingest-predictions 入力契約）

予想（印・短評・買い目・結果）を DB に保存するための JSON。**DB が正で、pad の MD はこの
レコードから生成する**（`--render`）。予想を作るときはこの JSON を吐いて
`paddock-ingest-predictions` に渡せばよく、MD を手書きする必要はない。

## 取り込み

```bash
# stdin から
cat pred.json | cargo run -p ingest-predictions
# ファイルから
cargo run -p ingest-predictions -- --input pred.json
# パース・検証のみ（保存しない）
cargo run -p ingest-predictions -- --input pred.json --dry-run
# DB の全予想を pad の MD に生成
cargo run -p ingest-predictions -- --render
```

単一オブジェクト・オブジェクト配列のどちらでも受け付ける。

## スキーマ

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `date` | string | ○ | 開催日 `YYYY-MM-DD` |
| `venue` | string | ○ | 開催場。日本語(`阪神`)か romaji(`hanshin`) |
| `race_num` | int | ○ | レース番号 |
| `title` | string | | レース名/クラス（H1 に出す） |
| `budget` | int | | 予算（円） |
| `strategy_note` | string | | 買い目の狙い/方針（買い目表の後に出す） |
| `commentary` | string | | 敗因分析等の自由記述（生成 MD 末尾に出す） |
| `horses` | Horse[] | ○ | 各馬（下記） |
| `bets` | Bet[] | | 買い目（下記） |
| `result` | Result | | 結果（答え合わせ後のみ） |

### Horse

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `horse_num` | int | ○ | 馬番 |
| `horse_name` | string | ○ | 馬名 |
| `jockey` | string | | 騎手 |
| `mark` | string | | 印。記号(`◎○▲△☆注`)か slug(`honmei`/`taikou`/`tanana`/`renge`/`hoshi`/`chui`) |
| `win_odds` | number | | 単勝オッズ |
| `popularity` | int | | 人気 |
| `win_prob` | number | | 勝率（**百分率の表示値**。例 `25.4` = 25.4%） |
| `place_prob` | number | | 連対率（百分率） |
| `show_prob` | number | | 複勝率（百分率） |
| `comment` | string | | 短評 |

### Bet

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `bet_type` | string | ○ | 券種（`単勝`/`複勝`/`馬連`/`ワイド`/`馬単`/`3連複`/`3連単` 等、表示ラベル） |
| `combination` | string | ○ | 買い目。arabic 馬番のハイフン連結（`7` / `7-14` / `7-14-13`） |
| `amount` | int | ○ | 金額（円） |

### Result

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `finish` | int[] | | 1〜3 着の馬番（先頭から、最大 3 要素） |
| `recovery_rate` | number | | 回収率（%） |
| `pnl` | int | | 収支（円, 符号付き） |
| `note` | string | | 結果コメント |

## 同定とキー

- レースは `(date, venue, race_num)` で一意（同じキーで再取り込みすると upsert＝冪等）。
- `race_id` は `races`/`race_cards` を `(date, venue, race_num)` で照合できた時だけ自動解決して保持する（未確定・未取込レースでは NULL）。

## 例

```json
{
  "date": "2026-06-13",
  "venue": "hanshin",
  "race_num": 4,
  "title": "3歳未勝利",
  "budget": 10000,
  "strategy_note": "人気軸＋相手広め",
  "horses": [
    {"horse_num":7,"horse_name":"ラパンドール","jockey":"松山","mark":"◎",
     "win_odds":2.4,"popularity":1,"win_prob":25.4,"place_prob":25.4,"show_prob":25.4,
     "comment":"市場・モデルとも単独最上位"}
  ],
  "bets": [
    {"bet_type":"単勝","combination":"7","amount":600},
    {"bet_type":"馬連","combination":"7-14","amount":1000}
  ],
  "result": {"finish":[7,4,13],"recovery_rate":52.1,"pnl":-4790,"note":"印は上位3頭を捕捉"}
}
```
