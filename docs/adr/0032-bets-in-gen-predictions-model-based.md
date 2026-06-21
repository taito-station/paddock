# 0032. gen_predictions.py の買い目はモデル確率ベースで常に生成する

## ステータス
承認済み

## コンテキスト

`gen_predictions.py`（朝の一括予想生成スクリプト）が生成する prediction JSON の
`bets` フィールドが空で、Obsidian web-viewer の「買い目」欄が常に空白になっていた（#201）。

買い目生成には2つの選択肢があった。

- **案 A**: モデル確率だけから常に生成する（EV フィルタなし）
- **案 B**: ワイド/馬連/3連複オッズを取得・ROI を計算し、+EV レースだけ付ける

## 決定

**案 A（モデル確率ベース、EV フィルタなし）**を採用する。

`build_bets`（`live_ev.py` から import）を用い、各レースのモデル勝率から
確率重み配分で買い目を算出して `bets` に載せる。予算は ¥5,000/レース 固定。

## 理由

- `gen_predictions.py` は朝の一括実行を想定しており、その時点でワイドオッズ（netkeiba
  type=5）や馬連/3連複オッズ（発売前または未取得）が揃っていないケースが多い。
- `build_bets` の入力はモデル勝率のみで、ここから組合せ・金額を確定できる。
  ROI 計算（`race_roi`）は別の関心事であり、EV 判断は引き続き `refresh_ev.sh` が担う。
- 案 B は生成に netkeiba fetch が必要で、gen_predictions.py を遅くし・外部依存を増やす。
  既存の役割分担（gen_predictions = 本命確認 + 買い目案、refresh_ev = EV 判定）を崩さない。

## 影響

- `gen_predictions.py` が常に `bets` を含む prediction JSON を出力するようになる。
  Obsidian web-viewer に買い目案が表示される。
- `bets` はモデル確率時点の「デフォルト買い目案」であり、ライブ EV で −EV と判定された
  レースでも表示される。レース選択（張る/見送り）は引き続きライブ EV の ROI ≥ 100% で行う。
- `live_ev.py` のロジック（`build_bets`/`is_konsen`/`band_of` 等）を変更した場合、
  gen_predictions.py も同じ変更の影響を受ける（同一モジュールを import しているため自動的に追従）。
