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
  /** A choice from a list instead of typing (the resource type, as in Microsoft Project). */
  choose?: { options: string[]; value: (r: ResourceRow) => string; make: (r: ResourceRow, value: string) => Edit };
  /** Fields that do not apply to this kind of resource show "—" and cannot be edited. */
  notApplicable?: (r: ResourceRow) => boolean;
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
    width: "100px",
    text: (r) => r.resourceType,
    choose: {
      options: ["Work", "Material"],
      value: (r) => r.resourceType,
      make: (r, value) => update(r, { resourceType: value === "Material" ? "Material" : "Work" }),
    },
  },
  {
    // Assign straight from the sheet: task names (or row numbers) with optional units, as in
    // the task grid's Resources column.
    label: "Tasks",
    width: "minmax(200px, 1fr)",
    text: (r) => r.tasksText,
    edit: (r) => ({ initial: r.tasksText, make: (t) => ({ kind: "SetResourceTasks", id: r.id, text: t }) }),
  },
  {
    label: "Max Units",
    width: "90px",
    align: "right",
    text: (r) => r.maxUnitsText,
    edit: (r) => (r.resourceType === "Work" ? { initial: r.maxUnitsText, make: (t) => update(r, { maxUnits: t }) } : null),
    notApplicable: (r) => r.resourceType === "Material",
  },
  {
    label: "Material Label",
    width: "110px",
    text: (r) => r.materialLabel,
    edit: (r) => (r.resourceType === "Material" ? { initial: r.materialLabel, make: (t) => update(r, { materialLabel: t }) } : null),
    notApplicable: (r) => r.resourceType === "Work",
  },
  {
    label: "Rate",
    width: "110px",
    align: "right",
    // Per hour for people and equipment, per unit for materials ("€120.00/ton").
    text: (r, currency) => (r.rateCents ? `${formatMoney(r.rateCents, currency)}/${r.resourceType === "Work" ? "h" : r.materialLabel || "unit"}` : ""),
    edit: (r) => ({ initial: r.rateCents ? (r.rateCents / 100).toFixed(2) : "", make: (t) => update(r, { rate: t }) }),
  },
  { label: "Work / Qty", width: "96px", align: "right", text: (r) => r.totalText },
  { label: "Cost", width: "100px", align: "right", text: (r, currency) => (r.costCents ? formatMoney(r.costCents, currency) : "") },
  { label: "Peak", width: "70px", align: "right", text: (r) => `${r.peakPercent}%`, notApplicable: (r) => r.resourceType === "Material" },
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

  const chooseRef = useRef<HTMLSelectElement>(null);

  const start = (initial?: string) => {
    const col = COLUMNS[active.col];
    if (current && col.choose) {
      // A typed first letter picks the matching option; Return or Space opens the list.
      const match = initial && col.choose.options.find((o) => o.toLowerCase().startsWith(initial.toLowerCase()));
      if (match) void choose(current, match);
      else chooseRef.current?.focus();
      return;
    }
    if (current && col.notApplicable?.(current)) return;
    const e = editorFor();
    if (e) setEditing({ text: initial ?? e.initial, error: null });
  };

  const choose = async (r: ResourceRow, value: string) => {
    const col = COLUMNS[active.col];
    if (col.choose && value !== col.choose.value(r)) {
      try {
        await onEdit(col.choose.make(r, value));
      } catch (err) {
        setEditing({ text: "", error: errorMessage(err) });
      }
    }
    gridRef.current?.focus();
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
      case " ":
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
                    ) : c.choose ? (
                      <select
                        ref={isActive ? chooseRef : undefined}
                        className="cell-select"
                        aria-label={`${c.label} of ${r.name}`}
                        value={c.choose.value(r)}
                        tabIndex={-1}
                        onMouseDown={(e) => e.stopPropagation()}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Escape") gridRef.current?.focus();
                        }}
                        onChange={(e) => {
                          move(i, ci);
                          void choose(r, e.target.value);
                        }}
                      >
                        {c.choose.options.map((o) => (
                          <option key={o}>{o}</option>
                        ))}
                      </select>
                    ) : c.notApplicable?.(r) ? (
                      <span className="secondary" aria-label="Does not apply">
                        —
                      </span>
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
        {resource.taskCount === 0 ? " is not assigned to any task yet. Type task names in its Tasks column, such as Survey, Build[50%]." : ` works on ${resource.taskCount} ${resource.taskCount === 1 ? "task" : "tasks"} and is never over-allocated.`}
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
