import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { errorMessage, type Edit, type Snapshot } from "./api";
import type { EditResult } from "./bindings/EditResult";
import type { Row } from "./bindings/Row";
import { formatDate, formatDateTime, formatDays } from "./format";

const ROW_HEIGHT = 26;
const OVERSCAN = 8;

interface Column {
  label: string;
  width: string;
  align?: "right";
  /** Text shown in the cell. */
  text: (r: Row, perDay: number) => string;
  /** Text placed in the editor, and the edit to send. Absent for read-only columns. */
  edit?: { initial: (r: Row) => string; make: (r: Row, text: string) => Edit };
  title?: (r: Row) => string;
}

const COLUMNS: Column[] = [
  { label: "#", width: "44px", align: "right", text: () => "" },
  { label: "", width: "22px", text: () => "" },
  {
    label: "Task Name",
    width: "minmax(220px, 1fr)",
    text: (r) => r.name,
    edit: { initial: (r) => r.name, make: (r, text) => ({ kind: "RenameTask", id: r.id, name: text }) },
  },
  {
    label: "Duration",
    width: "84px",
    align: "right",
    text: (r, perDay) => (r.isMilestone ? "0 days" : formatDays(r.durationMinutes, perDay)),
    edit: { initial: (r) => r.durationText, make: (r, text) => ({ kind: "SetDuration", id: r.id, text }) },
  },
  { label: "Start", width: "128px", text: (r) => formatDate(r.start), title: (r) => formatDateTime(r.start) },
  { label: "Finish", width: "128px", text: (r) => formatDate(r.finish), title: (r) => formatDateTime(r.finish) },
  {
    label: "Predecessors",
    width: "116px",
    text: (r) => r.predecessors,
    edit: { initial: (r) => r.predecessors, make: (r, text) => ({ kind: "SetPredecessors", id: r.id, text }) },
  },
  { label: "Slack", width: "76px", align: "right", text: (r, perDay) => formatDays(r.totalSlackMinutes, perDay) },
];
const NAME_COL = 2;
const TEMPLATE = COLUMNS.map((c) => c.width).join(" ");

export interface GridProps {
  snapshot: Snapshot;
  /** Resolves with the result, or rejects with a message to show next to the cell. */
  onEdit: (edit: Edit) => Promise<EditResult>;
  onTextEditing: (editing: boolean) => void;
}

interface Editing {
  text: string;
  error: string | null;
}

/**
 * The task grid. Keyboard first: arrows move, Return edits and saves, Escape cancels,
 * Tab / Shift-Tab indent and outdent, Delete removes rows, typing starts editing.
 * Only visible rows are rendered, so thousands of tasks scroll smoothly.
 */
export function Grid({ snapshot, onEdit, onTextEditing }: GridProps) {
  const rows = snapshot.rows;
  const rowCount = rows.length + 1; // plus the blank row for new tasks
  const perDay = snapshot.project.minutesPerDay;

  const [active, setActive] = useState({ row: 0, col: NAME_COL });
  const [anchor, setAnchor] = useState(0);
  const [editing, setEditingState] = useState<Editing | null>(null);
  // The editor is saved from several events (Return, blur, clicking elsewhere); the ref makes
  // sure each edit is sent exactly once, even before React re-renders.
  const editingRef = useRef<Editing | null>(null);
  const setEditing = (e: Editing | null) => {
    editingRef.current = e;
    setEditingState(e);
  };
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(600);
  const bodyRef = useRef<HTMLDivElement>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const busy = useRef(false);

  const row = Math.min(active.row, rowCount - 1);
  const current: Row | undefined = rows[row];
  const selection = (() => {
    const [a, b] = [Math.min(anchor, row), Math.max(anchor, row)];
    return rows.slice(a, Math.min(b, rows.length - 1) + 1);
  })();
  const selectedIds = new Set(selection.map((r) => r.id));
  const flagged = new Set([...(snapshot.problem?.taskIds ?? []), ...snapshot.conflicts.map((c) => c.taskId)]);

  useEffect(() => onTextEditing(editing !== null), [editing !== null, onTextEditing]); // eslint-disable-line react-hooks/exhaustive-deps

  useLayoutEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setViewport(el.clientHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Keep the active row visible.
  useEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const top = row * ROW_HEIGHT;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + ROW_HEIGHT > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW_HEIGHT - el.clientHeight;
  }, [row]);

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => gridRef.current?.focus(), []);

  const focusGrid = () => gridRef.current?.focus();

  const moveTo = useCallback(
    (r: number, c: number, extend = false) => {
      const nr = Math.max(0, Math.min(rowCount - 1, r));
      setActive({ row: nr, col: Math.max(0, Math.min(COLUMNS.length - 1, c)) });
      if (!extend) setAnchor(nr);
    },
    [rowCount],
  );

  const send = async (edit: Edit): Promise<string | null> => {
    if (busy.current) return null; // one edit at a time; extra key presses are ignored
    busy.current = true;
    try {
      const result = await onEdit(edit);
      if (result.createdId) {
        const at = result.snapshot.rows.findIndex((r) => r.id === result.createdId);
        if (at >= 0) moveTo(at, active.col);
      }
      return null;
    } catch (e) {
      return errorMessage(e);
    } finally {
      busy.current = false;
    }
  };

  const startEditing = (initial?: string) => {
    const col = COLUMNS[active.col];
    if (current && col.edit) {
      setEditing({ text: initial ?? col.edit.initial(current), error: null });
    } else if (!current && active.col === NAME_COL) {
      setEditing({ text: initial ?? "", error: null }); // blank row: typing a name adds a task
    }
  };

  /** Saves the editor. Returns false (and keeps editing) if the edit was refused. */
  const commit = async (): Promise<boolean> => {
    const ed = editingRef.current;
    if (!ed) return true;
    editingRef.current = null;
    const text = ed.text;
    const col = COLUMNS[active.col];
    let edit: Edit | null = null;
    if (current && col.edit) {
      if (text !== col.edit.initial(current)) edit = col.edit.make(current, text);
    } else if (!current && text.trim() !== "") {
      edit = { kind: "AddTask", afterId: rows.at(-1)?.id ?? null, name: text.trim() };
    }
    if (edit) {
      const error = await send(edit);
      if (error) {
        setEditing({ text, error });
        return false;
      }
    }
    setEditing(null);
    focusGrid();
    return true;
  };

  const cancel = () => {
    setEditing(null);
    focusGrid();
  };

  const indent = (outdent: boolean) => {
    const ids = selection.length ? selection.map((r) => r.id) : current ? [current.id] : [];
    if (ids.length) void send({ kind: outdent ? "Outdent" : "Indent", ids });
  };

  const onGridKey = (e: React.KeyboardEvent) => {
    if (editing) return;
    const page = Math.max(1, Math.floor(viewport / ROW_HEIGHT) - 1);
    const shift = e.shiftKey;
    switch (e.key) {
      case "ArrowUp":
        moveTo(e.metaKey ? 0 : row - 1, active.col, shift);
        break;
      case "ArrowDown":
        moveTo(e.metaKey ? rowCount - 1 : row + 1, active.col, shift);
        break;
      case "ArrowLeft":
        moveTo(row, e.metaKey ? 0 : active.col - 1);
        break;
      case "ArrowRight":
        moveTo(row, e.metaKey ? COLUMNS.length - 1 : active.col + 1);
        break;
      case "PageUp":
        moveTo(row - page, active.col, shift);
        break;
      case "PageDown":
        moveTo(row + page, active.col, shift);
        break;
      case "Home":
        moveTo(0, active.col, shift);
        break;
      case "End":
        moveTo(rowCount - 1, active.col, shift);
        break;
      case "Enter":
        startEditing();
        break;
      case "Tab":
        indent(shift);
        break;
      case "Backspace":
      case "Delete":
        if (selection.length) void send({ kind: "DeleteTasks", ids: selection.map((r) => r.id) });
        break;
      case "Escape":
        setAnchor(row);
        break;
      default:
        if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
          startEditing(e.key);
          break;
        }
        return; // let the browser and menus handle everything else
    }
    e.preventDefault();
  };

  const onEditorKey = async (e: React.KeyboardEvent<HTMLInputElement>) => {
    e.stopPropagation();
    switch (e.key) {
      case "Enter":
        e.preventDefault();
        if (await commit()) moveTo(row + 1, active.col);
        break;
      case "Escape":
        e.preventDefault();
        cancel();
        break;
      case "Tab":
        e.preventDefault();
        if (await commit()) indent(e.shiftKey);
        break;
      case "ArrowUp":
      case "ArrowDown":
        e.preventDefault();
        if (await commit()) moveTo(row + (e.key === "ArrowUp" ? -1 : 1), active.col);
        break;
    }
  };

  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const last = Math.min(rowCount, Math.ceil((scrollTop + viewport) / ROW_HEIGHT) + OVERSCAN);
  const visible: number[] = [];
  for (let i = first; i < last; i++) visible.push(i);

  return (
    <div
      ref={gridRef}
      className="grid"
      role="grid"
      aria-label="Tasks"
      aria-rowcount={rowCount + 1}
      aria-colcount={COLUMNS.length}
      aria-multiselectable
      aria-activedescendant={`cell-${row}-${active.col}`}
      tabIndex={0}
      onKeyDown={onGridKey}
    >
      <div className="grid-header" role="row" aria-rowindex={1} style={{ gridTemplateColumns: TEMPLATE }}>
        {COLUMNS.map((c, i) => (
          <div key={i} role="columnheader" className={c.align === "right" ? "right" : undefined}>
            {c.label}
          </div>
        ))}
      </div>
      <div ref={bodyRef} className="grid-body" onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}>
        <div style={{ height: rowCount * ROW_HEIGHT, position: "relative" }}>
          {visible.map((i) => {
            const r = rows[i];
            const selected = r ? selectedIds.has(r.id) : false;
            const classes = ["grid-row", i % 2 ? "odd" : "", r?.isSummary ? "summary" : "", selected ? "selected" : "", r && flagged.has(r.id) ? "flagged" : ""];
            return (
              <div
                key={r?.id ?? "new"}
                role="row"
                aria-rowindex={i + 2}
                aria-selected={selected}
                className={classes.filter(Boolean).join(" ")}
                style={{ top: i * ROW_HEIGHT, height: ROW_HEIGHT, gridTemplateColumns: TEMPLATE }}
              >
                {COLUMNS.map((c, ci) => {
                  const isActive = i === row && ci === active.col;
                  const isEditing = isActive && editing !== null;
                  return (
                    <div
                      key={ci}
                      id={`cell-${i}-${ci}`}
                      role="gridcell"
                      aria-readonly={!c.edit}
                      className={["cell", c.align === "right" ? "right" : "", isActive ? "active" : ""].filter(Boolean).join(" ")}
                      title={r && c.title ? c.title(r) : undefined}
                      onMouseDown={(e) => {
                        if (editing && !isEditing) void commit();
                        moveTo(i, ci, e.shiftKey);
                      }}
                      onDoubleClick={() => startEditing()}
                      style={ci === NAME_COL && r ? { paddingLeft: 6 + r.depth * 18 } : undefined}
                    >
                      {isEditing ? (
                        <>
                          <input
                            ref={inputRef}
                            className="editor"
                            aria-label={c.label || "Task"}
                            aria-invalid={editing.error !== null}
                            value={editing.text}
                            onChange={(e) => setEditing({ text: e.target.value, error: null })}
                            onKeyDown={onEditorKey}
                            onBlur={() => void commit()}
                          />
                          {editing.error && (
                            <div role="alert" className="cell-error">
                              {editing.error}
                            </div>
                          )}
                        </>
                      ) : (
                        cellContent(r, i, ci, c, perDay)
                      )}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function cellContent(r: Row | undefined, i: number, ci: number, c: Column, perDay: number) {
  if (!r) {
    return ci === NAME_COL ? <span className="placeholder">Type a task name</span> : null;
  }
  if (ci === 0) return <span className="secondary">{i + 1}</span>;
  if (ci === 1) {
    // A marker and a label, not colour alone (spec 5.1.9).
    return r.critical ? (
      <span className="critical" role="img" aria-label="Critical" title="Critical: any delay moves the project finish">
        ▲
      </span>
    ) : null;
  }
  if (ci === NAME_COL && r.isMilestone) {
    return (
      <>
        <span aria-label="Milestone" title="Milestone">◆ </span>
        {r.name}
      </>
    );
  }
  return c.text(r, perDay);
}
