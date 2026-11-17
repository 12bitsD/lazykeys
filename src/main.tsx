import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// 前端进程入口：把主窗口编排层挂载到 Tauri 提供的 DOM 根节点。
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
