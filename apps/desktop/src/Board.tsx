import { useMemo, useRef, useState } from "react";
import { errorMessage, type Edit, type Snapshot } from "./api";
import type { EditResult } from "./bindings/EditResult";
import type { Row } from "./bindings/Row";
import { formatDate } from "./format";
import { GLOSSARY } from "./help";
import { blocked } from "./permissions";
import { useContextMenu } from "./ContextMenu";

/** Microsoft Project's Task Board columns, tied to % complete as it does. */
const COLUMNS = [
  { key: "todo", title: "Not started", percent: 0 },
  { key: "doing", title: "In progress", percent: 50 },
  { key: "done", title: "Done", percent: 100 },
] as const;
type ColumnKey = (typeof COLUMNS)[number]["key"];

const columnOf = (r: Row): ColumnKey => (r.percentComplete >= 100 ? "done" : r.percentComplete > 0 ? "doing" : "todo");

/** Large projects show this many cards per column until asked for more. */
const PAGE = 200;

/**
 * Board: work tasks as cards in Not started / In progress / Done. Dragging a card (or its
 * arrow buttons) sets % complete, which records the actual dates as the task list does.
 */
export function Board({
  snapshot,
  onEdit,
  onShowTask,
  onTaskInfo,
}: {
  snapshot: Snapshot;
  onEdit: (edit: Edit) => Promise<EditResult>;
  onShowTask: (row: number) => void;
  onTaskInfo?: (r: Row) => void;
}) {
  const openMenu = useContextMenu();
  const [phase, setPhase] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  // Dragging uses pointer events: the webview's own drag and drop is reserved for files.
  type Drag = { id: string; x: number; y: number; dx: number; dy: number; moved: boolean; left: number; top: number; width: number };
  const [drag, setDragState] = useState<Drag | null>(null);
  // Handlers read the ref: a quick drag can end before React re-renders.
  const dragRef = useRef<Drag | null>(null);
  const justDragged = useRef(false);
  const setDrag = (d: Drag | null) => {
    dragRef.current = d;
    setDragState(d);
  };
  const [over, setOver] = useState<ColumnKey | null>(null);
  const columnRefs = useRef(new Map<ColumnKey, HTMLElement>());
  const columnAt = (x: number, y: number): ColumnKey | null => {
    for (const [key, el] of columnRefs.current) {
      const b = el.getBoundingClientRect();
      if (x >= b.left && x <= b.right && y >= b.top && y <= b.bottom) return key;
    }
    return null;
  };
  const [limit, setLimit] = useState<Record<ColumnKey, number>>({ todo: PAGE, doing: PAGE, done: PAGE });

  const rowNumber = useMemo(() => new Map(snapshot.rows.map((r, i) => [r.id, i + 1])), [snapshot.rows]);
  const byId = useMemo(() => new Map(snapshot.rows.map((r) => [r.id, r])), [snapshot.rows]);
  const phases = snapshot.rows.filter((r) => r.isSummary && r.depth === 0);
  /** The top-level phase a task belongs to. */
  const phaseOf = (r: Row): Row | undefined => {
    let at: Row | undefined = r;
    while (at?.parentId) at = byId.get(at.parentId);
    return at && at !== r ? at : undefined;
  };
  const cards = snapshot.rows.filter((r) => !r.isSummary && (!phase || phaseOf(r)?.id === phase));

  const move = (r: Row, to: ColumnKey) => {
    if (columnOf(r) === to) return;
    const percent = COLUMNS.find((c) => c.key === to)!.percent;
    const why = blocked(snapshot, "progress", r);
    if (why) {
      setError(why);
      return;
    }
    setError(null);
    onEdit({ kind: "SetPercentComplete", ids: [r.id], percent }).catch((e) => setError(errorMessage(e)));
  };

  return (
    <div className="board">
      <div className="workload-bar">
        <p className="secondary small">{GLOSSARY.board.text}</p>
        {phases.length > 0 && (
          <label className="inline">
            Phase{" "}
            <select value={phase} onChange={(e) => setPhase(e.target.value)}>
              <option value="">All</option>
              {phases.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>
      {error && (
        <p role="alert" className="banner error">
          {error}
        </p>
      )}
      {drag?.moved && (
        // Drawn above everything, so it is not clipped by the scrolling columns.
        <div className="card ghost" aria-hidden style={{ left: drag.left + drag.dx, top: drag.top + drag.dy, width: drag.width }}>
          <strong>{byId.get(drag.id)?.name}</strong>
        </div>
      )}
      <div className="board-columns">
        {COLUMNS.map((col, ci) => {
          const list = cards.filter((r) => columnOf(r) === col.key);
          return (
            <section
              key={col.key}
              className={over === col.key ? "board-column drop" : "board-column"}
              aria-label={col.title}
              ref={(el) => {
                if (el) columnRefs.current.set(col.key, el);
                else columnRefs.current.delete(col.key);
              }}
            >
              <h3>
                {col.title} <span className="secondary">{list.length}</span>
              </h3>
              <ul>
                {list.slice(0, limit[col.key]).map((r) => {
                  const ph = phaseOf(r);
                  return (
                    <li
                      key={r.id}
                      className={["card", r.critical && col.key !== "done" ? "critical" : "", drag?.id === r.id && drag.moved ? "lifted" : ""].filter(Boolean).join(" ")}
                      onPointerDown={(e) => {
                        if (e.button !== 0 || (e.target as HTMLElement).closest(".card-moves")) return;
                        const b = e.currentTarget.getBoundingClientRect();
                        setDrag({ id: r.id, x: e.clientX, y: e.clientY, dx: 0, dy: 0, moved: false, left: b.left, top: b.top, width: b.width });
                        // Follow the pointer anywhere on the page until it is released.
                        const onMove = (ev: PointerEvent) => {
                          const d = dragRef.current;
                          if (!d) return;
                          const dx = ev.clientX - d.x;
                          const dy = ev.clientY - d.y;
                          const moved = d.moved || Math.hypot(dx, dy) > 5;
                          setDrag({ ...d, dx, dy, moved });
                          if (moved) setOver(columnAt(ev.clientX, ev.clientY));
                        };
                        const onUp = (ev: PointerEvent) => {
                          window.removeEventListener("pointermove", onMove);
                          window.removeEventListener("pointerup", onUp);
                          window.removeEventListener("pointercancel", onUp);
                          const d = dragRef.current;
                          // Releasing far from where it started still counts as a drag.
                          const moved = !!d && (d.moved || Math.hypot(ev.clientX - d.x, ev.clientY - d.y) > 5);
                          justDragged.current = moved;
                          if (moved && ev.type === "pointerup") {
                            const to = columnAt(ev.clientX, ev.clientY);
                            if (to) move(r, to);
                          }
                          setDrag(null);
                          setOver(null);
                        };
                        window.addEventListener("pointermove", onMove);
                        window.addEventListener("pointerup", onUp);
                        window.addEventListener("pointercancel", onUp);
                      }}
                      onContextMenu={(e) =>
                        openMenu(e, [
                          ...COLUMNS.map((c) => ({ label: `Move to ${c.title}`, checked: c.key === col.key, run: () => move(r, c.key) })),
                          { separator: true },
                          { label: "Show in Gantt Chart", run: () => onShowTask(rowNumber.get(r.id) ?? 1) },
                          { label: "Task Info…", run: () => onTaskInfo?.(r) },
                          { separator: true },
                          { label: "Delete Task", danger: true, run: () => void onEdit({ kind: "DeleteTasks", ids: [r.id] }).catch((err) => setError(errorMessage(err))) },
                        ])
                      }
                      onDoubleClick={() => onTaskInfo?.(r)}
                      onClickCapture={(e) => {
                        // A drag ends with a click on the card; it should not open the task.
                        if (justDragged.current) e.stopPropagation();
                        justDragged.current = false;
                      }}
                    >
                      <button className="link card-name" onClick={() => onShowTask(rowNumber.get(r.id) ?? 1)} title="Show in the Gantt Chart">
                        {r.isMilestone && "◆ "}
                        {r.name}
                      </button>
                      {r.critical && col.key !== "done" && <span className="critical-tag"> critical</span>}
                      <div className="secondary small">
                        {rowNumber.get(r.id)}
                        {ph && !phase ? ` · ${ph.name}` : ""} · {formatDate(r.start)} – {formatDate(r.finish)}
                      </div>
                      {r.resourceNames && <div className="small">{r.resourceNames}</div>}
                      {col.key === "doing" && (
                        <div className="card-progress" role="progressbar" aria-valuenow={r.percentComplete} aria-valuemin={0} aria-valuemax={100} aria-label="% complete">
                          <span style={{ width: `${r.percentComplete}%` }} />
                        </div>
                      )}
                      <div className="card-moves">
                        {ci > 0 && (
                          <button onClick={() => move(r, COLUMNS[ci - 1].key)} aria-label={`Move “${r.name}” to ${COLUMNS[ci - 1].title}`} title={`Move to ${COLUMNS[ci - 1].title}`}>
                            ←
                          </button>
                        )}
                        {ci < COLUMNS.length - 1 && (
                          <button onClick={() => move(r, COLUMNS[ci + 1].key)} aria-label={`Move “${r.name}” to ${COLUMNS[ci + 1].title}`} title={`Move to ${COLUMNS[ci + 1].title}`}>
                            →
                          </button>
                        )}
                      </div>
                    </li>
                  );
                })}
              </ul>
              {list.length > limit[col.key] && (
                <button className="link" onClick={() => setLimit((l) => ({ ...l, [col.key]: l[col.key] + PAGE }))}>
                  Show {Math.min(PAGE, list.length - limit[col.key])} more
                </button>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}
