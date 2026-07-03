import { NavLink, Outlet } from "react-router-dom";
import { todayJst } from "./lib/format";

export function Layout() {
  return (
    <div className="app">
      <header className="app-header">
        <h1>paddock</h1>
        <nav>
          <NavLink to="/" end>
            レース
          </NavLink>
          <NavLink to={`/live/${todayJst()}`}>ライブ</NavLink>
          <NavLink to="/analyze">分析</NavLink>
        </nav>
      </header>
      <main className="app-main">
        <Outlet />
      </main>
    </div>
  );
}
