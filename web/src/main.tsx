import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import { Layout } from "./Layout";
import { RaceList } from "./routes/RaceList";
import { Analyze } from "./routes/Analyze";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    // 永続データ表示が既定（自動ポーリングなし）。更新は明示操作のみ。
    queries: { refetchOnWindowFocus: false, retry: 1 },
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Routes>
          <Route element={<Layout />}>
            <Route index element={<RaceList />} />
            <Route path="analyze" element={<Analyze />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
);
