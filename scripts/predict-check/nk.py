"""netkeiba 共通ヘルパ（予想→答え合わせ ハーネス用）.

外部依存なし（curl サブプロセスのみ）。netkeiba は既に本体スクレイパが使う唯一のデータ源。
"""
import re
import subprocess
import sys

# JRA 場コード → (slug, 日本語)
VENUES = {
    "01": ("sapporo", "札幌"), "02": ("hakodate", "函館"), "03": ("fukushima", "福島"),
    "04": ("niigata", "新潟"), "05": ("tokyo", "東京"), "06": ("nakayama", "中山"),
    "07": ("chukyo", "中京"), "08": ("kyoto", "京都"), "09": ("hanshin", "阪神"),
    "10": ("kokura", "小倉"),
}
# slug → 日本語場名（VENUES から導出。paddock race_id の slug を場名に戻す用）。
SLUG2JP = {slug: jp for slug, jp in VENUES.values()}
UA = "Mozilla/5.0"


def curl(url: str, timeout: int = 25) -> bytes:
    # 取得失敗を黙って空 bytes で流すと、結果 0 件が「正常」として集計を静かに汚す。
    # returncode 非 0（通信失敗・タイムアウト）は例外、本文空は警告で気づけるようにする。
    # `--url <url>` で URL を渡し、`-` 始まりでもオプション誤認されないようにする。
    p = subprocess.run(
        ["curl", "-sf", "--max-time", str(timeout), "-H", f"User-Agent: {UA}", "--url", url],
        capture_output=True,
    )
    if p.returncode != 0:
        raise RuntimeError(f"curl 失敗 (exit {p.returncode}): {url}")
    if not p.stdout:
        print(f"[warn] 空レスポンス: {url}", file=sys.stderr)
    return p.stdout


def decode(raw: bytes) -> str:
    # netkeiba は EUC-JP。errors 無指定の strict で試し、失敗時のみ UTF-8 フォールバック
    # （errors="replace" だと例外が出ずフォールバックに到達しないため strict にする）。
    try:
        return raw.decode("euc_jp")
    except UnicodeDecodeError:
        return raw.decode("utf-8", errors="replace")


def list_race_ids(date_yyyymmdd: str, venue_codes=None):
    """指定日の JRA 全 race_id（12桁）を返す。venue_codes でフィルタ（例 ["05","09"]）。

    race_id = YYYY + 場(2) + 回(2) + 日(2) + R(2)。
    """
    if not re.fullmatch(r"\d{8}", date_yyyymmdd):
        raise ValueError(f"date は YYYYMMDD 8桁: {date_yyyymmdd!r}")
    if venue_codes and not all(re.fullmatch(r"\d{2}", v) for v in venue_codes):
        raise ValueError(f"venue コードは 2桁数字: {venue_codes!r}")
    url = f"https://race.netkeiba.com/top/race_list_sub.html?kaisai_date={date_yyyymmdd}"
    html = decode(curl(url))
    ids = sorted(set(re.findall(r"race_id=([0-9]{12})", html)))
    if venue_codes:
        vs = set(venue_codes)
        ids = [i for i in ids if i[4:6] in vs]
    return ids


def parse_race_id(rid: str):
    """12桁 race_id を (year, venue_code, slug, jp, round, day, race_num) に分解。"""
    vc = rid[4:6]
    slug, jp = VENUES.get(vc, (vc, vc))
    return {
        "race_id": rid, "year": int(rid[0:4]), "venue_code": vc,
        "venue_slug": slug, "venue_jp": jp,
        "round": int(rid[6:8]), "day": int(rid[8:10]), "race_num": int(rid[10:12]),
    }


def fetch_result(rid: str):
    """race/result.html をパースし finishing rows を返す。

    各 HorseList 行: Rank=着順 / 2列目 Num=枠 / 3列目 Num=馬番 / HorseNameSpan=馬名。
    """
    url = f"https://race.netkeiba.com/race/result.html?race_id={rid}"
    html = decode(curl(url))
    rows = []
    for r in re.findall(r'class="HorseList">(.*?)</tr>', html, re.S):
        rank = re.search(r'class="Rank">\s*(\d+)\s*<', r)
        name = re.search(r'HorseNameSpan">\s*([^<]+?)\s*</span>', r)
        # 馬番は `class="Num Txt_C"` セル（枠 `Num WakuN` と区別する）。これを優先し、
        # 取れなければ Num セル列の最後（=馬番）にフォールバック。どちらも無ければ行をスキップ。
        mnum = re.search(r'class="Num[^"]*Txt_C[^"]*">\s*<div[^>]*>\s*(\d+)\s*</div>', r)
        if mnum:
            horse_num = int(mnum.group(1))
        else:
            nums = re.findall(r'class="Num[^"]*">\s*<div[^>]*>\s*(\d+)\s*</div>', r)
            if not nums:
                continue
            horse_num = int(nums[-1])
        rows.append({
            "rank": int(rank.group(1)) if rank else None,
            "horse_num": horse_num,
            "name": name.group(1).strip() if name else "",
        })
    # HTML 取得は成功したのに 1 行も取れない＝サイト構造変化の疑い。集計を黙って汚さないよう警告。
    if not rows:
        print(f"[warn] 結果行を抽出できませんでした（HTML 構造変化の疑い）: {rid}", file=sys.stderr)
    return rows


# 払戻ブロックの券種クラス（<tr class="...">）→ paddock の type_label。
# Wakuren（枠連）は predict 非対象のため載せない（マップに無い券種は呼び出し側で無視）。
# 本体 Rust の src/interface/netkeiba-scraper/src/parse/payout.rs と同じ対応。
PAYOUT_TYPE = {
    "Tansho": "win", "Fukusho": "place", "Umaren": "quinella",
    "Wide": "wide", "Umatan": "exacta", "Fuku3": "trio", "Tan3": "trifecta",
}
# 無順（quinella/wide/trio）は組番を昇順ソートして `-` 連結、順序付きは出現順 `>` 連結。
_UNORDERED = {"quinella", "wide", "trio"}


def fetch_payouts(rid: str):
    """race/result.html の払戻ブロックから確定配当を抽出する（答え合わせ・戦略評価用）.

    返り値: {type_label: {combination_code: payout_per_100}}（100 円あたり払戻[円]）。
    combination_code は本体 BetCombination::combination_code と一致する形式
    （単複=馬番 "17" / 無順="5-13-17" / 順序付き="17>13>5"）。

    本体 Rust parse_race_payouts(parse/payout.rs) の規則をミラー:
    table.Payout_Detail_Table（ワイド以降は別テーブルなので複数並ぶ）の各 <tr class>。
    """
    url = f"https://race.netkeiba.com/race/result.html?race_id={rid}"
    html = decode(curl(url))
    out = {}
    n_rows = 0
    # 払戻テーブルは複数並ぶ。各テーブル内の <tr ...class="..."> を順に処理する。
    # class は値全体を取り空白区切りで券種語を引く（複数クラス・他属性の付加に耐える）。
    # 非貪欲 `.*?</table>` は払戻テーブルが入れ子 table を持たない flat 構造である前提
    # （netkeiba の現行 DOM。入れ子になると最初の </table> で切れる）。崩れたら有効払戻 0 件
    # 警告で気づける。
    for table in re.findall(r'class="Payout_Detail_Table".*?</table>', html, re.S):
        for cls, body in re.findall(r'<tr\b[^>]*\bclass="([^"]*)"[^>]*>(.*?)</tr>', table, re.S):
            label = next((PAYOUT_TYPE[c] for c in cls.split() if c in PAYOUT_TYPE), None)
            if label is None:
                continue  # 枠連・想定外クラスはスキップ
            n_rows += 1
            result_cell = re.search(r'<td class="Result">(.*?)</td>', body, re.S)
            payout_cell = re.search(r'<td class="Payout">(.*?)</td>', body, re.S)
            if not result_cell or not payout_cell:
                continue
            # 配当: 「数字（桁区切りカンマ可）＋直後 円」を順に。`5人気` 等の円無し数字は拾わない。
            amounts = [int(a.replace(",", ""))
                       for a in re.findall(r'([\d,]+)円', payout_cell.group(1))]
            # span は属性付き（<span class=...> 等）でも拾えるようにする（CSS セレクタの
            # Rust 実装と挙動を揃える）。数字を含まない空 span は (\d+) 不一致で自然に除外。
            if label in ("win", "place"):
                # 単勝/複勝: div>span の数字のある馬番。複勝は馬番ごとに 1 点。
                # div 配下に限定し（Rust の `div span` セレクタと一致）、td.Result 内に将来
                # div 外の数字 span が混入しても馬番として誤検出しないようにする。
                combos = re.findall(
                    r'<div[^>]*>\s*<span[^>]*>\s*(\d+)\s*</span>', result_cell.group(1))
            else:
                # 組合せ券種: ul 1 つ＝1 組合せ。li>span の馬番（空 li/span は数字無しで除外）。
                combos = []
                for ul in re.findall(r'<ul>(.*?)</ul>', result_cell.group(1), re.S):
                    nums = [int(x) for x in re.findall(r'<span[^>]*>\s*(\d+)\s*</span>', ul)]
                    if not nums:
                        continue
                    if label in _UNORDERED:
                        combos.append("-".join(str(x) for x in sorted(nums)))
                    else:
                        combos.append(">".join(str(x) for x in nums))
            # 組合せ数と配当数が食い違う行は対応がズレ誤った組番に配当を貼るおそれ。
            # 当該券種を skip して warn（払戻金額に直結するため沈黙させない）。
            if len(combos) != len(amounts):
                print(f"[warn] {rid} {label}: 組合せ {len(combos)} 件 / 配当 {len(amounts)} 件 "
                      f"が不一致のためスキップ", file=sys.stderr)
                continue
            bucket = out.setdefault(label, {})
            for code, pay in zip(combos, amounts):
                bucket[code] = pay
    # 有効な払戻が 1 件も取れない＝中止/全馬取消、または構造変化。空 dict を返し警告。
    # 対象行を検出したのに out が空（セル欠落や件数不一致で全 skip）のケースも拾えるよう
    # n_rows ではなく out で判定する（検出行数を併記して構造変化に気づけるようにする）。
    if not out:
        print(f"[warn] 有効な払戻を抽出できませんでした"
              f"（中止/全馬取消 or 構造変化の疑い, 検出行数={n_rows}）: {rid}", file=sys.stderr)
    return out
