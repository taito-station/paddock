"""persist_live_ev.py の SQL 生成の最小テスト（pytest 不要・`python3 test_persist_live_ev.py`）.

SQL リテラルのエスケープ（インジェクション面）と build_sql の構造をデグレから守る。
"""
import math

import persist_live_ev as P


def test_lit_str_escapes_single_quote():
    # 単一引用符は 2 重化。standard_conforming_strings=on 前提でバックスラッシュは素通し。
    assert P.lit_str("a'b") == "'a''b'"
    assert P.lit_str("O'Brien") == "'O''Brien'"
    assert P.lit_str("plain") == "'plain'"
    # コロンは psql 変数展開の対象だが引用符内では発動しないためそのまま。
    assert P.lit_str("a:b") == "'a:b'"


def test_lit_num_finite_and_nonfinite():
    assert P.lit_num(None) == "NULL"
    assert float(P.lit_num(125.3)) == 125.3
    assert P.lit_num(0) == "0.0"
    # NaN/Inf は数値リテラルにできないため NULL に落とす（構文エラー防止）。
    assert P.lit_num(float("nan")) == "NULL"
    assert P.lit_num(float("inf")) == "NULL"
    assert P.lit_num(float("-inf")) == "NULL"


def test_lit_bool_int_jsonb():
    assert P.lit_bool(True) == "TRUE" and P.lit_bool(False) == "FALSE"
    assert P.lit_int(5) == "5" and P.lit_int(None) == "NULL"
    j = P.lit_jsonb({"a": 1, "b": "x'y"})
    assert j.endswith("::jsonb")
    # JSON 内の単一引用符も SQL リテラルとして 2 重化されている。
    assert "x''y" in j


def test_build_sql_structure():
    payload = {
        "default_budget": 5000,
        "races": [
            {
                "race_id": "2026-3-tokyo-5-11R", "venue": "tokyo", "race_no": 11,
                "verdict": "bet", "roi": 125.3, "konsen": False, "axis": 4,
                "axis_prob": 35.2, "axis_win_odds": 1.7, "odds_missing": False,
                "slip": {"race_budget": 5000, "legs": []},
            },
            {
                "race_id": "2026-3-tokyo-5-10R", "venue": "tokyo", "race_no": 10,
                "verdict": "skip", "roi": 80.0, "konsen": False, "axis": 2,
                "axis_prob": 48.0, "axis_win_odds": None, "odds_missing": True,
                "slip": {"race_budget": 5000, "legs": []},
            },
        ],
    }
    sql = P.build_sql(payload, "2026-06-20", "2026-06-20T15:20:00Z")
    # トランザクションで括り、エスケープ前提を明示 SET する。
    assert sql.startswith("BEGIN;")
    assert "SET LOCAL standard_conforming_strings TO on;" in sql
    assert sql.rstrip().endswith("COMMIT;")
    # 2 レース分の upsert（ON CONFLICT）。
    assert sql.count("INSERT INTO live_ev_snapshots") == 2
    assert sql.count("ON CONFLICT (race_id, captured_at) DO UPDATE SET") == 2
    # post_time は race_cards サブクエリで補完。
    assert "SELECT post_time FROM race_cards WHERE race_id = '2026-3-tokyo-5-11R'" in sql
    # 欠落オッズは NULL、date/captured_at はリテラル化。
    assert "NULL" in sql
    assert "'2026-06-20'" in sql and "'2026-06-20T15:20:00Z'" in sql


def test_build_sql_empty_races():
    sql = P.build_sql({"races": []}, "2026-06-20", "2026-06-20T15:20:00Z")
    assert "INSERT INTO" not in sql
    assert sql.startswith("BEGIN;") and sql.rstrip().endswith("COMMIT;")


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    main()
