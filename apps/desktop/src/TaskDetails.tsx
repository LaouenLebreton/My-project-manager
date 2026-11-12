import { useEffect, useState } from "react";
import { errorMessage, type Edit, type EditResult } from "./api";
import type { Row } from "./bindings/Row";
import type { Snapshot } from "./bindings/Snapshot";
import { columnById, type Column } from "./columns";
import { blocked } from "./permissions";

/** The attributes shown, in order, under each heading. Each is a column of the task list, so the
 * panel and the list always show and edit a field the same way. */
const SECTIONS: { title: string; fields: string[] }[] = [
  { title: "Schedule", fields: ["duration", "start", "finish", "deadline", "predecessors", "slack", "critical", "constraint", "mode"] },
  { title: "People and cost", fields: ["resources", "work", "cost", "fixedCost", "taskType"] },
  { title: "Progress", fields: ["percent", "actualStart", "actualFinish", "baselineStart", "baselineFinish", "finishVar"] },
];

/**
 * The selected task's attributes, beside the Gantt Chart or the WBS: everything about it at a
 * glance, and the common fields editable in place (Return or leaving the field saves).
 */
export function TaskDetails({
  snapshot,
  taskId,
  count,
  onEdit,
  onMore,
  onClose,
}: {
  snapshot: Snapshot;
  taskId: string | null;
  /** How many tasks are selected (the panel shows the first). */
  count: number;
  onEdit: (e: Edit) => Promise<EditResult>;
  onMore: (r: Row) => void;
  onClose: () => void;
}) {
  const row = taskId ? snapshot.rows.find((r) => r.id === taskId) : undefined;
  const perDay = snapshot.project.minutesPerDay;
  const currency = snapshot.project.currency;
  const successors = row
    ? snapshot.links
        .filter((l) => l.predecessorId === row.id)
        .map((l) => snapshot.rows.findIndex((r) => r.id === l.successorId) + 1)
        .filter((n) => n > 0)
        .join(", ")
    : "";

  return (
    <aside className="task-details" aria-label="Task details">
      <header>
        <h2>{row ? "Task" : "Details"}</h2>
        <button className="link small" onClick={onClose} aria-label="Hide details" title="Hide details">
          ✕
        </button>
      </header>
      {!row ? (
        <p className="secondary small">Select a task to see everything about it here.</p>
      ) : (
        <div className="details-body" key={row.id}>
          <p className="details-wbs secondary small">
            {row.wbs} · row {snapshot.rows.indexOf(row) + 1}
            {row.isSummary ? " · summary" : row.isMilestone ? " · milestone" : ""}
            {count > 1 && ` · ${count} selected`}
          </p>
          <Field row={row} snapshot={snapshot} column={columnById("name")!} onEdit={onEdit} big />
          <div className="details-progress" aria-hidden>
            <span style={{ width: `${row.percentComplete}%` }} />
          </div>
          {SECTIONS.map((s) => (
            <section key={s.title}>
              <h3>{s.title}</h3>
              <dl>
                {s.fields.map((id) => {
                  const c = columnById(id);
                  if (!c) return null;
                  const shown = c.text(row, perDay, currency);
                  // Empty read-only fields are left out, so the panel stays short.
                  if (!c.edit && (shown === "" || shown === "No")) return null;
                  if (row.isSummary && c.edit && ["percent", "resources", "work", "mode", "taskType", "fixedCost", "actualStart", "actualFinish"].includes(id)) {
                    if (!shown) return null;
                    return <Pair key={id} label={c.label} value={shown} />;
                  }
                  return (
                    <div key={id} className="details-pair">
                      <dt>{c.label}</dt>
                      <dd>
                        <Field row={row} snapshot={snapshot} column={c} onEdit={onEdit} />
                      </dd>
                    </div>
                  );
                })}
                {s.title === "Schedule" && successors && <Pair label="Successors" value={successors} />}
              </dl>
            </section>
          ))}
          <section>
            <h3>Notes</h3>
            <Field row={row} snapshot={snapshot} column={columnById("notes")!} onEdit={onEdit} multiline />
          </section>
          <button onClick={() => onMore(row)}>All Task Info…</button>
        </div>
      )}
    </aside>
  );
}

function Pair({ label, value }: { label: string; value: string }) {
  return (
    <div className="details-pair">
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

/** One attribute: its text, or an editor when the column can be edited (saved on Return or blur). */
function Field({ row, snapshot, column: c, onEdit, big, multiline }: { row: Row; snapshot: Snapshot; column: Column; onEdit: (e: Edit) => Promise<EditResult>; big?: boolean; multiline?: boolean }) {
  const initial = c.edit ? c.edit.initial(row) : "";
  const [text, setText] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  // A change from elsewhere (the list, undo, a teammate) replaces what is shown, unless typing.
  const [focused, setFocused] = useState(false);
  useEffect(() => {
    if (!focused) setText(initial);
  }, [initial, focused]);

  const why = c.kind ? blocked(snapshot, c.kind, row) : null;
  if (!c.edit || why) {
    return <span title={why ?? c.title?.(row)}>{c.text(row, snapshot.project.minutesPerDay, snapshot.project.currency) || "—"}</span>;
  }
  const save = async () => {
    if (text === initial) return;
    try {
      await onEdit(c.edit!.make(row, text));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  // Shown as in the list ("Tue, 13 Oct 2026", "1 day") until clicked; then the typed form.
  const shown = c.text(row, snapshot.project.minutesPerDay, snapshot.project.currency);
  const common = {
    value: focused || multiline || error ? text : shown,
    "aria-label": c.label,
    "aria-invalid": error !== null,
    title: c.title?.(row),
    onFocus: (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      setFocused(true);
      setText(initial);
      const el = e.currentTarget;
      requestAnimationFrame(() => el.select());
    },
    onBlur: () => {
      setFocused(false);
      void save();
    },
  };
  return (
    <>
      {multiline ? (
        <textarea {...common} rows={4} placeholder="Anything to remember about this task" onChange={(e) => setText(e.target.value)} />
      ) : (
        <input
          {...common}
          className={big ? "details-name" : undefined}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void save();
            if (e.key === "Escape") {
              setText(initial);
              setError(null);
            }
          }}
        />
      )}
      {error && (
        <span role="alert" className="error small">
          {error}
        </span>
      )}
    </>
  );
}
