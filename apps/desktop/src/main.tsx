import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { PortfolioView } from "./PortfolioView";
import { ContextMenuProvider } from "./ContextMenu";
import "./styles.css";

// In a plain browser during development, stand in for the Rust backend (see dev/mockBackend.ts).
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  (await import("./dev/mockBackend")).install();
}

/** The portfolio window (Window > Portfolio) shows the portfolio; every other window, a project.
 * In the browser, `?portfolio` stands in for it. */
function isPortfolioWindow(): boolean {
  const meta = (window as unknown as { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } }).__TAURI_INTERNALS__?.metadata;
  return meta?.currentWindow?.label === "portfolio" || new URLSearchParams(location.search).has("portfolio");
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ContextMenuProvider>{isPortfolioWindow() ? <PortfolioView /> : <App />}</ContextMenuProvider>
  </StrictMode>,
);
