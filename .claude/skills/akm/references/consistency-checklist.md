# 横断整合性チェックリスト

AKM Step 3 で使用する。HVE の QA-DocConsistency に相当する。

## 機械検査（スクリプトで実行）

以下は `scripts/check-doc-classes.py` と `scripts/check-decision-log-immutability.py` が検査する。
手動で目視する必要はない。

- [ ] `doc_class` と `tags` の一致
- [ ] `sources` のパスが実在する
- [ ] `distilled_from_sha` が stale でない
- [ ] 本文の相対リンクが実在する
- [ ] `doc-classes.md` の割当索引と実ファイルが 1:1 で対応する
- [ ] REQ-ID がクラス内でグローバルに一意
- [ ] REQ 表の `出典` が `sources` にも載っている（docs-original を名指しする場合）
- [ ] 決定ログが append-only（既存エントリの改変・削除がない）

## 人手の確認（機械検査が届かない範囲）

以下は機械で検出できないため、AKM Step 3 で目視確認する。

### 用語の整合性

- [ ] glossary.md（D07）に定義された用語が、各文書で同じ意味で使われているか
  - 特に: `win_prob` のスケール（0-1 vs %）、`blended` の α の方向（α=1.0 が純モデル）、
    `軸ロック` / `混戦` / `ながし` の定義
- [ ] 新しい用語が本文に初出したとき、glossary.md に索引エントリが追加されているか

### SoT チェーン

- [ ] Confirmed な knowledge が Tentative な source だけを根拠にしていないか
  - 根拠の source がすべて Tentative なら、knowledge も Tentative であるべき
- [ ] docs-original の記述と knowledge の記述が矛盾していないか
  - 矛盾がある場合: docs-original が勝つ（SoT 優先順位）

### CLAUDE.md との整合

- [ ] CLAUDE.md の「買い方ルール」と specifications の記述が矛盾していないか
  - 特に: 予算・配分・混戦判定の閾値・相手の広さ
  - CLAUDE.md を変えたら specifications も、specifications を変えたら CLAUDE.md も直す
- [ ] CLAUDE.md の「予想ワークフロー」と knowledge の記述が矛盾していないか
  - 特に: fetch-card の手順、predict のオプション、番兵値の扱い

### 決定ログの網羅性

- [ ] 最近の設計判断・ルール変更・実験の採用/棄却が決定ログに記録されているか
  - 機械検査は「書いたかどうか」を検出できない（append-only の不変性しか見ない）
- [ ] 決定ログで supersede されたエントリの本文側が更新されているか
  - 決定ログに「ADR XXXX を supersede」と書いても、本文が古いままなら読者は古い知を読む

### sources の網羅性

- [ ] 本文が根拠にしている一次資料が `sources` に漏れなく載っているか
  - 機械検査は REQ 表の `出典` 列しか見ない。本文の散文で根拠にしている資料は検出できない
  - `sources` から行を消せば stale も消えるので、誤って消していないか確認する
