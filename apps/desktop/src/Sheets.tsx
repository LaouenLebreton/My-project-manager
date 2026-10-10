// Task Info, Project Info and Working Time: the advanced fields, shown when asked for
// (progressive disclosure, spec 5.1.8). Each saves as one undoable edit.

import { useEffect, useRef, useState } from "react";
import { errorMessage, type Edit } from "./api";
import type { ConstraintKind } from "./bindings/ConstraintKind";
import type { Row } from "./bindings/Row";
import type { Snapshot } from "./bindings/Snapshot";
import type { TimeRange } from "./bindings/TimeRange";
import { formatDate, formatMoney } from "./format";
import { GLOSSARY } from "./help";


type Send = (edit: Edit) => Promise<unknown>;

/** Microsoft Project's eight constraint types, explained in plain words. */
export const CONSTRAINTS: Record<ConstraintKind, { label: string; dated: boolean; help: string }> = {
  ASAP: { label: "As soon as possible", dated: false, help: "The task starts as soon as its predecessors allow." },
  ALAP: { label: "As late as possible", dated: false, help: "The task starts as late as it can without delaying the tasks after it or the project's finish." },
  SNET: { label: "Start no earlier than", dated: true, help: "The task will not start before this date, even if its predecessors finish sooner. Links can still push it later." },
  SNLT: { label: "Start no later than", dated: true, help: "The task must start by this date. If its predecessors would make it later, Super PM keeps the date and warns about the conflict." },
  FNET: { label: "Finish no earlier than", dated: true, help: "The task will not finish before the end of this day." },
  FNLT: { label: "Finish no later than", dated: true, help: "The task must finish by the end of this day. If its predecessors would make it later, Super PM keeps the date and warns about the conflict." },
  MSO: { label: "Must start on", dated: true, help: "The task starts on this date even if its predecessors finish later; Super PM then warns about the conflict. Leveling never moves it." },
  MFO: { label: "Must finish on", dated: true, help: "The task finishes at the end of this day even if its predecessors finish later; Super PM then warns about the conflict. Leveling never moves it." },
};

/** A modal sheet: Escape or Cancel closes; Save sends the edit and closes unless refused.
 * Without `onSave` it only shows information, with a Done button. */
export function Sheet({
  title,
  onClose,
  onSave,
  action = "Save",
  children,
}: {
  title: string;
  onClose: () => void;
  onSave?: () => Promise<string | null>;
  /** The main button's label when it does more than save. */
  action?: string;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => ref.current?.showModal(), []);
  return (
    <dialog ref={ref} className="report sheet" onClose={onClose} aria-labelledby="sheet-title">
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          const problem = onSave ? await onSave() : null;
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
          {onSave && (
            <button type="button" onClick={() => ref.current?.close()}>
              Cancel
            </button>
          )}
          <button className="primary" type="submit">
            {onSave ? action : "Done"}
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
  const [manual, setManual] = useState(row.manual);
  const [fixedCost, setFixedCost] = useState(row.fixedCostCents ? (row.fixedCostCents / 100).toFixed(2) : "");
  const initialStart = row.actualStart?.slice(0, 10) ?? "";
  const initialFinish = row.actualFinish?.slice(0, 10) ?? "";
  const [actualStart, setActualStart] = useState(initialStart);
  const [actualFinish, setActualFinish] = useState(initialFinish);
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
          constraint: { kind, date: CONSTRAINTS[kind].dated ? date : null },
          percentComplete: Math.max(0, Math.min(100, Math.round(Number(percent) || 0))),
          notes,
          resources: row.isSummary ? null : resources,
          fixedCost,
          manual: row.isSummary ? null : manual,
          // Only typed dates are sent, so a changed % complete can fill them in from the schedule.
          actualStart: row.isSummary || actualStart === initialStart ? null : actualStart,
          actualFinish: row.isSummary || actualFinish === initialFinish ? null : actualFinish,
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

        {!row.isSummary && (
          <>
            <label htmlFor="ti-mode">Scheduling</label>
            <div>
              <select id="ti-mode" value={manual ? "manual" : "auto"} onChange={(e) => setManual(e.target.value === "manual")}>
                <option value="auto">Automatic</option>
                <option value="manual">Manual</option>
              </select>
              <p className="secondary small">
                {manual
                  ? "The task stays where you put it (drag its bar to move it). Links and constraints no longer move it; Super PM warns if a predecessor finishes too late."
                  : "Super PM works out the dates from links, constraints and the calendar."}
              </p>
            </div>
          </>
        )}

        <label htmlFor="ti-constraint">Start</label>
        <div>
          <select id="ti-constraint" value={kind} onChange={(e) => setKind(e.target.value as ConstraintKind)}>
            {(Object.keys(CONSTRAINTS) as ConstraintKind[]).map((k) => (
              <option key={k} value={k}>
                {CONSTRAINTS[k].label}
              </option>
            ))}
          </select>
          {CONSTRAINTS[kind].dated && <input type="date" aria-label="Constraint date" value={date} onChange={(e) => setDate(e.target.value)} required />}
          <p className="secondary small">{CONSTRAINTS[kind].help}</p>
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

        <label htmlFor="ti-fixed-cost">Fixed cost</label>
        <div>
          <input id="ti-fixed-cost" value={fixedCost} onChange={(e) => setFixedCost(e.target.value)} placeholder="0.00" size={12} />
          <p className="secondary small">A one-off cost on top of the resources, such as a venue fee or a permit.</p>
        </div>

        {row.baselineFinish && (
          <>
            <span className="form-label">Baseline</span>
            <p className="secondary small">
              Planned to finish {formatDate(row.baselineFinish)}
              {row.finishVarianceMinutes ? `; now ${leveledBy(Math.abs(row.finishVarianceMinutes))} ${row.finishVarianceMinutes > 0 ? "later" : "earlier"}` : "; still on plan"}.
            </p>
          </>
        )}

        <label htmlFor="ti-percent">% complete</label>
        <div>
          <input id="ti-percent" type="number" min={0} max={100} value={percent} onChange={(e) => setPercent(e.target.value)} />
          {!row.isSummary && <p className="secondary small">{GLOSSARY.progress.text}</p>}
        </div>

        {!row.isSummary && (
          <>
            <label htmlFor="ti-actual-start">Actual start</label>
            <input id="ti-actual-start" type="date" value={actualStart} onChange={(e) => setActualStart(e.target.value)} />
            <label htmlFor="ti-actual-finish">Actual finish</label>
            <input id="ti-actual-finish" type="date" value={actualFinish} onChange={(e) => setActualFinish(e.target.value)} />
          </>
        )}

        <label htmlFor="ti-notes">Notes</label>
        <textarea id="ti-notes" rows={4} value={notes} onChange={(e) => setNotes(e.target.value)} />
      </div>
    </Sheet>
  );
}

export function ProjectInfo({ snapshot, send, onClose }: { snapshot: Snapshot; send: Send; onClose: () => void }) {
  const [start, setStart] = useState(snapshot.project.start.slice(0, 10));
  const [currency, setCurrency] = useState(snapshot.project.currency);
  const initialStatus = snapshot.project.statusDate?.slice(0, 10) ?? "";
  const [statusDate, setStatusDate] = useState(initialStatus);
  const save = async () => {
    let problem = await attempt(send, { kind: "SetProjectStart", date: start });
    if (!problem && currency.trim() !== snapshot.project.currency) problem = await attempt(send, { kind: "SetCurrency", symbol: currency });
    if (!problem && statusDate !== initialStatus) problem = await attempt(send, { kind: "SetStatusDate", date: statusDate || null });
    return problem;
  };
  return (
    <Sheet title="Project Info" onClose={onClose} onSave={save}>
      <div className="form">
        <label htmlFor="pi-start">Project starts</label>
        <div>
          <input id="pi-start" type="date" value={start} onChange={(e) => setStart(e.target.value)} required autoFocus />
          <p className="secondary small">Tasks without predecessors start here, at the first working hour on or after this day.</p>
        </div>
        <label htmlFor="pi-currency">Currency</label>
        <div>
          <input id="pi-currency" value={currency} onChange={(e) => setCurrency(e.target.value)} placeholder="€, $, £, CHF" size={6} />
          <p className="secondary small">Shown with every cost in this project.</p>
        </div>
        <label htmlFor="pi-status">Status date</label>
        <div>
          <input id="pi-status" type="date" value={statusDate} onChange={(e) => setStatusDate(e.target.value)} />
          <p className="secondary small">The day progress is reported up to, drawn as a dashed line in the chart. Leave empty if you do not track progress.</p>
        </div>
      </div>
    </Sheet>
  );
}

/** Project > Update Project: record work as done on schedule up to a day. */
export function UpdateProject({ snapshot, send, onClose }: { snapshot: Snapshot; send: Send; onClose: () => void }) {
  const today = new Date();
  const fallback = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
  const [through, setThrough] = useState(snapshot.project.statusDate?.slice(0, 10) ?? fallback);
  return (
    <Sheet title="Update Project" onClose={onClose} onSave={() => attempt(send, { kind: "UpdateProgress", throughDate: through })}>
      <div className="form">
        <label htmlFor="up-through">Work done through</label>
        <div>
          <input id="up-through" type="date" value={through} onChange={(e) => setThrough(e.target.value)} required autoFocus />
          <p className="secondary small">
            Every task scheduled to start by the end of this day is marked as started on its scheduled start, with its % complete as far as the plan says. Tasks
            due to finish by then become 100% complete. Progress you already entered is never lowered. This day becomes the status date.
          </p>
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

/** "+€250" / "−€250" for variances. */
function signedMoney(cents: number, currency: string): string {
  return `${cents > 0 ? "+" : cents < 0 ? "−" : ""}${formatMoney(Math.abs(cents), currency)}`;
}

/** Project > Project Statistics: dates, costs and earned value at a glance. */
export function ProjectStatistics({ snapshot, onClose }: { snapshot: Snapshot; onClose: () => void }) {
  const p = snapshot.project;
  const money = (c: number) => formatMoney(c, p.currency);
  const ev = p.earnedValue;
  const index = (v: number | null, good: string, bad: string) =>
    v === null ? "—" : `${v.toFixed(2)} (${v >= 1 ? good : bad})`;
  return (
    <Sheet title="Project Statistics" onClose={onClose}>
      <table className="stats">
        <tbody>
          <tr>
            <th scope="row">Start</th>
            <td>{formatDate(p.start)}</td>
          </tr>
          <tr>
            <th scope="row">Finish</th>
            <td>
              {formatDate(p.finish)}
              {p.baselineFinish && ` (baseline ${formatDate(p.baselineFinish)})`}
            </td>
          </tr>
          <tr>
            <th scope="row">Cost</th>
            <td>{money(p.costCents)}</td>
          </tr>
          <tr>
            <th scope="row">Status date</th>
            <td>{p.statusDate ? formatDate(p.statusDate) : "Not set (today is used)"}</td>
          </tr>
        </tbody>
      </table>
      <h3>Earned value</h3>
      {ev ? (
        <>
          <table className="stats">
            <tbody>
              <tr>
                <th scope="row" title="BCWS: the baseline cost of the work that should be done by now">
                  Planned value
                </th>
                <td>{money(ev.bcws)}</td>
              </tr>
              <tr>
                <th scope="row" title="BCWP: the baseline cost of the work actually done">
                  Earned value
                </th>
                <td>{money(ev.bcwp)}</td>
              </tr>
              <tr>
                <th scope="row" title="ACWP: what the work done has cost">
                  Actual cost
                </th>
                <td>{money(ev.acwp)}</td>
              </tr>
              <tr>
                <th scope="row">Schedule variance (SV)</th>
                <td>{signedMoney(ev.bcwp - ev.bcws, p.currency)}</td>
              </tr>
              <tr>
                <th scope="row">Cost variance (CV)</th>
                <td>{signedMoney(ev.bcwp - ev.acwp, p.currency)}</td>
              </tr>
              <tr>
                <th scope="row">SPI</th>
                <td>{index(ev.spi, "on or ahead of schedule", "behind schedule")}</td>
              </tr>
              <tr>
                <th scope="row">CPI</th>
                <td>{index(ev.cpi, "on or under budget", "over budget")}</td>
              </tr>
            </tbody>
          </table>
          <p className="secondary small">{GLOSSARY.earned.text}</p>
        </>
      ) : (
        <p className="secondary small">Save a baseline (Project &gt; Set Baseline) to compare progress against the plan.</p>
      )}
    </Sheet>
  );
}

/** Asks before an action that may have consequences the user did not intend. */
export function ConfirmSheet({ title, message, action, onAnswer }: { title: string; message: string; action: string; onAnswer: (yes: boolean) => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const answered = useRef(false);
  useEffect(() => ref.current?.showModal(), []);
  const answer = (yes: boolean) => {
    if (answered.current) return;
    answered.current = true;
    ref.current?.close();
    onAnswer(yes);
  };
  return (
    <dialog ref={ref} className="report sheet" onClose={() => answer(false)} aria-labelledby="confirm-title">
      <h2 id="confirm-title">{title}</h2>
      <p>{message}</p>
      <div className="report-actions">
        <button type="button" onClick={() => answer(false)} autoFocus>
          Cancel
        </button>
        <button type="button" className="primary" onClick={() => answer(true)}>
          {action}
        </button>
      </div>
    </dialog>
  );
}
