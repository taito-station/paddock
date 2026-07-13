import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api, type GroupStat } from "../api/client";
import { pct, SURFACE_JP, todayJst } from "../lib/format";
import { isIsoDate, raceListHref } from "../lib/header-date";

type Kind = "horse" | "jockey" | "trainer" | "course";

const KIND_LABEL: Record<Kind, string> = {
  horse: "馬",
  jockey: "騎手",
  trainer: "調教師",
  course: "コース",
};

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

export function Analyze() {
  const [kind, setKind] = useState<Kind>("horse");
  const [searchParams] = useSearchParams();
  // 分析は全期間統計（date でフィルタしない）。ヘッダの開催日を戻る導線に引き継ぐだけ。
  // 不正な ?date= は当日へ倒し、ヘッダの currentHeaderDate と検証方針を揃える
  // （汚染 date を戻り導線→RaceList→API へ伝播させない）。
  const dateParam = searchParams.get("date");
  const date = isIsoDate(dateParam) ? dateParam : todayJst();

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
            onClick={() => setKind(k)}
          >
            {KIND_LABEL[k]}
          </button>
        ))}
      </div>
      {kind === "course" ? (
        <CourseAnalyze />
      ) : (
        // key で kind ごとにインスタンスを分け、タブ切替で入力・結果をリセットする。
        <NameAnalyze key={kind} kind={kind} />
      )}
    </section>
  );
}

function NameAnalyze({ kind }: { kind: "horse" | "jockey" | "trainer" }) {
  const [input, setInput] = useState("");
  const [name, setName] = useState("");

  const path = (
    {
      horse: "/api/analyze/horse",
      jockey: "/api/analyze/jockey",
      trainer: "/api/analyze/trainer",
    } as const
  )[kind];

  const q = useQuery({
    queryKey: ["analyze", kind, name],
    enabled: name.length > 0,
    queryFn: async () => {
      const { data, error } = await api.GET(path, {
        params: { query: { name } },
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
          setName(input.trim());
        }}
      >
        <input
          type="text"
          placeholder={`${KIND_LABEL[kind]}名（完全一致）`}
          value={input}
          onChange={(e) => setInput(e.target.value)}
        />
        <button type="submit">検索</button>
      </form>

      {q.isPending && name.length > 0 && <p>読み込み中…</p>}
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

function CourseAnalyze() {
  const [form, setForm] = useState({ venue: "", distance: "", surface: "turf" });
  const [submitted, setSubmitted] = useState<typeof form | null>(null);

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
          setSubmitted(form);
        }}
      >
        <input
          type="text"
          placeholder="開催場（例: nakayama）"
          value={form.venue}
          onChange={(e) => setForm({ ...form, venue: e.target.value })}
        />
        <input
          type="number"
          placeholder="距離[m]"
          min={1000}
          step={100}
          value={form.distance}
          onChange={(e) => setForm({ ...form, distance: e.target.value })}
        />
        <select
          value={form.surface}
          onChange={(e) => setForm({ ...form, surface: e.target.value })}
        >
          <option value="turf">芝</option>
          <option value="dirt">ダート</option>
        </select>
        <button type="submit" disabled={!form.venue || !form.distance}>
          検索
        </button>
      </form>

      {q.isPending && submitted && <p>読み込み中…</p>}
      {q.isError && <p className="error">{q.error.message}</p>}
      {q.data && (
        <>
          <h2>
            {q.data.venue} {q.data.distance}m{" "}
            {SURFACE_JP[q.data.surface] ?? q.data.surface}
          </h2>
          <StatTable title="枠順別" rows={q.data.by_gate_group} />
        </>
      )}
    </div>
  );
}
