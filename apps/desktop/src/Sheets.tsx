// Task Info, Project Info and Working Time: the advanced fields, shown when asked for
// (progressive disclosure, spec 5.1.8). Each saves as one undoable edit.

import { useEffect, useRef, useState } from "react";
import { errorMessage, type Edit } from "./api";
import type { ConstraintKind } from "./bindings/ConstraintKind";
import type { Row } from "./bindings/Row";
import type { Snapshot } from "./bindings/Snapshot";
import type { TimeRange } from "./bindings/TimeRange";
import { GLOSSARY } from "./help";

type Send = (edit: Edit) => Promise<unknown>;

/** A modal sheet: Escape or Cancel closes; Save sends the edit and closes unless refused. */
function Sheet({ title, onClose, onSave, children }: { title: string; onClose: () => void; onSave: () => Promise<string | null>; children: React.ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => ref.current?.showModal(), []);
  return (
    <dialog ref={ref} className="report sheet" onClose={onClose} aria-labelledby="sheet-title">
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          const problem = await onSave();
          if (problem) setError(problem);
          else ref.current?.close();
        }}
      >
        <h2 id="sheet-title">{title}</h2>
        {children}
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        <div className="report-actions">
          <button type="button" onClick={() => ref.current?.close()}>
            Cancel
          </button>
          <button className="primary" type="submit">
            Save
          </button>
        </div>
      </form>
    </dialog>
  );
}

/** "1 working day", "2.5 working days", "3 working hours". */
function leveledBy(minutes: number): string {
  if (minutes >= 480) {
    const days = Math.round((minutes / 480) * 100) / 100;
    return `${days} working ${days === 1 ? "day" : "days"}`;
  }
  const hours = Math.round((minutes / 60) * 100) / 100;
  return `${hours} working ${hours === 1 ? "hour" : "hours"}`;
}

async function attempt(send: Send, edit: Edit): Promise<string | null> {
  try {
    await send(edit);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

export function TaskInfo({ row, send, onClose }: { row: Row; send: Send; onClose: () => void }) {
  const [name, setName] = useState(row.name);
  const [duration, setDuration] = useState(row.isMilestone ? "1d" : row.durationText);
  const [milestone, setMilestone] = useState(row.isMilestone);
  const [kind, setKind] = useState<ConstraintKind>(row.constraint.kind);
  const [date, setDate] = useState(row.constraint.date ?? row.start?.slice(0, 10) ?? "");
  const [percent, setPercent] = useState(String(row.percentComplete));
  const [notes, setNotes] = useState(row.notes);
  const [resources, setResources] = useState(row.resourceNames);
  return (
    <Sheet
      title="Task Info"
      onClose={onClose}
      onSave={() =>
        attempt(send, {
          kind: "UpdateTask",
          id: row.id,
          name,
          durationText: row.isSummary ? null : duration,
          milestone: row.isSummary ? null : milestone,
          constraint: { kind, date: kind === "ASAP" ? null : date },
          percentComplete: Math.max(0, Math.min(100, Math.round(Number(percent) || 0))),
          notes,
          resources: row.isSummary ? null : resources,
        })
      }
    >
      <div className="form">
        <label htmlFor="ti-name">Name</label>
        <input id="ti-name" value={name} onChange={(e) => setName(e.target.value)} autoFocus />

        {!row.isSummary && (
          <>
            <label htmlFor="ti-duration">Duration</label>
            <div>
              <input id="ti-duration" value={duration} disabled={milestone} onChange={(e) => setDuration(e.target.value)} size={8} />
              <label className="inline">
                <input type="checkbox" checked={milestone} onChange={(e) => setMilestone(e.target.checked)} /> Milestone
              </label>
            </div>
          </>
        )}

        <label htmlFor="ti-constraint">Start</label>
        <div>
          <select id="ti-constraint" value={kind} onChange={(e) => setKind(e.target.value as ConstraintKind)}>
            <option value="ASAP">As soon as possible</option>
            <option value="SNET">No earlier than</option>
            <option value="MSO">Must start on</option>
          </select>
          {kind !== "ASAP" && <input type="date" aria-label="Constraint date" value={date} onChange={(e) => setDate(e.target.value)} required />}
          <p className="secondary small">
            {kind === "MSO"
              ? "The task starts on this date even if its predecessors finish later; Super PM then warns about the conflict."
              : kind === "SNET"
                ? GLOSSARY.constraint.text
                : "The task starts as soon as its predecessors allow."}
          </p>
        </div>

        {!row.isSummary && (
          <>
            <label htmlFor="ti-resources">Resources</label>
            <div>
              <input id="ti-resources" value={resources} onChange={(e) => setResources(e.target.value)} placeholder="Alex, Sam[50%]" />
              <p className="secondary small">{GLOSSARY.resources.text}</p>
            </div>
          </>
        )}

        {row.levelingDelayMinutes > 0 && (
          <>
            <span className="form-label">Leveling</span>
            <p className="secondary small">
              Starts {leveledBy(row.levelingDelayMinutes)} later than it could, so its resources are not over-allocated. Project &gt; Clear Leveling removes
              this.
            </p>
          </>
        )}

        <label htmlFor="ti-percent">% complete</label>
        <input id="ti-percent" type="number" min={0} max={100} value={percent} onChange={(e) => setPercent(e.target.value)} />

        <label htmlFor="ti-notes">Notes</label>
        <textarea id="ti-notes" rows={4} value={notes} onChange={(e) => setNotes(e.target.value)} />
      </div>
    </Sheet>
  );
}

export function ProjectInfo({ snapshot, send, onClose }: { snapshot: Snapshot; send: Send; onClose: () => void }) {
  const [start, setStart] = useState(snapshot.project.start.slice(0, 10));
  return (
    <Sheet title="Project Info" onClose={onClose} onSave={() => attempt(send, { kind: "SetProjectStart", date: start })}>
      <div className="form">
        <label htmlFor="pi-start">Project starts</label>
        <div>
          <input id="pi-start" type="date" value={start} onChange={(e) => setStart(e.target.value)} required autoFocus />
          <p className="secondary small">Tasks without predecessors start here, at the first working hour on or after this day.</p>
        </div>
      </div>
    </Sheet>
  );
}

const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

function time(m: number): string {
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

/** "08:00–12:00, 13:00–17:00" for display and editing. */
function rangesText(ranges: TimeRange[]): string {
  return ranges.map((r) => `${time(r.start)}–${time(r.end)}`).join(", ");
}

/** Reads "8-12, 13:00–17:30"; returns null if it cannot. Rust validates the result. */
function parseRanges(text: string): TimeRange[] | null {
  const parts = text.split(",").map((p) => p.trim()).filter(Boolean);
  const minutes = (t: string) => {
    const m = /^(\d{1,2})(?::(\d{2}))?$/.exec(t.trim());
    return m ? Number(m[1]) * 60 + Number(m[2] ?? 0) : NaN;
  };
  const ranges = parts.map((p) => {
    const [a, b] = p.split(/[-–]/);
    return { start: minutes(a ?? ""), end: minutes(b ?? "") };
  });
  return ranges.every((r) => Number.isFinite(r.start) && Number.isFinite(r.end)) ? ranges : null;
}

interface DayRow {
  working: boolean;
  text: string;
}

export function WorkingTime({ snapshot, send, onClose }: { snapshot: Snapshot; send: Send; onClose: () => void }) {
  const p = snapshot.project;
  const [week, setWeek] = useState<DayRow[]>(
    p.workingWeek.map((ranges) => ({ working: ranges.length > 0, text: ranges.length ? rangesText(ranges) : "08:00–12:00, 13:00–17:00" })),
  );
  const [exceptions, setExceptions] = useState(
    p.calendarExceptions.map((e) => ({ date: e.date, working: e.working, text: e.working ? rangesText(e.ranges) : "" })),
  );

  const save = async () => {
    const weekRanges: TimeRange[][] = [];
    for (const [i, d] of week.entries()) {
      const r = d.working ? parseRanges(d.text) : [];
      if (!r) return `${WEEKDAYS[i]}: type hours like 08:00–12:00, 13:00–17:00`;
      weekRanges.push(r);
    }
    const ex = [];
    for (const e of exceptions) {
      if (!e.date) return "Choose a date for each exception, or remove it";
      const r = e.working ? parseRanges(e.text) : [];
      if (!r) return `${e.date}: type hours like 09:00–13:00`;
      ex.push({ date: e.date, ranges: r });
    }
    return attempt(send, { kind: "SetCalendar", week: weekRanges, exceptions: ex });
  };

  return (
    <Sheet title="Working Time" onClose={onClose} onSave={save}>
      <h3>Working week</h3>
      <table className="working-week">
        <tbody>
          {week.map((d, i) => (
            <tr key={WEEKDAYS[i]}>
              <th scope="row">
                <label>
                  <input
                    type="checkbox"
                    checked={d.working}
                    onChange={(e) => setWeek(week.map((x, j) => (j === i ? { ...x, working: e.target.checked } : x)))}
                  />{" "}
                  {WEEKDAYS[i]}
                </label>
              </th>
              <td>
                <input
                  aria-label={`${WEEKDAYS[i]} working hours`}
                  value={d.working ? d.text : "Not working"}
                  disabled={!d.working}
                  onChange={(e) => setWeek(week.map((x, j) => (j === i ? { ...x, text: e.target.value } : x)))}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3>Holidays and changed days</h3>
      {exceptions.length === 0 && <p className="secondary small">None yet. Add public holidays, office closures or days with different hours.</p>}
      <table className="working-week">
        <tbody>
          {exceptions.map((e, i) => (
            <tr key={i}>
              <td>
                <input type="date" aria-label="Date" value={e.date} onChange={(ev) => setExceptions(exceptions.map((x, j) => (j === i ? { ...x, date: ev.target.value } : x)))} />
              </td>
              <td>
                <select
                  aria-label="Kind of day"
                  value={e.working ? "hours" : "off"}
                  onChange={(ev) =>
                    setExceptions(exceptions.map((x, j) => (j === i ? { ...x, working: ev.target.value === "hours", text: x.text || "09:00–13:00" } : x)))
                  }
                >
                  <option value="off">Not working</option>
                  <option value="hours">Working hours:</option>
                </select>{" "}
                {e.working && (
                  <input aria-label="Hours" value={e.text} size={14} onChange={(ev) => setExceptions(exceptions.map((x, j) => (j === i ? { ...x, text: ev.target.value } : x)))} />
                )}
              </td>
              <td>
                <button type="button" aria-label={`Remove ${e.date || "this day"}`} onClick={() => setExceptions(exceptions.filter((_, j) => j !== i))}>
                  −
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <button type="button" onClick={() => setExceptions([...exceptions, { date: "", working: false, text: "" }])}>
        Add Day
      </button>
    </Sheet>
  );
}
