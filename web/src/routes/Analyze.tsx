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
  completeCourse,
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

// name 系タブの lift 状態。submitted=確定した検索語（部分一致）、selected=一覧から確定した名前。
// 同一 NameAnalyze インスタンスがタブ間で使い回されるため、選択は kind 別に親が保持する（#401）。
type NameSlot = { input: string; submitted: string; selected: string };
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
  // 初期値は URL からアクティブタブ分を hydrate する（リロード/共有復元）。hydrate は初回のみ
  // で、以降は searchParams 変化に追従しない（全 setSearchParams が replace 運用のため
  // back/forward で analyze 内の state 履歴は生成されない。kind だけは毎レンダー URL 由来）。
  const [names, setNames] = useState<NameState>(() => {
    const init = parseAnalyzeParams(searchParams);
    const q = init.kind !== "course" ? init.name : "";
    const slot = (k: NameKind): NameSlot =>
      init.kind === k
        ? { input: q, submitted: q, selected: "" }
        : { input: "", submitted: "", selected: "" };
    return { horse: slot("horse"), jockey: slot("jockey"), trainer: slot("trainer") };
  });
  const [course, setCourse] = useState<CourseState>(() => {
    const init = parseAnalyzeParams(searchParams);
    return {
      form: init.course,
      submitted: init.kind === "course" ? completeCourse(init.course) : null,
    };
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
  // submit 系は URL 反映（writeUrl）に確定後の値を同期的に渡す必要があるため、
  // クロージャの現在値から next を組んで state と URL を一度に更新する。
  // 新しい検索語では前回の候補選択（selected）を破棄する。
  const onNameSubmit = (k: NameKind, submitted: string) => {
    const next: NameState = { ...names, [k]: { input: submitted, submitted, selected: "" } };
    setNames(next);
    writeUrl(k, next, course);
  };
  // 候補一覧からの確定名（部分一致で複数ヒットしたときの選択）。URL は検索語のみ持つため反映しない。
  const onNameSelect = (k: NameKind, selected: string) =>
    setNames((s) => ({ ...s, [k]: { ...s[k], selected } }));

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
          selected={names[kind].selected}
          onInput={(v) => onNameInput(kind, v)}
          onSubmit={(v) => onNameSubmit(kind, v)}
          onSelect={(v) => onNameSelect(kind, v)}
        />
      )}
    </section>
  );
}

// 名前系タブの API パス。部分一致は `/candidates?q=`（#401）、確定名の統計は `?name=`（完全一致）。
const NAME_PATHS = {
  horse: { candidates: "/api/analyze/horse/candidates", stats: "/api/analyze/horse" },
  jockey: { candidates: "/api/analyze/jockey/candidates", stats: "/api/analyze/jockey" },
  trainer: { candidates: "/api/analyze/trainer/candidates", stats: "/api/analyze/trainer" },
} as const;

function NameAnalyze({
  kind,
  input,
  submitted,
  selected,
  onInput,
  onSubmit,
  onSelect,
}: {
  kind: NameKind;
  input: string;
  submitted: string;
  selected: string;
  onInput: (v: string) => void;
  onSubmit: (v: string) => void;
  onSelect: (v: string) => void;
}) {
  const paths = NAME_PATHS[kind];

  // 部分一致の候補（#401）。中間一致・カナ正規化は取り込み時と共有（#50）。
  const cand = useQuery({
    queryKey: ["analyze-candidates", kind, submitted],
    enabled: submitted.length > 0,
    queryFn: async () => {
      const { data, error } = await api.GET(paths.candidates, {
        params: { query: { q: submitted } },
      });
      if (error) throw new Error("候補の取得に失敗しました");
      return data;
    },
  });

  // 確定名: 明示選択 > 候補が 1 件だけならそれを自動確定（CLI の 0/1/多数 と同じ挙動）。
  const names = cand.data?.names ?? [];
  const resolved = selected || (names.length === 1 ? names[0] : "");

  // 確定名の統計（完全一致 stats を再利用）。
  const stats = useQuery({
    queryKey: ["analyze", kind, resolved],
    enabled: resolved.length > 0,
    queryFn: async () => {
      const { data, error } = await api.GET(paths.stats, {
        params: { query: { name: resolved } },
      });
      if (error) throw new Error("統計の取得に失敗しました");
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
        {/* 部分一致・カナ正規化（#50 の normalizer を REST に露出・#401）。表記ゆれ/部分入力でも拾う。 */}
        <input
          type="text"
          placeholder={`${KIND_LABEL[kind]}名（部分一致）`}
          value={input}
          onChange={(e) => onInput(e.target.value)}
        />
        <button type="submit">検索</button>
      </form>

      {cand.isPending && submitted.length > 0 && <p>読み込み中…</p>}
      {cand.isError && <p className="error">{cand.error.message}</p>}
      {cand.data && names.length === 0 && (
        <p>「{submitted}」に一致する{KIND_LABEL[kind]}が見つかりません。</p>
      )}

      {/* 複数ヒット時は候補一覧を提示（クリックで確定）。1 件のみは自動確定するので一覧は出さない。
          選択後も一覧は畳まず統計と併存させる（別候補へ選び直せるように意図的に残す。選択中は
          candidate-active + aria-pressed で示す）。 */}
      {names.length > 1 && (
        <div className="candidates">
          <p>
            「{submitted}」に一致する{KIND_LABEL[kind]}が {names.length}
            {cand.data?.truncated ? " 件以上" : " 件"}あります。選択してください:
          </p>
          <ul className="candidate-list">
            {names.map((n) => (
              <li key={n}>
                <button
                  type="button"
                  className={n === selected ? "candidate candidate-active" : "candidate"}
                  aria-pressed={n === selected}
                  onClick={() => onSelect(n)}
                >
                  {n}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {stats.isPending && resolved.length > 0 && <p>読み込み中…</p>}
      {stats.isError && <p className="error">{stats.error.message}</p>}
      {stats.data && (
        <>
          <h2>
            {"horse_name" in stats.data
              ? stats.data.horse_name
              : "jockey_name" in stats.data
                ? stats.data.jockey_name
                : stats.data.trainer_name}
          </h2>
          <StatTable title="通算" rows={[stats.data.overall]} />
          {"by_surface" in stats.data && (
            <StatTable title="芝ダ別" rows={stats.data.by_surface} />
          )}
          {"by_distance_band" in stats.data && (
            <StatTable title="距離帯別" rows={stats.data.by_distance_band} />
          )}
          {"by_popularity_band" in stats.data && (
            <StatTable title="人気帯別" rows={stats.data.by_popularity_band} />
          )}
          {"by_track_condition" in stats.data && (
            <StatTable title="馬場状態別" rows={stats.data.by_track_condition} />
          )}
          <StatTable title="枠順別" rows={stats.data.by_gate_group} />
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
        <button type="submit" disabled={completeCourse(form) === null}>
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
