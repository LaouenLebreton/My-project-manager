import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";
import { keys } from "./platform";
import { api, errorMessage, type Edit, type Snapshot } from "./api";
import type { EditResult } from "./bindings/EditResult";
import type { Row } from "./bindings/Row";
import { formatDate } from "./format";
import { GLOSSARY, type Term } from "./help";
import { blocked } from "./permissions";
import { CONSTRAINTS } from "./Sheets";
import { ALL_COLUMNS, columnById, defaultColumns, loadLayout, moved, saveLayout, toggled, type Column, type Layout } from "./columns";
import { useContextMenu, type MenuItem } from "./ContextMenu";
import { isMac } from "./platform";

import { HEADER_HEIGHT, ROW_HEIGHT } from "./Gantt";

const OVERSCAN = 8;

export interface GridProps {
  snapshot: Snapshot;
  /** Resolves with the result, or rejects with a message to show next to the cell. */
  onEdit: (edit: Edit) => Promise<EditResult>;
  onTextEditing: (editing: boolean) => void;
  /** Vertical scroll position, shared with the Gantt chart. */
  onScrollTop: (scrollTop: number) => void;
  onSelectionChange: (ids: Set<string>) => void;
  /** Edit > Paste: `text` from the clipboard, pasted after `afterId`. */
  onPaste: (afterId: string | null, text: string) => Promise<EditResult>;
  /** An edit the person may not make in this team project: why, for a notice. */
  onBlocked?: (reason: string) => void;
  /** Task Info for a row (from its right-click menu). */
  onTaskInfo?: (row: Row) => void;
  /** Each task's number in the whole project (rows may be hidden by a filter or the outline). */
  rowNumbers?: Map<string, number>;
  /** Summaries whose tasks are hidden, and showing or hiding them. */
  collapsed?: ReadonlySet<string>;
  onToggleCollapsed?: (id: string) => void;
}

/** What the Gantt chart can ask of the grid. */
export interface GridHandle {
  selectRow: (row: number) => void;
  scrollBy: (dy: number) => void;
  /** For the command bar: act on the selection. */
  indent: (outdent: boolean) => void;
  /** Task > Move Up / Move Down. */
  moveRows: (down: boolean) => void;
  /** Task > Duplicate. */
  duplicate: () => void;
  addTaskBelow: () => void;
  /** Insert Task: a new task above the active one, at its level. */
  insertTask: () => void;
  /** A new task inside the active one. */
  addSubtask: () => void;
  /** The right-click menu for row `row` (0-based), e.g. from its bar in the chart. */
  openRowMenu: (e: React.MouseEvent, row: number) => void;
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
  { snapshot, onEdit, onPaste, onTextEditing, onScrollTop, onSelectionChange, onBlocked, onTaskInfo, rowNumbers, collapsed, onToggleCollapsed },
  handle,
) {
  const rows = snapshot.rows;
  const hasBaseline = snapshot.project.baselineSavedAt !== null;
  // Which columns, in what order and how wide: the person's choice, kept on this computer.
  const [layout, setLayoutState] = useState<Layout | null>(loadLayout);
  const setLayout = (l: Layout | null) => {
    setLayoutState(l);
    saveLayout(l);
  };
  const ids = layout?.ids ?? defaultColumns(hasBaseline);
  const widths = layout?.widths ?? {};
  const cols = useMemo(() => ids.map(columnById).filter((c): c is Column => c !== undefined), [ids.join(",")]); // eslint-disable-line react-hooks/exhaustive-deps
  const widthOf = (c: Column) => widths[c.id] ?? c.width;
  // The Task Name takes any spare room; the list never gets narrower than its columns, so a
  // narrow window scrolls sideways with headers and rows in line.
  const template = cols.map((c) => (c.id === "name" ? `minmax(${widthOf(c)}px, 1fr)` : `${widthOf(c)}px`)).join(" ");
  const minWidth = cols.reduce((sum, c) => sum + widthOf(c), 0);
  const NAME_COL = Math.max(0, cols.findIndex((c) => c.id === "name"));
  const openMenu = useContextMenu();
  const changeColumns = (next: string[]) => setLayout({ ids: next, widths });
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
    duplicate: () => {
      const ids = selection.length ? selection.map((r) => r.id) : current ? [current.id] : [];
      if (ids.length) void send({ kind: "DuplicateTasks", ids });
    },
    moveRows: (down: boolean) => {
      const ids = selection.length ? selection.map((r) => r.id) : current ? [current.id] : [];
      if (ids.length) void reorder({ kind: "MoveTasks", ids, down }, ids);
    },
    addTaskBelow: () => void send({ kind: "AddTask", afterId: current?.id ?? null, name: "New task", place: current ? "After" : "End" }),
    insertTask: () => void send({ kind: "AddTask", afterId: current?.id ?? null, name: "New task", place: current ? "Above" : "End" }),
    addSubtask: () => {
      if (current) void send({ kind: "AddSubtask", parentId: current.id, name: "New task" });
    },
    deleteSelection: () => {
      if (selection.length) void send({ kind: "DeleteTasks", ids: selection.map((r) => r.id) });
    },
    activeRow: () => current,
    openRowMenu: (e: React.MouseEvent, i: number) => {
      const r = rows[i];
      if (!r) return;
      const inSelection = selectedIds.has(r.id) && selection.length > 1;
      if (!inSelection) moveTo(i, active.col);
      openMenu(e, rowMenu(r, inSelection ? selection : [r]));
    },
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
    const why = col.kind ? blocked(snapshot, col.kind, current ?? null) : null;
    if (why && (current ? col.edit : active.col === NAME_COL)) {
      onBlocked?.(why);
      return;
    }
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
      // The blank row under the list: a new top-level task at the end (Tab makes it a subtask).
      edit = { kind: "AddTask", afterId: null, name: text.trim(), place: "End" };
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

  /** A reorder: the selection follows the moved tasks to their new rows. */
  const reorder = async (edit: Edit, ids: string[]) => {
    if (busy.current) return;
    busy.current = true;
    try {
      const result = await onEdit(edit);
      rowCountRef.current = result.snapshot.rows.length + 1;
      const wanted = new Set(ids);
      const at = result.snapshot.rows.flatMap((r, i) => (wanted.has(r.id) ? [i] : []));
      if (at.length) {
        setAnchor(at[0]);
        setActive((a) => ({ row: at[at.length - 1], col: a.col }));
      }
    } catch (e) {
      onBlocked?.(errorMessage(e));
    } finally {
      busy.current = false;
    }
  };

  // Edit > Copy / Cut / Paste while the list (not a cell editor) has focus. The names go on the
  // system clipboard (one per line, indented by level), so they paste into other apps; the app
  // keeps the full tasks for pasting in any project.
  const clipRef = useRef({ selection, current, rows });
  clipRef.current = { selection, current, rows };
  useEffect(() => {
    const inList = () => document.activeElement === gridRef.current && !editingRef.current;
    const copy = (e: ClipboardEvent, cut: boolean) => {
      const { selection: sel } = clipRef.current;
      if (!inList() || !sel.length || !e.clipboardData) return;
      e.preventDefault();
      const base = Math.min(...sel.map((r) => r.depth));
      const text = sel.map((r) => "\t".repeat(r.depth - base) + r.name).join("\n");
      e.clipboardData.setData("text/plain", text);
      const ids = sel.map((r) => r.id);
      void api.copyTasks(ids, text).then(() => (cut ? send({ kind: "DeleteTasks", ids }) : null), (err) => onBlocked?.(errorMessage(err)));
    };
    const onCopy = (e: ClipboardEvent) => copy(e, false);
    const onCut = (e: ClipboardEvent) => copy(e, true);
    const onPasteEvent = (e: ClipboardEvent) => {
      if (!inList() || !e.clipboardData) return;
      const text = e.clipboardData.getData("text/plain");
      if (!text.trim()) return;
      e.preventDefault();
      const { current: cur, rows: all } = clipRef.current;
      const afterId = cur?.id ?? all.at(-1)?.id ?? null;
      void onPaste(afterId, text).then(
        (result) => {
          if (result.createdId) {
            const at = result.snapshot.rows.findIndex((r) => r.id === result.createdId);
            if (at >= 0) moveTo(at, active.col);
          }
        },
        (err) => onBlocked?.(errorMessage(err)),
      );
    };
    document.addEventListener("copy", onCopy);
    document.addEventListener("cut", onCut);
    document.addEventListener("paste", onPasteEvent);
    return () => {
      document.removeEventListener("copy", onCopy);
      document.removeEventListener("cut", onCut);
      document.removeEventListener("paste", onPasteEvent);
    };
  });

  // Dragging rows by their number: where they would land (`index` rows from the top, before or
  // after it), shown as a line, applied on release.
  const [drop, setDrop] = useState<{ index: number; after: boolean } | null>(null);
  const startRowDrag = (startY: number, ids: string[]) => {
    const body = bodyRef.current;
    if (!body) return;
    let target: { index: number; after: boolean } | null = null;
    const where = (clientY: number) => {
      const y = clientY - body.getBoundingClientRect().top + body.scrollTop;
      const index = Math.max(0, Math.min(rows.length - 1, Math.floor(y / ROW_HEIGHT)));
      return { index, after: y - index * ROW_HEIGHT > ROW_HEIGHT / 2 };
    };
    const move = (e: MouseEvent) => {
      if (Math.abs(e.clientY - startY) < 4 && !target) return;
      target = where(e.clientY);
      setDrop(target);
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      setDrop(null);
      const t = target && rows[target.index];
      if (t && !ids.includes(t.id)) void reorder({ kind: "MoveTo", ids, targetId: t.id, after: target!.after }, ids);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
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
        moveTo(e.metaKey || e.ctrlKey ? 0 : row - 1, active.col, shift);
        break;
      case "ArrowDown":
        moveTo(e.metaKey || e.ctrlKey ? rowCount - 1 : row + 1, active.col, shift);
        break;
      case "ArrowLeft":
        // ⌥← hides a summary's tasks; ⌥→ shows them.
        if (e.altKey && current?.isSummary) {
          if (!collapsed?.has(current.id)) onToggleCollapsed?.(current.id);
          break;
        }
        moveTo(row, e.metaKey || e.ctrlKey ? 0 : active.col - 1);
        break;
      case "ArrowRight":
        if (e.altKey && current?.isSummary) {
          if (collapsed?.has(current.id)) onToggleCollapsed?.(current.id);
          break;
        }
        moveTo(row, e.metaKey || e.ctrlKey ? cols.length - 1 : active.col + 1);
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

  // Column headers: drag to reorder, drag the right edge to resize, right-click to choose.
  const [dragCol, setDragCol] = useState<{ id: string; over: string | null } | null>(null);
  const startResize = (e: React.MouseEvent, c: Column) => {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const start = widthOf(c);
    let w = start;
    const move = (ev: MouseEvent) => {
      w = Math.max(36, Math.round(start + ev.clientX - startX));
      setLayoutState({ ids, widths: { ...widths, [c.id]: w } });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      setLayout({ ids, widths: { ...widths, [c.id]: w } });
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };
  const GROUPS = ["General", "Schedule", "Progress", "Cost", "Baseline", "Advanced"] as const;
  const chooserMenu = (): MenuItem[] => [
    ...GROUPS.flatMap((g): MenuItem[] => [
      { heading: g },
      ...ALL_COLUMNS.filter((c) => c.group === g && !c.fixed).map((c) => ({ label: c.label, checked: ids.includes(c.id), run: () => changeColumns(toggled(ids, c.id)) })),
    ]),
    { separator: true },
    { label: "Reset Columns", run: () => setLayout(null) },
  ];
  const columnMenu = (c: Column): MenuItem[] => [
    ...(c.fixed ? [] : [{ label: `Hide “${c.label}”`, run: () => changeColumns(toggled(ids, c.id)) }, { separator: true } as MenuItem]),
    ...(widths[c.id] ? [{ label: "Default Width", run: () => setLayout({ ids, widths: Object.fromEntries(Object.entries(widths).filter(([k]) => k !== c.id)) }) }, { separator: true } as MenuItem] : []),
    ...chooserMenu(),
  ];

  /** The blank row: start typing a new task there. */
  const startEditingAt = (i: number) => {
    moveTo(i, NAME_COL);
    setEditing({ text: "", error: null });
  };
  const mod = isMac ? "⌘" : "Ctrl+";
  const rowMenu = (r: Row, sel: Row[]): MenuItem[] => {
    const ids = sel.map((x) => x.id);
    const one = sel.length === 1;
    const index = rows.findIndex((x) => x.id === r.id);
    const above = index > 0 ? rows[index - 1] : null;
    return [
      { label: "Insert Task Above", shortcut: isMac ? "⇧⌘↩" : "Ctrl+Shift+Enter", run: () => void send({ kind: "AddTask", afterId: r.id, name: "New task", place: "Above" }) },
      { label: "Add Task Below", run: () => void send({ kind: "AddTask", afterId: r.id, name: "New task", place: "After" }) },
      { label: "Add Subtask", run: () => void send({ kind: "AddSubtask", parentId: r.id, name: "New task" }) },
      { separator: true },
      { label: one ? "Task Info…" : "Task Info… (first selected)", shortcut: `${mod}I`, run: () => onTaskInfo?.(r) },
      {
        label: "Link to the Task Above",
        disabled: !one || !above || r.isSummary,
        run: () => {
          const n = String(rowNumbers?.get(above!.id) ?? index); // the number shown in the # column
          const list = r.predecessors.trim() ? `${r.predecessors}, ${n}` : n;
          void send({ kind: "SetPredecessors", id: r.id, text: list });
        },
      },
      { separator: true },
      { label: "Indent", shortcut: "Tab", run: () => void send({ kind: "Indent", ids }) },
      { label: "Outdent", shortcut: "⇧Tab", run: () => void send({ kind: "Outdent", ids }) },
      { label: "Move Up", shortcut: isMac ? "⌃⌘↑" : "Alt+Shift+↑", run: () => void reorder({ kind: "MoveTasks", ids, down: false }, ids) },
      { label: "Move Down", shortcut: isMac ? "⌃⌘↓" : "Alt+Shift+↓", run: () => void reorder({ kind: "MoveTasks", ids, down: true }, ids) },
      { separator: true },
      { heading: "Mark complete" },
      ...[0, 50, 100].map((n) => ({ label: `${n}%`, checked: one ? r.percentComplete === n : undefined, run: () => void send({ kind: "SetPercentComplete", ids, percent: n }) })),
      { separator: true },
      { label: "Duplicate", shortcut: `${mod}D`, run: () => void send({ kind: "DuplicateTasks", ids }) },
      { label: one ? "Delete Task" : `Delete ${sel.length} Tasks`, shortcut: "⌫", danger: true, run: () => void send({ kind: "DeleteTasks", ids }) },
    ];
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
      <button
        className="column-chooser"
        title="Choose columns"
        aria-label="Choose columns"
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          openMenu({ clientX: r.left, clientY: r.bottom, preventDefault: () => {}, stopPropagation: () => {} }, chooserMenu());
        }}
      >
        ⊕
      </button>
      <div className="grid-header-wrap">
        <div
          className="grid-header"
          role="row"
          aria-rowindex={1}
          style={{ gridTemplateColumns: template, height: HEADER_HEIGHT, minWidth: minWidth, transform: `translateX(${-scrollLeft}px)` }}
        >
          {cols.map((c, i) => (
            <div
              key={c.id}
              role="columnheader"
              className={[c.align === "right" ? "right" : "", dragCol && dragCol.over === c.id ? "drop-before" : ""].filter(Boolean).join(" ") || undefined}
              data-tour={c.tour}
              draggable={!c.fixed || c.id === "name"}
              title={c.fixed && c.id !== "name" ? undefined : "Drag to move this column; right-click for more columns"}
              onDragStart={(e) => {
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", c.id);
                setDragCol({ id: c.id, over: null });
              }}
              onDragOver={(e) => {
                if (!dragCol || i < 2) return;
                e.preventDefault();
                if (dragCol.over !== c.id) setDragCol({ ...dragCol, over: c.id });
              }}
              onDragEnd={() => setDragCol(null)}
              onDrop={(e) => {
                e.preventDefault();
                if (dragCol && dragCol.id !== c.id) changeColumns(moved(ids, dragCol.id, c.id));
                setDragCol(null);
              }}
              onContextMenu={(e) => openMenu(e, columnMenu(c))}
            >
              {c.label}
              {c.help && <HeaderHelp term={c.help} />}
              {i > 0 && (
                <span
                  className="col-resize"
                  aria-hidden
                  onMouseDown={(e) => startResize(e, c)}
                  onDoubleClick={() => setLayout({ ids, widths: Object.fromEntries(Object.entries(widths).filter(([k]) => k !== c.id)) })}
                />
              )}
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
          {drop && <div className="drop-line" style={{ top: (drop.index + (drop.after ? 1 : 0)) * ROW_HEIGHT - 1 }} aria-hidden />}
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
                      aria-readonly={!c.edit || (!!c.kind && !!r && blocked(snapshot, c.kind, r) !== null)}
                      className={["cell", c.align === "right" ? "right" : "", isActive ? "active" : "", c.edit && c.kind && r && blocked(snapshot, c.kind, r) ? "locked" : ""].filter(Boolean).join(" ")}
                      title={r && c.title ? c.title(r) : undefined}
                      onMouseDown={(e) => {
                        if (editing && !isEditing) void commit();
                        // The row number is the handle for dragging rows (the whole selection
                        // when this row is part of it).
                        if (e.button !== 0) return; // right-click: the menu, keeping the selection
                        if (c.id === "row" && r && !e.shiftKey) {
                          const ids = selectedIds.has(r.id) && selection.length > 1 ? selection.map((x) => x.id) : [r.id];
                          if (ids.length === 1) moveTo(i, ci);
                          startRowDrag(e.clientY, ids);
                          return;
                        }
                        moveTo(i, ci, e.shiftKey);
                      }}
                      onDoubleClick={() => startEditing()}
                      onContextMenu={(e) => {
                        if (!r) return openMenu(e, [{ label: "Add Task", run: () => startEditingAt(i) }]);
                        if (!selectedIds.has(r.id)) moveTo(i, ci);
                        openMenu(e, rowMenu(r, selectedIds.has(r.id) && selection.length > 1 ? selection : [r]));
                      }}
                      style={c.id === "name" && r ? { paddingLeft: (r.isSummary ? 2 : 20) + r.depth * 18 } : undefined}
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
                        cellContent(r, rowNumbers?.get(r?.id ?? "") ?? i + 1, c, perDay, snapshot.project.currency, r?.isSummary ? { collapsed: collapsed?.has(r.id) ?? false, toggle: () => onToggleCollapsed?.(r.id) } : null)
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

function cellContent(r: Row | undefined, number: number, c: Column, perDay: number, currency: string, outline: { collapsed: boolean; toggle: () => void } | null) {
  if (!r) {
    return c.id === "name" ? <span className="placeholder">Type a task name</span> : null;
  }
  if (c.id === "row") return <span className="secondary">{number}</span>;
  if (c.id === "name" && outline) {
    return (
      <>
        <button
          className="outline-toggle"
          aria-label={outline.collapsed ? `Show the tasks in ${r.name}` : `Hide the tasks in ${r.name}`}
          aria-expanded={!outline.collapsed}
          title={outline.collapsed ? "Show the tasks inside (⌥→)" : "Hide the tasks inside (⌥←)"}
          tabIndex={-1}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={outline.toggle}
        >
          {outline.collapsed ? "▸" : "▾"}
        </button>
        {r.name}
      </>
    );
  }
  if (c.id === "indicators") {
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
        {r.missedDeadline && (
          <span className="critical" role="img" aria-label={`Finishes after its deadline, ${formatDate(r.deadline)}`} title={`Finishes after its deadline (${formatDate(r.deadline)})`}>
            ⚑
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
            title={keys("A resource on this task has more work than time while it runs. See the Resource Sheet (⌘2).")}
          >
            ⚠
          </span>
        )}
      </>
    );
  }
  if (c.id === "name" && r.isMilestone) {
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
