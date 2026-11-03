import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { TabView } from "./bindings/TabView";

/**
 * The tab line at the top of every window: each open project (and Home), with a × to close it
 * and the + at the end, as in Safari. Drawn by the app because macOS would not show its own +
 * for these windows; the projects still share one window as macOS tabs underneath.
 */
export function TabStrip({ refreshKey }: { refreshKey?: unknown }) {
  const [tabs, setTabs] = useState<TabView[]>([]);
  const load = useCallback(() => {
    invoke<TabView[]>("tab_list").then(setTabs, () => {});
  }, []);
  useEffect(() => {
    load();
    const off = listen("tabs-changed", load);
    return () => void off.then((u) => u());
  }, [load]);
  // A project renamed in this tab: its title changes.
  useEffect(load, [refreshKey, load]);

  const select = (label: string) => void invoke("select_tab", { label }).catch(() => {});
  const close = (label: string) => void invoke("close_tab", { label }).catch(() => {});
  const add = () => void invoke("show_home", { newTab: true }).catch(() => {});

  return (
    <nav className="tab-strip" aria-label="Open projects">
      {tabs.map((t) => (
        <div key={t.label} className={t.active ? "app-tab active" : "app-tab"}>
          <button className="app-tab-title" onClick={() => select(t.label)} aria-current={t.active} title={t.title}>
            {t.title === "Home" ? "⌂ Home" : t.title}
            {t.shared && <span className="secondary"> · team</span>}
          </button>
          <button className="app-tab-close" onClick={() => close(t.label)} aria-label={`Close ${t.title}`} title="Close tab (⌘W)">
            ×
          </button>
        </div>
      ))}
      <button className="app-tab-add" onClick={add} aria-label="New tab" title="New tab (⌘T)">
        +
      </button>
    </nav>
  );
}
