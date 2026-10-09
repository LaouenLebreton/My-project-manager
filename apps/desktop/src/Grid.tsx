import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";
import { errorMessage, type Edit, type Snapshot } from "./api";
import type { EditResult } from "./bindings/EditResult";
import type { Row } from "./bindings/Row";
import { formatDate, formatDateTime, formatDays, formatMoney } from "./format";
import { GLOSSARY, type Term } from "./help";
import { CONSTRAINTS } from "./Sheets";

import { HEADER_HEIGHT, ROW_HEIGHT } from "./Gantt";

const OVERSCAN = 8;

interface Column {
  label: string;
  width: string;
  align?: "right";
  /** Text shown in the cell. */
  text: (r: Row, perDay: number, currency: string) => string;
  /** Text placed in the editor, and the edit to send. Absent for read-only columns. */
  edit?: { initial: (r: Row) => string; make: (r: Row, text: string) => Edit };
  title?: (r: Row) => string;
  /** "What does this mean?" for the header. */
  help?: Term;
  /** Anchor for the first-run tour. */
  tour?: string;
}

const COLUMNS: Column[] = [
  { label: "#", width: "44px", align: "right", text: () => "" },
  { label: "", width: "40px", text: () => "" },
  {
    label: "Task Name",
    width: "minmax(160px, 1fr)",
    tour: "name",
    text: (r) => r.name,
    edit: { initial: (r) => r.name, make: (r, text) => ({ kind: "RenameTask", id: r.id, name: text }) },
  },
  {
    label: "Duration",
    width: "84px",
    align: "right",
    help: "duration",
    text: (r, perDay) => (r.isMilestone ? "0 days" : formatDays(r.durationMinutes, perDay)),
    edit: { initial: (r) => r.durationText, make: (r, text) => ({ kind: "SetDuration", id: r.id, text }) },
  },
  { label: "Start", width: "112px", text: (r) => formatDate(r.start), title: (r) => formatDateTime(r.start) },
  { label: "Finish", width: "112px", text: (r) => formatDate(r.finish), title: (r) => formatDateTime(r.finish) },
  {
    label: "% Done",
    width: "80px",
    align: "right",
    help: "progress",
    text: (r) => `${r.percentComplete}%`,
    title: (r) => (r.actualFinish ? `Finished ${formatDateTime(r.actualFinish)}` : r.actualStart ? `Started ${formatDateTime(r.actualStart)}` : "Not started"),
    edit: { initial: (r) => String(r.percentComplete), make: (r, text) => ({ kind: "SetPercentComplete", ids: [r.id], percent: parsePercent(text) }) },
  },
  {
    label: "Predecessors",
    width: "116px",
    help: "predecessors",
    tour: "predecessors",
    text: (r) => r.predecessors,
    edit: { initial: (r) => r.predecessors, make: (r, text) => ({ kind: "SetPredecessors", id: r.id, text }) },
  },
  {
    label: "Resources",
    width: "140px",
    help: "resources",
    text: (r) => r.resourceNames,
    edit: { initial: (r) => r.resourceNames, make: (r, text) => ({ kind: "SetTaskResources", id: r.id, text }) },
  },
  { label: "Cost", width: "96px", align: "right", help: "cost", text: (r, _perDay, currency) => (r.costCents ? formatMoney(r.costCents, currency) : "") },
  { label: "Slack", width: "76px", align: "right", help: "slack", text: (r, perDay) => formatDays(r.totalSlackMinutes, perDay) },
];
const NAME_COL = 2;

/** "50", "50%" or " 50 % "; anything else becomes 255, which Rust rejects with a clear message. */
function parsePercent(text: string): number {
  const n = Math.round(Number(text.replace("%", "").trim()));
  return text.trim() !== "" && Number.isFinite(n) && n >= 0 && n <= 100 ? n : 255;
}

/** Shown only once a baseline exists: how far each finish has moved since it was saved. */
const VARIANCE_COLUMN: Column = {
  label: "Finish Var.",
  width: "92px",
  align: "right",
  help: "baseline",
  text: (r, perDay) => {
    const v = r.finishVarianceMinutes;
    if (v === null) return "";
    if (v === 0) return "on plan";
    return `${v > 0 ? "+" : "−"}${formatDays(Math.abs(v), perDay)}`;
  },
};

function columnsFor(hasBaseline: boolean): Column[] {
  return hasBaseline ? [...COLUMNS, VARIANCE_COLUMN] : COLUMNS;
}
/** The columns' combined minimum width: rows and headers never get narrower, so a narrow
 * window scrolls sideways with headers and rows staying in line. */
const minWidthOf = (cols: Column[]) => cols.reduce((sum, c) => sum + Number(/(\d+)px/.exec(c.width)?.[1] ?? 0), 0);

export interface GridProps {
  snapshot: Snapshot;
  /** Resolves with the result, or rejects with a message to show next to the cell. */
  onEdit: (edit: Edit) => Promise<EditResult>;
  onTextEditing: (editing: boolean) => void;
  /** Vertical scroll position, shared with the Gantt chart. */
  onScrollTop: (scrollTop: number) => void;
  onSelectionChange: (ids: Set<string>) => void;
}

/** What the Gantt chart can ask of the grid. */
export interface GridHandle {
  selectRow: (row: number) => void;
  scrollBy: (dy: number) => void;
  /** For the command bar: act on the selection. */
  indent: (outdent: boolean) => void;
  addTaskBelow: () => void;
  deleteSelection: () => void;
  /** The task under the cursor, for Task Info. */
  activeRow: () => Row | undefined;
  /** Edit > Find: the next task whose name contains `text`, wrapping. While typing
   * (`fromCurrent`), the current row counts; Return moves past it. */
  findNext: (text: string, fromCurrent?: boolean) => boolean;
}

interface Editing {
  text: string;
  error: string | null;
  /** Select the existing text when the editor opens, so typing replaces it (as in Finder). */
  selectAll?: boolean;
}

/**
 * The task grid. Keyboard first: arrows move, Return edits and saves, Escape cancels,
 * Tab / Shift-Tab indent and outdent, Delete removes rows, typing starts editing.
 * Only visible rows are rendered, so thousands of tasks scroll smoothly.
 */
export const Grid = forwardRef<GridHandle, GridProps>(function Grid(
  { snapshot, onEdit, onTextEditing, onScrollTop, onSelectionChange },
  handle,
) {
  const rows = snapshot.rows;
  const hasBaseline = snapshot.project.baselineSavedAt !== null;
  const cols = useMemo(() => columnsFor(hasBaseline), [hasBaseline]);
  const template = cols.map((c) => c.width).join(" ");
  const minWidth = minWidthOf(cols);
  const colCount = useRef(cols.length);
  colCount.current = cols.length;
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
  const [scrollLeft, setScrollLeft] = useState(0);
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
  const selectionKey = selection.map((r) => r.id).join(",");
  useEffect(() => onSelectionChange(new Set(selectionKey ? selectionKey.split(",") : [])), [selectionKey, onSelectionChange]);
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

  // Focus the editor before the browser paints, so the next keystroke lands in it.
  useLayoutEffect(() => {
    if (!editing) return;
    inputRef.current?.focus();
    if (editing.selectAll) inputRef.current?.select();
  }, [editing !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => gridRef.current?.focus(), []);

  const focusGrid = () => gridRef.current?.focus();

  useImperativeHandle(handle, () => ({
    selectRow: (r: number) => {
      void commit();
      moveTo(r, active.col);
      focusGrid();
    },
    scrollBy: (dy: number) => {
      if (bodyRef.current) bodyRef.current.scrollTop += dy;
    },
    indent: (outdent: boolean) => indent(outdent),
    addTaskBelow: () => void send({ kind: "AddTask", afterId: current?.id ?? rows.at(-1)?.id ?? null, name: "New task" }),
    deleteSelection: () => {
      if (selection.length) void send({ kind: "DeleteTasks", ids: selection.map((r) => r.id) });
    },
    activeRow: () => current,
    findNext: (text: string, fromCurrent = false) => {
      const needle = text.trim().toLocaleLowerCase();
      if (!needle) return false;
      for (let k = fromCurrent ? 0 : 1; k <= rows.length; k++) {
        const i = (row + k) % rows.length;
        if (rows[i].name.toLocaleLowerCase().includes(needle)) {
          moveTo(i, NAME_COL);
          return true;
        }
      }
      return false;
    },
  }));

  // Moves can follow an edit that added rows, before this render's rowCount is updated.
  const rowCountRef = useRef(rowCount);
  rowCountRef.current = rowCount;
  const moveTo = useCallback((r: number, c: number, extend = false) => {
    const nr = Math.max(0, Math.min(rowCountRef.current - 1, r));
    setActive({ row: nr, col: Math.max(0, Math.min(colCount.current - 1, c)) });
    if (!extend) setAnchor(nr);
  }, []);

  const send = async (edit: Edit): Promise<string | null> => {
    if (busy.current) return null; // one edit at a time; extra key presses are ignored
    busy.current = true;
    try {
      const result = await onEdit(edit);
      rowCountRef.current = result.snapshot.rows.length + 1;
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

  /** `initial`: a typed character that replaces the cell. Without it, the cell's text is selected. */
  const startEditing = (initial?: string) => {
    const col = cols[active.col];
    if (current && col.edit) {
      setEditing({ text: initial ?? col.edit.initial(current), error: null, selectAll: initial === undefined });
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
    const col = cols[active.col];
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
    const open = editingRef.current;
    if (open) {
      // The editor just opened and has not taken focus yet: keep fast typing, don't lose it.
      if (e.key.length === 1 && !e.metaKey && !e.ctrlKey) {
        setEditing({ text: open.text + e.key, error: null });
        e.preventDefault();
      } else if (e.key === "Enter" || e.key === "Escape") {
        void onEditorKey(e as React.KeyboardEvent<HTMLInputElement>);
      }
      return;
    }
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
        moveTo(row, e.metaKey ? cols.length - 1 : active.col + 1);
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
      aria-colcount={cols.length}
      aria-multiselectable
      aria-activedescendant={`cell-${row}-${active.col}`}
      tabIndex={0}
      onKeyDown={onGridKey}
    >
      <div className="grid-header-wrap">
        <div
          className="grid-header"
          role="row"
          aria-rowindex={1}
          style={{ gridTemplateColumns: template, height: HEADER_HEIGHT, minWidth: minWidth, transform: `translateX(${-scrollLeft}px)` }}
        >
          {cols.map((c, i) => (
            <div key={i} role="columnheader" className={c.align === "right" ? "right" : undefined} data-tour={c.tour}>
              {c.label}
              {c.help && <HeaderHelp term={c.help} />}
            </div>
          ))}
        </div>
      </div>
      <div
        ref={bodyRef}
        className="grid-body"
        onScroll={(e) => {
          setScrollTop(e.currentTarget.scrollTop);
          setScrollLeft(e.currentTarget.scrollLeft);
          onScrollTop(e.currentTarget.scrollTop);
        }}
      >
        <div style={{ height: rowCount * ROW_HEIGHT, position: "relative", minWidth: minWidth }}>
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
                style={{ top: i * ROW_HEIGHT, height: ROW_HEIGHT, gridTemplateColumns: template }}
              >
                {cols.map((c, ci) => {
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
                        cellContent(r, i, ci, c, perDay, snapshot.project.currency)
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
});

function cellContent(r: Row | undefined, i: number, ci: number, c: Column, perDay: number, currency: string) {
  if (!r) {
    return ci === NAME_COL ? <span className="placeholder">Type a task name</span> : null;
  }
  if (ci === 0) return <span className="secondary">{i + 1}</span>;
  if (ci === 1) {
    // A marker and a label, not colour alone (spec 5.1.9). A constraint gets its own marker,
    // so a date the user fixed is never hidden.
    const constraint = r.constraint.kind === "ASAP" ? null : `${CONSTRAINTS[r.constraint.kind].label}${r.constraint.date ? ` ${formatDate(r.constraint.date)}` : ""}`;
    return (
      <>
        {r.critical && (
          <span className="critical" role="img" aria-label="Critical" title="Critical: any delay moves the project finish">
            ▲
          </span>
        )}
        {constraint && (
          <span className="constraint" role="img" aria-label={constraint} title={constraint}>
            ⌖
          </span>
        )}
        {r.overloaded && (
          <span
            className="overload"
            role="img"
            aria-label="A resource on this task is over-allocated"
            title="A resource on this task has more work than time while it runs. See the Resource Sheet (⌘2)."
          >
            ⚠
          </span>
        )}
      </>
    );
  }
  if (ci === NAME_COL && r.isMilestone) {
    return (
      <>
        <span aria-label="Milestone" title="Milestone">◆ </span>
        {r.name}
      </>
    );
  }
  return c.text(r, perDay, currency);
}

/** A small "?" next to a column name that explains the term in plain words. */
function HeaderHelp({ term }: { term: Term }) {
  const [open, setOpen] = useState(false);
  const g = GLOSSARY[term];
  return (
    <span className="header-help">
      <button
        className="help-button"
        aria-label={`What does ${g.term} mean?`}
        aria-expanded={open}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={() => setOpen(!open)}
        onBlur={() => setOpen(false)}
      >
        ?
      </button>
      {open && (
        <span role="tooltip" className="help-popover">
          <strong>{g.term}</strong>
          <br />
          {g.text}
        </span>
      )}
    </span>
  );
}
