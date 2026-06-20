"""netkeiba のライブ（発走前）ワイドオッズを取得する.

本体 `fetch-card` は単複・馬連・馬単・三連複・三連単を取得・保存するが
**ワイド(type=5)は未対応**（Issue #187）。ライブ EV 計算でワイドを反映するため、
netkeiba のオッズ API (`api_get_jra_odds.html?type=5`) を直接叩く。

確定後の払戻を取る `nk.fetch_payouts`（result.html パース）とは別ソース＝
発走前の変動オッズである点に注意。

使い方:
    python3 fetch_wide.py <netkeiba_race_id> [paddock_race_id]
        → `<key>\t<a-b>\t<mid>` を1行ずつ stdout（key は引数2が無ければ netkeiba_id）

API レスポンス: data.odds["5"]["0102"] = [low, high, popularity]（"0102"=馬番2桁×2）。
mid = (low+high)/2 を採用（ワイドは下限〜上限のレンジで返る）。
"""
import json
import sys

import nk

ODDS_API = "https://race.netkeiba.com/api/api_get_jra_odds.html?race_id={rid}&type=5&action=update"


def fetch_wide(rid: str):
    """netkeiba race_id のライブワイドオッズを {(a,b): mid_odds} で返す（a<b の馬番）。"""
    raw = nk.curl(ODDS_API.format(rid=rid))
    # オッズ API は UTF-8 JSON。空レスポンス（nk.curl は空 bytes を返しうる）や
    # 構造変化で JSON にならない場合は警告して空 dict を返す（単独実行時のトレースバック回避）。
    try:
        data = json.loads(raw.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        print(f"[warn] JSON パース失敗（空レスポンス or 構造変化）: {rid}", file=sys.stderr)
        return {}
    odds = data.get("data", {}).get("odds", {}).get("5", {})
    out = {}
    for key, vals in odds.items():
        if len(key) != 4:  # "0102" 以外（合計行等）は無視
            continue
        a, b = int(key[:2]), int(key[2:])
        try:
            lo = float(str(vals[0]).replace(",", ""))
            hi = float(str(vals[1]).replace(",", ""))
        except (ValueError, IndexError, TypeError):
            continue
        # ワイドは「下限 <= 上限」の正のレンジで返る。崩れていれば構造変化/異常値として捨てる
        # （lo>hi のまま mid を取ると誤った中央値になるため）。
        if lo <= 0 or hi < lo:
            continue
        out[tuple(sorted((a, b)))] = (lo + hi) / 2
    if not out:
        print(f"[warn] ワイドオッズを抽出できませんでした（発走前で未確定 or 構造変化）: {rid}",
              file=sys.stderr)
    return out


def main():
    if len(sys.argv) < 2:
        print("usage: python3 fetch_wide.py <netkeiba_race_id> [paddock_race_id]",
              file=sys.stderr)
        sys.exit(2)
    rid = sys.argv[1]
    key = sys.argv[2] if len(sys.argv) > 2 else rid
    for (a, b), mid in fetch_wide(rid).items():
        print(f"{key}\t{a}-{b}\t{mid}")


if __name__ == "__main__":
    main()
