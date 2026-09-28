# Step 3: 横断的一貫性レビュー

## このステップの目的

Step 2.1（画面定義）/ Step 2.2（マイクロサービス定義）/ Step 2.3・2.4（TDD テストスペック）の全成果物が完了した時点で、画面 ↔ サービス ↔ テスト仕様の整合性を横断レビューし、UI 要件・API 要件・テスト要件の不整合を検出する。

## 入力

- `docs/screen/*.md`（Step 2.1 の全出力）
- `docs/services/*.md`（Step 2.2 の全出力）
- `docs/test-specs/*-test-spec.md`（Step 2.3・2.4 の全出力）
- `docs/catalog/service-catalog-matrix.md`
- `docs/catalog/app-catalog.md`
- `docs/catalog/data-model.md`

## 出力

- `docs/catalog/screen-service-consistency-report.md`

## 依存

- Step 2.1・2.2・2.3・2.4 の全 fan-out 子が完了していること（AND join）。

## 実行手順

Step 3 は fan-out せず、単発実行する（メインループ自身、または単一の Agent 呼び出しで実行）。

1. Step 2.1〜2.4 の全出力ファイルを読み込む。
2. 下記のレビュー観点で照合する。
3. 検出した不整合を `docs/catalog/screen-service-consistency-report.md` に出力する。

## レビュー観点

1. **画面 → API 整合性**: 各画面定義の「呼び出す API」が `service-catalog-matrix.md` および `docs/services/*.md` に存在するか
2. **API → 画面整合性**: 各サービス定義が画面側で実際に利用されているか
3. **テスト仕様 ↔ 画面/サービス整合性**: 各 test-spec が対応する画面またはサービスに紐づき、ID 命名規約に従っているか
4. **APP-ID 配分一貫性**: `docs/catalog/app-catalog.md` の各 APP に紐づく画面・サービスがすべての成果物で網羅されているか
5. **データモデル整合性**: 画面 I/O とサービス I/O が `docs/catalog/data-model.md` のエンティティと矛盾しないか

## 出力フォーマット

不整合の一覧表:

| No. | ファイル | 問題種別 | 重大度 | 根拠 | 説明 | 修正案 |
|-----|---------|---------|--------|------|------|--------|

重大度: `critical`（論理矛盾・実装/API/データ整合性への重大な影響） / `major`（判断に必要な情報欠落） / `minor`（表記揺れ・軽微な形式問題）

末尾にステータスサマリ（`OK` / `要修正` / `要確認`）を記載する。

## 完了条件

- `docs/catalog/screen-service-consistency-report.md` が作成されている。
- 検出された不整合項目が明示されている（確認できない項目は「要確認」と記載し、捏造しない）。
- ステータスサマリ（`OK` / `要修正` / `要確認`）が含まれている。

---

**捏造禁止**: 実際に読み取った内容だけを根拠にする。確認できない項目は `TBD（要確認）` と明記する。
**オーバーエンジニアリング禁止**: 指示にない未来予測的な汎用化・抽象化・将来拡張点の先回り追加を行わない。YAGNI に従う。
