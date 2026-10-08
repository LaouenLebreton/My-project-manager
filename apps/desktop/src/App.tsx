import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, type Edit, type IpcError, type RecentFile, type Snapshot } from "./api";
import { formatDate } from "./format";
import { Gantt } from "./Gantt";
import { Grid, type GridHandle } from "./Grid";
import { ImportReport } from "./ImportReport";
import type { ReportView } from "./bindings/ReportView";

export function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [message, setMessage] = useState<string | null>(null);
  const [gridWidth, setGridWidth] = useState(720);
  const [timing, setTiming] = useState<string | null>(null);
  const gridRef = useRef<GridHandle>(null);

  const [recent, setRecent] = useState<RecentFile[]>([]);
  const [report, setReport] = useState<ReportView | null>(null);

  const run = useCallback(async (action: () => Promise<Snapshot | null>) => {
    try {
      const result = await action();
      if (result) setSnapshot(result); // null: the user cancelled a dialog
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  /** For the grid: errors are shown next to the cell, so they are passed back, not shown here. */
  const edit = useCallback(async (e: Edit) => {
    const t0 = performance.now();
    const result = await api.applyEdit(e);
    const t1 = performance.now();
    setSnapshot(result.snapshot);
    setError(null);
    if (import.meta.env.DEV) {
      // Development readout for the performance check: round trip, then until painted.
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          const t2 = performance.now();
          setTiming(`last edit: ${Math.round(t1 - t0)} ms round trip + ${Math.round(t2 - t1)} ms to draw, ${result.snapshot.rows.length} tasks`);
        }),
      );
    }
    return result;
  }, []);

  const startSplitDrag = (e: React.PointerEvent) => {
    const startX = e.clientX;
    const startWidth = gridWidth;
    const move = (ev: PointerEvent) => setGridWidth(Math.max(320, Math.min(window.innerWidth - 200, startWidth + ev.clientX - startX)));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const textEditing = useCallback((editing: boolean) => {
    void api.setTextEditing(editing);
  }, []);

  useEffect(() => {
    api.getSnapshot().then(setSnapshot, (e) => setError(errorMessage(e)));
    api.recentFiles().then(setRecent, () => {});
    const subscriptions = [
      // Undo and Redo from the Edit menu run in Rust; the result arrives here.
      listen<Snapshot>("document-changed", (e) => {
        setSnapshot(e.payload);
        setError(null);
      }),
      listen<IpcError>("error", (e) => setError(e.payload.message)),
      listen<ReportView>("import-report", (e) => setReport(e.payload)),
      listen("document-closed", () => {
        setSnapshot(null);
        api.recentFiles().then(setRecent, () => {});
      }),
      // While a cell is being edited, the menu's Undo and Redo apply to the typing.
      listen<string>("menu", (e) => {
        if (e.payload === "edit.undo") document.execCommand("undo");
        if (e.payload === "edit.redo") document.execCommand("redo");
      }),
    ];
    return () => {
      subscriptions.forEach((s) => s.then((unlisten) => unlisten()));
    };
  }, []);

  if (!snapshot) {
    // The start screen always offers the next step (spec 5.1.5).
    return (
      <main className="empty">
        <h1>Super PM</h1>
        <p>Plan a project with tasks, links and a calendar. Every change is saved as you go.</p>
        <div className="start-actions">
          <button className="primary" onClick={() => run(api.newProject)}>
            New Project…
          </button>
          <button onClick={() => run(api.openProject)}>Open…</button>
          <button onClick={() => run(api.openSample)}>Open Sample Project</button>
        </div>
        {recent.length > 0 && (
          <nav className="recent" aria-label="Recent projects">
            <h2>Recent</h2>
            <ul>
              {recent.map((f) => (
                <li key={f.path}>
                  <button className="link" title={f.path} onClick={() => run(() => api.openRecent(f.path))}>
                    {f.name}
                  </button>
                </li>
              ))}
            </ul>
          </nav>
        )}
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
      </main>
    );
  }

  return (
    <main className="document">
      <header className="toolbar">
        <div>
          <h1>{snapshot.project.name}</h1>
          <p className="secondary">
            {formatDate(snapshot.project.start)} – {formatDate(snapshot.project.finish)} · {snapshot.rows.length} tasks
          </p>
        </div>
        <p className="secondary hint">Return to edit · Tab to indent · Drag bars to move, their ends to stretch, onto another bar to link</p>
      </header>
      {error && (
        <p role="alert" className="banner error">
          {error}
        </p>
      )}
      {snapshot.problem && (
        <p role="alert" className="banner error">
          {snapshot.problem.message}
        </p>
      )}
      {snapshot.conflicts.map((c) => (
        <p key={c.taskId} role="status" className="banner warning">
          {snapshot.rows.find((r) => r.id === c.taskId)?.name}: {c.message}
        </p>
      ))}
      <div className="workspace">
        <div className="pane" style={{ width: gridWidth }}>
          <Grid
            ref={gridRef}
            snapshot={snapshot}
            onEdit={edit}
            onTextEditing={textEditing}
            onScrollTop={setScrollTop}
            onSelectionChange={setSelectedIds}
          />
        </div>
        <div
          className="splitter"
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize task list"
          onPointerDown={startSplitDrag}
        />
        <Gantt
          snapshot={snapshot}
          scrollTop={scrollTop}
          selectedIds={selectedIds}
          onEdit={edit}
          onSelectRow={(r) => gridRef.current?.selectRow(r)}
          onScrollBy={(dy) => gridRef.current?.scrollBy(dy)}
          onMessage={setMessage}
        />
      </div>
      {report && <ImportReport report={report} onClose={() => setReport(null)} />}
      <footer className="status" role="status" aria-live="polite">
        {message ?? (import.meta.env.DEV ? timing : null) ?? "\u00a0"}
      </footer>
    </main>
  );
}
