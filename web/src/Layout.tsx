import { NavLink, Outlet } from "react-router-dom";

export function Layout() {
  return (
    <div className="app">
      <header className="app-header">
        <h1>paddock</h1>
        <nav>
          {/* ライブは一覧に統合（#378）。レース＝日次ダッシュボード。 */}
          <NavLink to="/" end>
            レース
          </NavLink>
          <NavLink to="/analyze">分析</NavLink>
        </nav>
      </header>
      <main className="app-main">
        <Outlet />
      </main>
    </div>
  );
}
