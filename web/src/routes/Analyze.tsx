import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api, type GroupStat } from "../api/client";
import { pct, SURFACE_JP, VENUE_JP, todayJst } from "../lib/format";
import { isIsoDate, raceListHref } from "../lib/header-date";
import {
  parseKind,
  parseAnalyzeParams,
  analyzeSearchParams,
  isVenueSlug,
  type Kind,
  type NameKind,
  type CourseParams,
} from "../lib/analyze";

const KIND_LABEL: Record<Kind, string> = {
  horse: "馬",
  jockey: "騎手",
  trainer: "調教師",
  course: "コース",
};

// JRA 場順で会場セレクトの option を出す（VENUE_JP のキー順に一致）。
const VENUE_SLUGS = Object.keys(VENUE_JP);

function StatTable({ title, rows }: { title: string; rows: GroupStat[] }) {
  if (rows.length === 0) return null;
  return (
    <div className="stat-block">
      <h3>{title}</h3>
      <table className="grid">
        <thead>
          <tr>
            <th>区分</th>
            <th>出走</th>
            <th>勝</th>
            <th>連対</th>
            <th>複勝</th>
            <th>勝率</th>
            <th>連対率</th>
            <th>複勝率</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((g) => (
            <tr key={g.label}>
              <td>{g.label}</td>
              <td>{g.starts}</td>
              <td>{g.wins}</td>
              <td>{g.places}</td>
              <td>{g.shows}</td>
              <td>{pct(g.win_rate)}</td>
              <td>{pct(g.place_rate)}</td>
              <td>{pct(g.show_rate)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// name 系タブの lift 状態。
type NameSlot = { input: string; submitted: string };
type NameState = Record<NameKind, NameSlot>;
// course タブの lift 状態（form=編集中 / submitted=確定した検索）。
type CourseState = { form: CourseParams; submitted: CourseParams | null };

export function Analyze() {
  const [searchParams, setSearchParams] = useSearchParams();
  // 分析は全期間統計（date でフィルタしない）。ヘッダの開催日を戻る導線に引き継ぐだけ（#379）。
  const dateParam = searchParams.get("date");
  const date = isIsoDate(dateParam) ? dateParam : todayJst();
  // アクティブタブは URL 正（?kind=）。既定 horse。
  const kind = parseKind(searchParams.get("kind"));

  // 各タブの検索状態は Analyze に lift して切替で保持する（#384。key 再マウント廃止）。
  // 初期値は URL からアクティブタブ分を hydrate（リロード/共有復元）。
  const [names, setNames] = useState<NameState>(() => {
    const init = parseAnalyzeParams(searchParams);
    const q = init.kind !== "course" ? init.name : "";
    const slot = (k: NameKind): NameSlot =>
      init.kind === k ? { input: q, submitted: q } : { input: "", submitted: "" };
    return { horse: slot("horse"), jockey: slot("jockey"), trainer: slot("trainer") };
  });
  const [course, setCourse] = useState<CourseState>(() => {
    const init = parseAnalyzeParams(searchParams);
    const c = init.course;
    const valid =
      init.kind === "course" && isVenueSlug(c.venue) && /^\d+$/.test(c.distance);
    return { form: c, submitted: valid ? c : null };
  });

  // アクティブタブの状態を URL に反映（date+kind+検索語）。履歴は汚さない。
  const writeUrl = (k: Kind, ns: NameState, cs: CourseState) => {
    const active =
      k === "course" ? { course: cs.submitted } : { name: ns[k as NameKind].submitted };
    setSearchParams(analyzeSearchParams(k, active, date), { replace: true });
  };

  const goKind = (k: Kind) => writeUrl(k, names, course);

  const onNameInput = (k: NameKind, input: string) =>
    setNames((s) => ({ ...s, [k]: { ...s[k], input } }));
  const onNameSubmit = (k: NameKind, submitted: string) => {
    const next: NameState = { ...names, [k]: { input: submitted, submitted } };
    setNames(next);
    writeUrl(k, next, course);
  };

  const onCourseForm = (form: CourseParams) =>
    setCourse((s) => ({ ...s, form }));
  const onCourseSubmit = (form: CourseParams) => {
    const next: CourseState = { form, submitted: form };
    setCourse(next);
    writeUrl("course", names, next);
  };

  return (
    <section>
      <div className="toolbar">
        <Link to={raceListHref(date)}>← レース一覧へ</Link>
      </div>
      <div className="tabs">
        {(Object.keys(KIND_LABEL) as Kind[]).map((k) => (
          <button
            key={k}
            className={k === kind ? "tab tab-active" : "tab"}
            onClick={() => goKind(k)}
          >
            {KIND_LABEL[k]}
          </button>
        ))}
      </div>
      {kind === "course" ? (
        // 状態は親が保持（lift）。key を付けずタブ切替でも入力・結果を維持する。
        <CourseAnalyze
          form={course.form}
          submitted={course.submitted}
          onForm={onCourseForm}
          onSubmit={onCourseSubmit}
        />
      ) : (
        <NameAnalyze
          kind={kind}
          input={names[kind].input}
          submitted={names[kind].submitted}
          onInput={(v) => onNameInput(kind, v)}
          onSubmit={(v) => onNameSubmit(kind, v)}
        />
      )}
    </section>
  );
}

function NameAnalyze({
  kind,
  input,
  submitted,
  onInput,
  onSubmit,
}: {
  kind: NameKind;
  input: string;
  submitted: string;
  onInput: (v: string) => void;
  onSubmit: (v: string) => void;
}) {
  const path = (
    {
      horse: "/api/analyze/horse",
      jockey: "/api/analyze/jockey",
      trainer: "/api/analyze/trainer",
    } as const
  )[kind];

  const q = useQuery({
    // queryKey に submitted を含めることでタブ毎に結果がキャッシュされ、切替で即再表示される。
    queryKey: ["analyze", kind, submitted],
    enabled: submitted.length > 0,
    queryFn: async () => {
      const { data, error } = await api.GET(path, {
        params: { query: { name: submitted } },
      });
      if (error) throw new Error("統計の取得に失敗しました（名前を確認）");
      return data;
    },
  });

  return (
    <div>
      <form
        className="toolbar"
        onSubmit={(e) => {
          e.preventDefault();
          onSubmit(input.trim());
        }}
      >
        {/* 名前検索は完全一致（部分一致・カタカナ正規化は #50・API 側対応待ち）。 */}
        <input
          type="text"
          placeholder={`${KIND_LABEL[kind]}名（完全一致）`}
          value={input}
          onChange={(e) => onInput(e.target.value)}
        />
        <button type="submit">検索</button>
      </form>

      {q.isPending && submitted.length > 0 && <p>読み込み中…</p>}
      {q.isError && <p className="error">{q.error.message}</p>}
      {q.data && (
        <>
          <h2>
            {"horse_name" in q.data
              ? q.data.horse_name
              : "jockey_name" in q.data
                ? q.data.jockey_name
                : q.data.trainer_name}
          </h2>
          <StatTable title="通算" rows={[q.data.overall]} />
          {"by_surface" in q.data && (
            <StatTable title="芝ダ別" rows={q.data.by_surface} />
          )}
          {"by_distance_band" in q.data && (
            <StatTable title="距離帯別" rows={q.data.by_distance_band} />
          )}
          {"by_popularity_band" in q.data && (
            <StatTable title="人気帯別" rows={q.data.by_popularity_band} />
          )}
          {"by_track_condition" in q.data && (
            <StatTable title="馬場状態別" rows={q.data.by_track_condition} />
          )}
          <StatTable title="枠順別" rows={q.data.by_gate_group} />
        </>
      )}
    </div>
  );
}

function CourseAnalyze({
  form,
  submitted,
  onForm,
  onSubmit,
}: {
  form: CourseParams;
  submitted: CourseParams | null;
  onForm: (f: CourseParams) => void;
  onSubmit: (f: CourseParams) => void;
}) {
  const q = useQuery({
    queryKey: ["analyze", "course", submitted],
    enabled: submitted !== null,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/analyze/course", {
        params: {
          query: {
            venue: submitted!.venue,
            distance: Number(submitted!.distance),
            surface: submitted!.surface,
          },
        },
      });
      if (error) throw new Error("コース統計の取得に失敗しました");
      return data;
    },
  });

  return (
    <div>
      <form
        className="toolbar"
        onSubmit={(e) => {
          e.preventDefault();
          onSubmit(form);
        }}
      >
        {/* 会場は VENUE_JP マスタのセレクト（value=slug, label=日本語）。slug 手入力を廃止（#384）。 */}
        <select
          value={form.venue}
          onChange={(e) => onForm({ ...form, venue: e.target.value })}
        >
          <option value="">開催場を選択</option>
          {VENUE_SLUGS.map((slug) => (
            <option key={slug} value={slug}>
              {VENUE_JP[slug]}
            </option>
          ))}
        </select>
        <input
          type="number"
          placeholder="距離[m]"
          min={1000}
          step={100}
          value={form.distance}
          onChange={(e) => onForm({ ...form, distance: e.target.value })}
        />
        <select
          value={form.surface}
          onChange={(e) =>
            onForm({ ...form, surface: e.target.value === "dirt" ? "dirt" : "turf" })
          }
        >
          <option value="turf">芝</option>
          <option value="dirt">ダート</option>
        </select>
        <button type="submit" disabled={!isVenueSlug(form.venue) || !form.distance}>
          検索
        </button>
      </form>

      {q.isPending && submitted && <p>読み込み中…</p>}
      {q.isError && <p className="error">{q.error.message}</p>}
      {q.data && (
        <>
          <h2>
            {VENUE_JP[q.data.venue] ?? q.data.venue} {q.data.distance}m{" "}
            {SURFACE_JP[q.data.surface] ?? q.data.surface}
          </h2>
          <StatTable title="枠順別" rows={q.data.by_gate_group} />
        </>
      )}
    </div>
  );
}
