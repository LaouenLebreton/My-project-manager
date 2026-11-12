import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { errorMessage, type Edit, type EditResult } from "./api";
import type { Row } from "./bindings/Row";
import type { Snapshot } from "./bindings/Snapshot";
import { formatDays, formatMoney } from "./format";
import { useContextMenu, type MenuItem } from "./ContextMenu";

const short = (iso: string | null) => (iso ? new Date(`${iso.slice(0, 10)}T12:00:00`).toLocaleDateString(undefined, { day: "numeric", month: "short" }) : "");

const BOX_W = 196;
const BOX_H = 104;
const COL_GAP = 32;
const ROW_GAP = 14;
const INDENT = 24;
const TOP = 24;
const LEVEL_GAP = 52;
const ROOT = "project";

interface Placed {
  row: Row | null; // null: the project itself
  id: string;
  x: number;
  y: number;
  parent: string | null;
  hiddenCount: number;
}

/**
 * The work breakdown structure: the project on top, top-level tasks side by side, and
 * everything inside each one stacked underneath. Built with mind-map keys: Return adds a
 * sibling, Tab a subtask, Space or a double-click renames, Delete removes, arrows move;
 * drag a box onto another to move it underneath.
 */
export function WbsChart({
  snapshot,
  onEdit,
  onTextEditing,
  onSelect,
  onTaskInfo,
}: {
  snapshot: Snapshot;
  onEdit: (e: Edit) => Promise<EditResult>;
  onTextEditing: (editing: boolean) => void;
  /** The selected task (null: the project box), for the details panel. */
  onSelect?: (id: string | null) => void;
  onTaskInfo?: (r: Row) => void;
}) {
  const rows = snapshot.rows;
  const perDay = snapshot.project.minutesPerDay;
  const [selected, setSelected] = useState<string>(ROOT);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  // Opened by hand although deeper than the "levels" setting.
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [levels, setLevels] = useState<number>(() => (rows.length > 400 ? 2 : 99));
  const [zoom, setZoom] = useState(1);
  const [editing, setEditingState] = useState<{ id: string; text: string; error: string | null } | null>(null);
  const editingRef = useRef<typeof editing>(null);
  const setEditing = (e: typeof editing) => {
    editingRef.current = e;
    setEditingState(e);
  };
  const [drag, setDrag] = useState<{ id: string; over: string | null } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => wrapRef.current?.focus(), []);
  useEffect(() => onTextEditing(editing !== null), [editing !== null, onTextEditing]); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Layout: top-level tasks in columns, everything inside them stacked and indented.
  const { placed, width, height } = useMemo(() => {
    const out: Placed[] = [];
    const kids = new Map<string, Row[]>();
    for (const r of rows) {
      const key = r.parentId ?? ROOT;
      kids.set(key, [...(kids.get(key) ?? []), r]);
    }
    const count = (id: string): number => (kids.get(id) ?? []).reduce((n, k) => n + 1 + count(k.id), 0);
    const tops = kids.get(ROOT) ?? [];
    let x = 0;
    let bottom = TOP + BOX_H;
    const columnTop = TOP + BOX_H + LEVEL_GAP;
    for (const top of tops) {
      let y = columnTop;
      let maxDepth = 0;
      const walk = (r: Row, depth: number, parent: string) => {
        const hidden = collapsed.has(r.id) || (depth + 1 >= levels && !expanded.has(r.id));
        out.push({ row: r, id: r.id, x: x + depth * INDENT, y, parent, hiddenCount: hidden ? count(r.id) : 0 });
        maxDepth = Math.max(maxDepth, depth);
        y += BOX_H + ROW_GAP;
        if (!hidden) for (const k of kids.get(r.id) ?? []) walk(k, depth + 1, r.id);
      };
      walk(top, 0, ROOT);
      bottom = Math.max(bottom, y);
      x += BOX_W + maxDepth * INDENT + COL_GAP;
    }
    const width = Math.max(x - COL_GAP, BOX_W);
    out.unshift({ row: null, id: ROOT, x: width / 2 - BOX_W / 2, y: TOP, parent: null, hiddenCount: 0 });
    return { placed: out, width: width + 48, height: bottom + 24 };
  }, [rows, collapsed, expanded, levels]);

  const byId = useMemo(() => new Map(placed.map((p) => [p.id, p])), [placed]);
  useEffect(() => {
    if (!byId.has(selected)) setSelected(ROOT);
  }, [byId, selected]);
  useEffect(() => onSelect?.(selected === ROOT ? null : selected), [selected, onSelect]);

  const send = async (edit: Edit) => {
    try {
      setMessage(null);
      return await onEdit(edit);
    } catch (e) {
      setMessage(errorMessage(e));
      return null;
    }
  };

  /** Adds a task and opens its name for typing. */
  const add = async (edit: Edit) => {
    const result = await send(edit);
    if (result?.createdId) {
      setSelected(result.createdId);
      setEditing({ id: result.createdId, text: "New task", error: null });
    }
  };

  const commit = async () => {
    const e = editingRef.current;
    if (!e) return;
    editingRef.current = null;
    const row = rows.find((r) => r.id === e.id);
    if (row && e.text.trim() && e.text !== row.name) {
      try {
        await onEdit({ kind: "RenameTask", id: e.id, name: e.text.trim() });
      } catch (err) {
        setEditing({ ...e, error: errorMessage(err) });
        return;
      }
    }
    setEditing(null);
    wrapRef.current?.focus();
  };

  const visibleOrder = placed.map((p) => p.id);
  const siblingsOf = (id: string) => placed.filter((p) => p.parent === byId.get(id)?.parent).map((p) => p.id);

  const onKey = (e: React.KeyboardEvent) => {
    if (editingRef.current) return;
    const sel = byId.get(selected);
    const row = sel?.row ?? null;
    const go = (id: string | undefined) => id && setSelected(id);
    switch (e.key) {
      case "ArrowDown":
        go(visibleOrder[visibleOrder.indexOf(selected) + 1]);
        break;
      case "ArrowUp":
        go(visibleOrder[visibleOrder.indexOf(selected) - 1]);
        break;
      case "ArrowLeft": {
        const sibs = siblingsOf(selected);
        const i = sibs.indexOf(selected);
        if (row && row.depth === 0 && i > 0) go(sibs[i - 1]);
        else go(sel?.parent ?? undefined);
        break;
      }
      case "ArrowRight": {
        const sibs = siblingsOf(selected);
        const i = sibs.indexOf(selected);
        if (row && row.depth === 0 && i < sibs.length - 1) go(sibs[i + 1]);
        else go(placed.find((p) => p.parent === selected)?.id);
        break;
      }
      case "Enter":
        // A sibling after this one (or, on the project, a new top-level task).
        void add(row ? { kind: "AddTask", afterId: row.id, name: "New task", place: "After" } : { kind: "AddTask", afterId: null, name: "New task", place: "End" });
        break;
      case "Tab":
        void add(row ? { kind: "AddSubtask", parentId: row.id, name: "New task" } : { kind: "AddTask", afterId: null, name: "New task", place: "End" });
        break;
      case " ":
        if (row) setEditing({ id: row.id, text: row.name, error: null });
        break;
      case "Backspace":
      case "Delete":
        if (row) void send({ kind: "DeleteTasks", ids: [row.id] });
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  const toggle = (id: string, hidden: boolean) => {
    const edit = (set: Set<string>, add: boolean) => {
      const next = new Set(set);
      if (add) next.add(id);
      else next.delete(id);
      return next;
    };
    setCollapsed((c) => edit(c, !hidden));
    setExpanded((x) => edit(x, hidden));
  };

  // Drag a box onto another to move it underneath.
  const startDrag = (e: React.PointerEvent, id: string) => {
    if (id === ROOT || e.button !== 0) return;
    const sx = e.clientX;
    const sy = e.clientY;
    let started = false;
    const move = (ev: PointerEvent) => {
      if (!started && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 6) return;
      started = true;
      const target = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)?.closest<HTMLElement>("[data-box]")?.dataset.box ?? null;
      setDrag({ id, over: target && target !== id ? target : null });
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDrag((d) => {
        if (d?.over) void send({ kind: "MoveUnder", id: d.id, parentId: d.over === ROOT ? null : d.over });
        return null;
      });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const lines: string[] = [];
  const tops = placed.filter((p) => p.parent === ROOT);
  if (tops.length) {
    const root = byId.get(ROOT)!;
    const busY = TOP + BOX_H + LEVEL_GAP / 2;
    lines.push(`M${root.x + BOX_W / 2},${TOP + BOX_H} V${busY}`);
    lines.push(`M${tops[0].x + BOX_W / 2},${busY} H${tops[tops.length - 1].x + BOX_W / 2}`);
    for (const t of tops) lines.push(`M${t.x + BOX_W / 2},${busY} V${t.y}`);
  }
  for (const p of placed) {
    if (!p.parent || p.parent === ROOT) continue;
    const parent = byId.get(p.parent)!;
    const lx = parent.x + 10;
    lines.push(`M${lx},${parent.y + BOX_H} V${p.y + BOX_H / 2} H${p.x}`);
  }

  const selectedRow = byId.get(selected)?.row;
  const openMenu = useContextMenu();
  const boxMenu = (p: Placed): MenuItem[] => {
    const r = p.row;
    if (!r) return [{ label: "Add Task", shortcut: "↩", run: () => void add({ kind: "AddTask", afterId: null, name: "New task", place: "End" }) }];
    return [
      { label: "Add Task After", shortcut: "↩", run: () => void add({ kind: "AddTask", afterId: r.id, name: "New task", place: "After" }) },
      { label: "Add Task Before", run: () => void add({ kind: "AddTask", afterId: r.id, name: "New task", place: "Above" }) },
      { label: "Add Subtask", shortcut: "⇥", run: () => void add({ kind: "AddSubtask", parentId: r.id, name: "New task" }) },
      { separator: true },
      { label: "Rename", shortcut: "Space", run: () => setEditing({ id: r.id, text: r.name, error: null }) },
      { label: "Task Info…", run: () => onTaskInfo?.(r) },
      ...(r.isSummary ? [{ label: p.hiddenCount ? "Show the Tasks Inside" : "Hide the Tasks Inside", run: () => toggle(p.id, p.hiddenCount > 0) }] : []),
      { separator: true },
      { heading: "Mark complete" },
      ...[0, 50, 100].map((n) => ({ label: `${n}%`, checked: r.percentComplete === n, run: () => void send({ kind: "SetPercentComplete", ids: [r.id], percent: n }) })),
      { separator: true },
      { label: "Delete Task", shortcut: "⌫", danger: true, run: () => void send({ kind: "DeleteTasks", ids: [r.id] }) },
    ];
  };

  return (
    <div className="wbs-view">
      <div className="wbs-toolbar">
        <button onClick={() => void add(selectedRow ? { kind: "AddTask", afterId: selectedRow.id, name: "New task", place: "After" } : { kind: "AddTask", afterId: null, name: "New task", place: "End" })}>
          Add Task <kbd>↩</kbd>
        </button>
        <button
          onClick={() => void add(selectedRow ? { kind: "AddSubtask", parentId: selectedRow.id, name: "New task" } : { kind: "AddTask", afterId: null, name: "New task", place: "End" })}
        >
          Add Subtask <kbd>⇥</kbd>
        </button>
        <label>
          Show{" "}
          <select aria-label="Levels shown" value={levels} onChange={(e) => setLevels(Number(e.target.value))}>
            <option value={1}>1 level</option>
            <option value={2}>2 levels</option>
            <option value={3}>3 levels</option>
            <option value={99}>all levels</option>
          </select>
        </label>
        <span className="wbs-zoom" role="group" aria-label="Zoom">
          <button aria-label="Zoom out" onClick={() => setZoom((z) => Math.max(0.3, z / 1.25))}>
            −
          </button>
          <button aria-label="Zoom in" onClick={() => setZoom((z) => Math.min(2, z * 1.25))}>
            +
          </button>
        </span>
        <span className="secondary small" role="status">
          {message ?? "Return adds a task, Tab a subtask, Space renames; drag a box onto another to move it."}
        </span>
      </div>
      <div
        ref={wrapRef}
        className="wbs-scroll"
        role="tree"
        aria-label="Work breakdown structure"
        aria-activedescendant={`wbs-${selected}`}
        tabIndex={0}
        onKeyDown={onKey}
        onWheel={(e) => {
          if (e.metaKey || e.ctrlKey) {
            e.preventDefault();
            setZoom((z) => Math.max(0.3, Math.min(2, z * (e.deltaY < 0 ? 1.1 : 1 / 1.1))));
          }
        }}
      >
        <div className="wbs-canvas" style={{ width: width * zoom, height: height * zoom }}>
          <div style={{ width, height, transform: `scale(${zoom})`, transformOrigin: "0 0", position: "relative" }}>
            <svg className="wbs-lines" width={width} height={height} aria-hidden>
              <path d={lines.join(" ")} />
            </svg>
            {placed.map((p) => {
              const r = p.row;
              const isEditing = editing?.id === p.id;
              const classes = [
                "wbs-box",
                r === null ? "root" : "",
                r?.isSummary ? "summary" : "",
                r?.critical ? "critical" : "",
                selected === p.id ? "selected" : "",
                drag?.over === p.id ? "drop" : "",
                drag?.id === p.id ? "dragging" : "",
              ].filter(Boolean);
              const label = r
                ? `${r.wbs} ${r.name}, ${r.isMilestone ? "milestone" : formatDays(r.durationMinutes, perDay)}, ${short(r.start)} to ${short(r.finish)}, ${r.percentComplete}% done${r.resourceNames ? `, ${r.resourceNames}` : ""}${r.critical ? ", critical" : ""}${r.overloaded ? ", over-allocated resource" : ""}`
                : `${snapshot.project.name}, the whole project`;
              return (
                <div
                  key={p.id}
                  id={`wbs-${p.id}`}
                  data-box={p.id}
                  role="treeitem"
                  aria-level={r ? r.depth + 2 : 1}
                  aria-selected={selected === p.id}
                  aria-expanded={r?.isSummary ? p.hiddenCount === 0 : undefined}
                  aria-label={label}
                  className={classes.join(" ")}
                  style={{ left: p.x, top: p.y, width: BOX_W, height: BOX_H }}
                  onPointerDown={(e) => {
                    if (e.button !== 0) return;
                    setSelected(p.id);
                    if (!isEditing) startDrag(e, p.id);
                  }}
                  onDoubleClick={() => r && setEditing({ id: r.id, text: r.name, error: null })}
                  onContextMenu={(e) => {
                    setSelected(p.id);
                    openMenu(e, boxMenu(p));
                  }}
                >
                  <div className="wbs-top">
                    <span className="wbs-code">{r ? r.wbs : "Project"}</span>
                    <span>
                      {r?.isMilestone && <span aria-hidden>◆ </span>}
                      {r?.critical && (
                        <span className="critical" aria-hidden>
                          ▲
                        </span>
                      )}
                      {r?.overloaded && (
                        <span className="overload" aria-hidden>
                          ⚠
                        </span>
                      )}
                    </span>
                  </div>
                  {isEditing ? (
                    <>
                      <input
                        ref={inputRef}
                        className="editor"
                        aria-label="Task name"
                        value={editing.text}
                        onChange={(e) => setEditing({ ...editing, text: e.target.value, error: null })}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Enter") void commit();
                          if (e.key === "Escape") {
                            setEditing(null);
                            wrapRef.current?.focus();
                          }
                        }}
                        onBlur={() => void commit()}
                        onPointerDown={(e) => e.stopPropagation()}
                      />
                      {editing.error && (
                        <div role="alert" className="cell-error">
                          {editing.error}
                        </div>
                      )}
                    </>
                  ) : (
                    <div className="wbs-name">{r ? r.name : snapshot.project.name}</div>
                  )}
                  <div className="wbs-meta secondary">
                    {r
                      ? `${r.isMilestone ? `Milestone · ${short(r.start)}` : `${short(r.start)} – ${short(r.finish)} · ${formatDays(r.durationMinutes, perDay)}`}`
                      : `${short(snapshot.project.start)} – ${short(snapshot.project.finish)} · ${rows.length} tasks`}
                  </div>
                  <div className="wbs-meta secondary">
                    {r
                      ? [r.resourceNames, r.costCents ? formatMoney(r.costCents, snapshot.project.currency) : ""].filter(Boolean).join(" · ") || "\u00a0"
                      : snapshot.project.costCents
                        ? formatMoney(snapshot.project.costCents, snapshot.project.currency)
                        : "\u00a0"}
                  </div>
                  {r && (
                    <div className="wbs-progress" title={`${r.percentComplete}% done`}>
                      <span className="wbs-progress-track">
                        <span style={{ width: `${r.percentComplete}%` }} />
                      </span>
                      <span className="wbs-percent">{r.percentComplete}%</span>
                    </div>
                  )}
                  {r?.isSummary && (
                    <button
                      className="wbs-toggle"
                      aria-label={p.hiddenCount ? `Show ${p.hiddenCount} tasks inside` : "Hide the tasks inside"}
                      onPointerDown={(e) => e.stopPropagation()}
                      onClick={() => toggle(p.id, p.hiddenCount > 0)}
                    >
                      {p.hiddenCount ? `+${p.hiddenCount}` : "−"}
                    </button>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
