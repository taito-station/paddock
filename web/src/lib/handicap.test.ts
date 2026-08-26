import { describe, expect, it } from "vitest";
import {
  CARD_MAX_POSITIONS,
  NO_RECORD,
  PREMISE_POPULARITY_MAX,
  type ConditionRun,
  type HandicapNote,
  conditionLabel,
  conditionRecordSummary,
  conditionRunLine,
  edgePtLabel,
  groupConditionLabel,
  layoffLabel,
  modelEdgePt,
  premiseBadges,
  showsGroupRecord,
} from "./handicap";

function run(date: string, pos: number, raceName?: string): ConditionRun {
  return {
    date,
    finishing_position: pos,
    race_name: raceName ?? null,
  };
}

function note(overrides: Partial<HandicapNote> = {}): HandicapNote {
  return {
    course_runs: [],
    group_runs: [],
    layoff_days: null,
    distance_untried: false,
    surface_untried: false,
    no_past_runs: false,
    ...overrides,
  };
}

describe("conditionLabel", () => {
  it("スラッグを日本語の条件ラベルにする", () => {
    expect(conditionLabel("niigata", "turf", 1000)).toBe("新潟芝1000m");
    expect(conditionLabel("tokyo", "dirt", 1600)).toBe("東京ダ1600m");
  });

  it("未知のスラッグはそのまま出す（表記を壊さない）", () => {
    expect(conditionLabel("unknown", "turf", 1200)).toBe("unknown芝1200m");
  });
});

describe("groupConditionLabel", () => {
  it("洋芝グループを併記ラベルにする", () => {
    expect(groupConditionLabel(["sapporo", "hakodate"], "turf", 2000)).toBe(
      "洋芝(札幌/函館)芝2000m",
    );
  });

  it("グループが空（＝当場のみ）なら null＝2 行目を出さない", () => {
    // 完全一致と同じ集合を 2 回書いても情報が増えないため。
    expect(groupConditionLabel([], "turf", 1000)).toBeNull();
  });
});

describe("conditionRecordSummary", () => {
  it("走数と着順（新しい順）を 1 行にする", () => {
    expect(
      conditionRecordSummary([run("2026-08-02", 3), run("2026-05-10", 3)]),
    ).toBe("2走 3,3着");
  });

  it("0 走は空欄でなく「該当なし」と明示する", () => {
    // 空欄は「まだ引いていない」に見えるため、走っていないことを言葉で出す（#628）。
    expect(conditionRecordSummary([])).toBe(NO_RECORD);
    expect(conditionRecordSummary([])).not.toBe("");
  });

  it("着順は上限まで・走数は常に全件を出す", () => {
    // 2026-08-16 新潟6R ⑩クールベイビー相当（千直 9 走）。狭幅カラムで折り返して
    // カード高さが揃わなくなるため着順だけ省略し、「9走」という母数は落とさない。
    // 期待値は定数から組む（リテラルで書くと上限を変えたとき原因の分かりにくい失敗になる）。
    const runs = Array.from({ length: CARD_MAX_POSITIONS + 4 }, (_, i) =>
      run(`2026-01-${String(i + 1).padStart(2, "0")}`, i + 1),
    );
    const shown = runs
      .slice(0, CARD_MAX_POSITIONS)
      .map((r) => r.finishing_position)
      .join(",");
    expect(conditionRecordSummary(runs)).toBe(`${runs.length}走 ${shown}着…`);
  });

  it("上限ちょうどなら省略記号を付けない", () => {
    const runs = Array.from({ length: CARD_MAX_POSITIONS }, (_, i) =>
      run(`2026-01-${String(i + 1).padStart(2, "0")}`, i + 1),
    );
    expect(conditionRecordSummary(runs).endsWith("…")).toBe(false);
  });
});

describe("showsGroupRecord", () => {
  it("完全一致より件数が増えているときだけ出す", () => {
    expect(
      showsGroupRecord(
        note({
          course_runs: [],
          group_runs: [run("2026-07-05", 1), run("2026-06-14", 5)],
        }),
      ),
    ).toBe(true);
  });

  it("同数なら同じ集合の再掲なので出さない", () => {
    const runs = [run("2026-07-05", 1)];
    expect(
      showsGroupRecord(note({ course_runs: runs, group_runs: runs })),
    ).toBe(false);
  });
});

describe("modelEdgePt / edgePtLabel", () => {
  it("純モデル − 市場を pt で返す", () => {
    // 2026-08-15 札幌9R ④キャトルブランシュ相当（純 10.4% vs 市場 1.1%）。
    expect(modelEdgePt(0.104, 0.011)).toBeCloseTo(9.3, 5);
    expect(edgePtLabel(modelEdgePt(0.104, 0.011))).toBe("+9.3pt");
  });

  it("負の差は符号付きで出す", () => {
    expect(edgePtLabel(modelEdgePt(0.05, 0.12))).toBe("-7.0pt");
  });

  it("市場 implied 未取得（単勝未発売）は null → 表記は -", () => {
    expect(modelEdgePt(0.104, null)).toBeNull();
    expect(modelEdgePt(0.104, undefined)).toBeNull();
    expect(edgePtLabel(null)).toBe("-");
  });
});

describe("layoffLabel", () => {
  it("日数をそのまま出す（久々かどうかの判定はしない）", () => {
    expect(layoffLabel(14)).toBe("間隔14日");
    // 10ヶ月半も 4ヶ月半も同じ書式で出し、質の読み分けは人に残す（#628）。
    expect(layoffLabel(315)).toBe("間隔315日");
    expect(layoffLabel(136)).toBe("間隔136日");
  });

  it("パネル側の見出し（前走からの間隔）と用語を揃える", () => {
    // 「休養」だと中1週の通常ローテにも休養明けのラベルが付いて語感が壊れる。
    expect(layoffLabel(7)).not.toContain("休養");
  });

  it("過去走なしは null", () => {
    expect(layoffLabel(null)).toBeNull();
    expect(layoffLabel(undefined)).toBeNull();
  });
});

describe("premiseBadges", () => {
  it("上位人気には事実バッジを出す", () => {
    const badges = premiseBadges(
      note({ layoff_days: 315, distance_untried: true }),
      1,
    );
    expect(badges).toEqual(["間隔315日", "距離初"]);
  });

  it("芝ダ未経験も出す", () => {
    // 2026-08-16 新潟7R ②番人気⑨ヤマニンアルリフラ相当（近 3 走すべて芝でダート戦へ）。
    expect(premiseBadges(note({ surface_untried: true }), 2)).toEqual([
      "芝ダ初",
    ]);
  });

  it("過去走 0 件の馬には未経験バッジを出さない", () => {
    // サーバ側で same_*_starts == 0 由来なのでデータ欠損馬では必ず真になる。
    // 「未経験」として出すと「データが無い」と取り違える（カードは 戦績データなし に一本化）。
    const badges = premiseBadges(
      note({
        no_past_runs: true,
        distance_untried: true,
        surface_untried: true,
      }),
      1,
    );
    expect(badges).toEqual([]);
  });

  it("過去走 0 件でも間隔が分かっていれば間隔だけは出す（防御的分岐）", () => {
    // サーバは `total_starts == 0 ⟺ last_run_date == None` なのでこの組み合わせを生成しない。
    // 「no_past_runs なら間隔も落とす」という実装にしないための境界固定。
    const badges = premiseBadges(
      note({ no_past_runs: true, distance_untried: true, layoff_days: 30 }),
      1,
    );
    expect(badges).toEqual(["間隔30日"]);
  });

  it("上位人気の外は出さない（全頭に出すとノイズになる）", () => {
    const n = note({ layoff_days: 315, distance_untried: true });
    expect(premiseBadges(n, PREMISE_POPULARITY_MAX)).not.toEqual([]);
    expect(premiseBadges(n, PREMISE_POPULARITY_MAX + 1)).toEqual([]);
  });

  it("人気不明（単勝未取得）は出さない", () => {
    expect(premiseBadges(note({ distance_untried: true }), null)).toEqual([]);
  });

  it("材料が無ければ空（空のバッジ行を作らない）", () => {
    expect(premiseBadges(note(), 1)).toEqual([]);
  });
});

describe("conditionRunLine", () => {
  it("日付・着順・レース名を 1 行にする", () => {
    expect(conditionRunLine(run("2026-08-02", 3, "キーンランドC"))).toBe(
      "2026-08-02 3着 キーンランドC",
    );
  });

  it("レース名が無い（PDF 経路）ときは日付と着順だけ", () => {
    expect(conditionRunLine(run("2026-08-02", 3))).toBe("2026-08-02 3着");
  });
});
