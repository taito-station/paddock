# QA: results の行が足りないレースの補完（#742・2026-10-03）

#742 の計画承認時に確定した方針の記録。一次資料: `docs-original/742-fill-missing-results.md`。
蒸留先: `knowledge/race-result-ingestion.md`（書き込み口・決定ログ #742）・`knowledge/netkeiba-datasource.md`（結果ページの生の HTML の取得）。

## Q1: 入り口（どの経路で INSERT するか）

- 選択肢: **専用の小さな CLI `paddock-fill-results`**（推奨）/ `ResultsInteractor::refresh` に「行不足のレースを対象にする」モードを足す /
  `paddock-fetch-results` に「無い馬番を出馬表から補って INSERT する」オプションを足す。
- 回答: **専用の CLI**。ADR 0015（`fetch-results` は既存行の UPDATE 専用）の契約は変えない。
- 補足（設計レビューで判明）: refresh の書き込み口 `upsert_results` は `ON CONFLICT DO UPDATE` で既存行を上書きするので、
  モードを足しても「既存行は上書きしない」を満たせない。CLI の書き込みは `ON CONFLICT DO NOTHING` の専用メソッドにした。

## Q2: 取消・除外の馬の扱い

- 選択肢: **出馬表と結果ページの両方にある馬番は status ごと全部入れる**（推奨・`upsert_results` と同じ扱い）/ 出走馬だけ入れる。
- 回答: **status ごと全部入れる**。成功条件の照合は出走馬（`finished`・`did_not_finish`）の行数で行う。
- 実データ: 取消・除外として入ったのは 2 行（中京 6 日 11R の 12 番・12R の 7 番、どちらも `scratched`）。

## Q3: 照合に失敗したレース（作業中の判断）

- 既存の 1〜5 行が出馬表と食い違うレース（馬名が違う・既存行が取消なのに netkeiba では出走馬・出走馬が出馬表に無い・
  入れた後の出走馬の数が結果ページと合わない）は、書かずに非ゼロで終える。食い違いが出たら、そのレースの扱いをユーザーに相談する。
- 実データ: 16 本とも照合を通過し、相談は不要だった。

## Q4: 共有 DB への適用（作業中の確認）

- 隔離 DB で同じ手順の照合が通った後、ユーザーの承認を得てから適用した（直前のバックアップを pinned に退避）。
