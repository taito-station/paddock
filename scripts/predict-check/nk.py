"""netkeiba 共通ヘルパ（予想→答え合わせ ハーネス用）.

外部依存なし（curl サブプロセスのみ）。netkeiba は既に本体スクレイパが使う唯一のデータ源。
"""
import re
import subprocess

# JRA 場コード → (slug, 日本語)
VENUES = {
    "01": ("sapporo", "札幌"), "02": ("hakodate", "函館"), "03": ("fukushima", "福島"),
    "04": ("niigata", "新潟"), "05": ("tokyo", "東京"), "06": ("nakayama", "中山"),
    "07": ("chukyo", "中京"), "08": ("kyoto", "京都"), "09": ("hanshin", "阪神"),
    "10": ("kokura", "小倉"),
}
UA = "Mozilla/5.0"


def curl(url: str, timeout: int = 25) -> bytes:
    return subprocess.run(
        ["curl", "-s", "--max-time", str(timeout), url, "-H", f"User-Agent: {UA}"],
        capture_output=True,
    ).stdout


def decode(raw: bytes) -> str:
    # netkeiba は EUC-JP。失敗時は UTF-8 フォールバック。
    try:
        return raw.decode("euc_jp", errors="replace")
    except Exception:
        return raw.decode("utf-8", errors="replace")


def list_race_ids(date_yyyymmdd: str, venue_codes=None):
    """指定日の JRA 全 race_id（12桁）を返す。venue_codes でフィルタ（例 ["05","09"]）。

    race_id = YYYY + 場(2) + 回(2) + 日(2) + R(2)。
    """
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
        nums = re.findall(r'class="Num[^"]*">\s*<div[^>]*>\s*(\d+)\s*</div>', r)
        name = re.search(r'HorseNameSpan">\s*([^<]+?)\s*</span>', r)
        if not nums:
            continue
        horse_num = int(nums[-1]) if len(nums) >= 2 else int(nums[0])
        rows.append({
            "rank": int(rank.group(1)) if rank else None,
            "horse_num": horse_num,
            "name": name.group(1).strip() if name else "",
        })
    return rows
