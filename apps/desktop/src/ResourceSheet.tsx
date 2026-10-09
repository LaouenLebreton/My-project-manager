import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { errorMessage, type Edit, type EditResult } from "./api";
import type { ResourceRow } from "./bindings/ResourceRow";
import type { Snapshot } from "./bindings/Snapshot";
import { formatDate, formatMoney } from "./format";
import { GLOSSARY } from "./help";

interface Column {
  label: string;
  width: string;
  align?: "right";
  text: (r: ResourceRow, currency: string) => string;
  /** Editable columns: what the editor starts with, and the edit to send. */
  edit?: (r: ResourceRow) => { initial: string; make: (text: string) => Edit } | null;
}

const update = (r: ResourceRow, fields: Partial<Extract<Edit, { kind: "UpdateResource" }>>): Edit => ({
  kind: "UpdateResource",
  id: r.id,
  name: null,
  initials: null,
  resourceType: null,
  maxUnits: null,
  materialLabel: null,
  rate: null,
  ...fields,
});

const COLUMNS: Column[] = [
  { label: "", width: "28px", text: () => "" },
  {
    label: "Resource Name",
    width: "minmax(180px, 1fr)",
    text: (r) => r.name,
    edit: (r) => ({ initial: r.name, make: (t) => update(r, { name: t }) }),
  },
  {
    label: "Initials",
    width: "70px",
    text: (r) => r.initials,
    edit: (r) => ({ initial: r.initials, make: (t) => update(r, { initials: t }) }),
  },
  {
    label: "Type",
    width: "90px",
    text: (r) => r.resourceType,
    edit: (r) => ({
      initial: r.resourceType,
      make: (t) => update(r, { resourceType: t.toLowerCase().startsWith("m") ? "Material" : "Work" }),
    }),
  },
  {
    label: "Max Units",
    width: "90px",
    align: "right",
    text: (r) => r.maxUnitsText,
    edit: (r) => (r.resourceType === "Work" ? { initial: r.maxUnitsText, make: (t) => update(r, { maxUnits: t }) } : null),
  },
  {
    label: "Material Label",
    width: "110px",
    text: (r) => r.materialLabel,
    edit: (r) => (r.resourceType === "Material" ? { initial: r.materialLabel, make: (t) => update(r, { materialLabel: t }) } : null),
  },
  {
    label: "Rate",
    width: "110px",
    align: "right",
    text: (r, currency) => (r.rateCents ? `${formatMoney(r.rateCents, currency)}${r.resourceType === "Work" ? "/h" : ""}` : ""),
    edit: (r) => ({ initial: r.rateCents ? (r.rateCents / 100).toFixed(2) : "", make: (t) => update(r, { rate: t }) }),
  },
  { label: "Work", width: "90px", align: "right", text: (r) => r.totalText },
  { label: "Cost", width: "100px", align: "right", text: (r, currency) => (r.costCents ? formatMoney(r.costCents, currency) : "") },
  { label: "Peak", width: "70px", align: "right", text: (r) => (r.resourceType === "Work" ? `${r.peakPercent}%` : "") },
  { label: "Tasks", width: "60px", align: "right", text: (r) => String(r.taskCount) },
];
const NAME_COL = 1;
const TEMPLATE = COLUMNS.map((c) => c.width).join(" ");
/** The columns' combined minimum width: rows and headers never get narrower, so a narrow
 * window scrolls sideways with headers and rows staying in line. */
const MIN_WIDTH = COLUMNS.reduce((sum, c) => sum + Number(/(\d+)px/.exec(c.width)?.[1] ?? 0), 0);

/**
 * The Resource Sheet: the project's people, equipment and materials. Keyboard first, like the
 * task grid: arrows move, Return edits, typing starts editing, Delete removes a resource
 * (its assignments go with it, and Undo brings both back).
 */
export function ResourceSheet({ snapshot, onEdit, onTextEditing }: { snapshot: Snapshot; onEdit: (e: Edit) => Promise<EditResult>; onTextEditing: (editing: boolean) => void }) {
  const rows = snapshot.resources;
  const rowCount = rows.length + 1; // plus the blank row for a new resource
  const [active, setActive] = useState({ row: 0, col: NAME_COL });
  const [editing, setEditingState] = useState<{ text: string; error: string | null } | null>(null);
  const editingRef = useRef<typeof editing>(null);
  const setEditing = (e: typeof editing) => {
    editingRef.current = e;
    setEditingState(e);
  };
  const [scrollLeft, setScrollLeft] = useState(0);
  const gridRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const row = Math.min(active.row, rowCount - 1);
  const current: ResourceRow | undefined = rows[row];

  useEffect(() => gridRef.current?.focus(), []);
  useEffect(() => onTextEditing(editing !== null), [editing !== null, onTextEditing]); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  const move = (r: number, c: number) => setActive({ row: Math.max(0, Math.min(rowCount - 1, r)), col: Math.max(0, Math.min(COLUMNS.length - 1, c)) });

  const editorFor = () => {
    if (!current) return active.col === NAME_COL ? { initial: "", make: (t: string): Edit => ({ kind: "AddResource", afterId: null, name: t }) } : null;
    return COLUMNS[active.col].edit?.(current) ?? null;
  };

  const start = (initial?: string) => {
    const e = editorFor();
    if (e) setEditing({ text: initial ?? e.initial, error: null });
  };

  const commit = async (): Promise<boolean> => {
    const ed = editingRef.current;
    if (!ed) return true;
    editingRef.current = null;
    const e = editorFor();
    if (e && ed.text !== e.initial && (current || ed.text.trim())) {
      try {
        await onEdit(e.make(ed.text.trim()));
      } catch (err) {
        setEditing({ text: ed.text, error: errorMessage(err) });
        return false;
      }
    }
    setEditing(null);
    gridRef.current?.focus();
    return true;
  };

  const onGridKey = (e: React.KeyboardEvent) => {
    if (editingRef.current) return;
    switch (e.key) {
      case "ArrowUp":
        move(row - 1, active.col);
        break;
      case "ArrowDown":
        move(row + 1, active.col);
        break;
      case "ArrowLeft":
        move(row, active.col - 1);
        break;
      case "ArrowRight":
        move(row, active.col + 1);
        break;
      case "Enter":
        start();
        break;
      case "Backspace":
      case "Delete":
        if (current) void onEdit({ kind: "DeleteResources", ids: [current.id] });
        break;
      default:
        if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
          start(e.key);
          break;
        }
        return;
    }
    e.preventDefault();
  };

  const onEditorKey = async (e: React.KeyboardEvent<HTMLInputElement>) => {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      // A new resource moves on to the next blank row, for fast entry.
      if (await commit()) move(row + 1, active.col);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setEditing(null);
      gridRef.current?.focus();
    }
  };

  return (
    <div className="resource-view">
      <div
        ref={gridRef}
        className="grid"
        role="grid"
        aria-label="Resources"
        aria-rowcount={rowCount + 1}
        aria-activedescendant={`res-${row}-${active.col}`}
        tabIndex={0}
        onKeyDown={onGridKey}
      >
        <div className="grid-header-wrap">
          <div className="grid-header" role="row" style={{ gridTemplateColumns: TEMPLATE, height: 34, minWidth: MIN_WIDTH, transform: `translateX(${-scrollLeft}px)` }}>
            {COLUMNS.map((c, i) => (
              <div key={i} role="columnheader" className={c.align === "right" ? "right" : undefined}>
                {c.label}
              </div>
            ))}
          </div>
        </div>
        <div className="grid-body" onScroll={(e) => setScrollLeft(e.currentTarget.scrollLeft)}>
          <div style={{ minWidth: MIN_WIDTH }}>
          {[...rows, undefined].map((r, i) => (
            <div
              key={r?.id ?? "new"}
              role="row"
              className={["grid-row", "static", i % 2 ? "odd" : "", i === row ? "selected" : ""].filter(Boolean).join(" ")}
              style={{ gridTemplateColumns: TEMPLATE, height: 26 }}
            >
              {COLUMNS.map((c, ci) => {
                const isActive = i === row && ci === active.col;
                return (
                  <div
                    key={ci}
                    id={`res-${i}-${ci}`}
                    role="gridcell"
                    className={["cell", c.align === "right" ? "right" : "", isActive ? "active" : ""].filter(Boolean).join(" ")}
                    onMouseDown={() => {
                      if (editingRef.current && !isActive) void commit();
                      move(i, ci);
                    }}
                    onDoubleClick={() => start()}
                  >
                    {isActive && editing ? (
                      <>
                        <input
                          ref={inputRef}
                          className="editor"
                          aria-label={c.label || "Resource"}
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
                    ) : !r ? (
                      ci === NAME_COL ? <span className="placeholder">Type a name to add a resource</span> : null
                    ) : ci === 0 ? (
                      r.overloads.length > 0 ? (
                        <span className="overload" role="img" aria-label="Over-allocated" title="Over-allocated: see below">
                          ⚠
                        </span>
                      ) : null
                    ) : (
                      c.text(r, snapshot.project.currency)
                    )}
                  </div>
                );
              })}
            </div>
          ))}
          </div>
        </div>
      </div>
      <ResourceDetails resource={current} />
    </div>
  );
}

/** Under the sheet: what is wrong with the selected resource, in plain words. */
function ResourceDetails({ resource }: { resource: ResourceRow | undefined }) {
  if (!resource) {
    return (
      <aside className="resource-details secondary" aria-live="polite">
        People, equipment and materials for this project. Assign them in the Tasks view's Resources column.
      </aside>
    );
  }
  if (resource.overloads.length === 0) {
    return (
      <aside className="resource-details" aria-live="polite">
        <strong>{resource.name}</strong>
        {resource.taskCount === 0 ? " is not assigned to any task yet." : ` works on ${resource.taskCount} ${resource.taskCount === 1 ? "task" : "tasks"} and is never over-allocated.`}
      </aside>
    );
  }
  return (
    <aside className="resource-details" aria-live="polite">
      <strong>{resource.name} is over-allocated</strong> (has {resource.maxUnitsText} available):
      <ul>
        {resource.overloads.map((o, i) => (
          <li key={i}>
            up to {o.peakPercent}% from {formatDate(o.start)} to {formatDate(o.finish)}, on {o.tasks.join(", ")}
          </li>
        ))}
      </ul>
      <p className="secondary small">{GLOSSARY.overallocated.text}</p>
    </aside>
  );
}
