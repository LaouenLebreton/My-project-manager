import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, type Edit, type IpcError, type Snapshot } from "./api";
import { formatDate } from "./format";
import { Grid } from "./Grid";

export function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async (action: () => Promise<Snapshot>) => {
    try {
      setSnapshot(await action());
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  /** For the grid: errors are shown next to the cell, so they are passed back, not shown here. */
  const edit = useCallback(async (e: Edit) => {
    const result = await api.applyEdit(e);
    setSnapshot(result.snapshot);
    setError(null);
    return result;
  }, []);

  const textEditing = useCallback((editing: boolean) => {
    void api.setTextEditing(editing);
  }, []);

  useEffect(() => {
    api.getSnapshot().then(setSnapshot, (e) => setError(errorMessage(e)));
    const subscriptions = [
      // Undo and Redo from the Edit menu run in Rust; the result arrives here.
      listen<Snapshot>("document-changed", (e) => {
        setSnapshot(e.payload);
        setError(null);
      }),
      listen<IpcError>("error", (e) => setError(e.payload.message)),
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
    return (
      <main className="empty">
        <h1>No project open</h1>
        <p>Open the sample project to see how scheduling works.</p>
        <button className="primary" onClick={() => run(api.openSample)}>
          Open Sample Project
        </button>
        {error && <p role="alert" className="error">{error}</p>}
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
        <p className="secondary hint">Return to edit · Tab to indent · Type numbers in Predecessors to link</p>
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
      <Grid snapshot={snapshot} onEdit={edit} onTextEditing={textEditing} />
    </main>
  );
}
