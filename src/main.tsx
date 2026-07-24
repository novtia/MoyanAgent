import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./i18n";
import "./store/chatFont";
import "./store/appearance";
import "./styles/globals.css";
import { installClipboardHistoryFix } from "./utils/clipboard";

installClipboardHistoryFix();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
