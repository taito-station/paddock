import {
  NavLink,
  Outlet,
  useLocation,
  useNavigate,
  useSearchParams,
} from "react-router-dom";
import {
  analyzeHref,
  currentHeaderDate,
  isIsoDate,
  raceListHref,
} from "./lib/header-date";

export function Layout() {
  const [searchParams] = useSearchParams();
  const { pathname } = useLocation();
  const navigate = useNavigate();
  // 開催日コンテキストをヘッダで一度だけ解決し、全画面へ引き継ぐ（#379）。
  const date = currentHeaderDate(searchParams, pathname);

  return (
    <div className="app">
      <header className="app-header">
        <h1>paddock</h1>
        <nav>
          {/* ライブは一覧に統合（#378）。レース＝日次ダッシュボード。 */}
          <NavLink to={raceListHref(date)} end>
            レース
          </NavLink>
          <NavLink to={analyzeHref(date)}>分析</NavLink>
        </nav>
        <label className="date-picker">
          開催日{" "}
          <input
            type="date"
            value={date}
            onChange={(e) => {
              const v = e.target.value;
              // input[type=date] は空 or YYYY-MM-DD。日付変更は常にレース一覧へ遷移。
              // 空クリア時は何もしない（当日に戻すには別日を選び直す）。
              if (isIsoDate(v)) navigate(raceListHref(v));
            }}
          />
        </label>
      </header>
      <main className="app-main">
        <Outlet />
      </main>
    </div>
  );
}
