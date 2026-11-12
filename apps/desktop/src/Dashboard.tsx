import { useEffect, useMemo, useState } from "react";
import { api, team } from "./api";
import type { Row } from "./bindings/Row";
import type { Snapshot } from "./bindings/Snapshot";
import { formatDate, formatMoney } from "./format";
import { health, STATUS_LABEL, type Health } from "./health";

/**
 * The progress of the project in one line, for the header: work done against work planned by
 * now, with the status colour. Shown on every view.
 */
export function ProgressBar({ snapshot }: { snapshot: Snapshot }) {
  const h = useMemo(() => health(snapshot), [snapshot]);
  if (h.tasks === 0) return null;
  const label = `${h.percentDone}% done; ${h.percentPlanned}% planned by now. ${STATUS_LABEL[h.status]}${h.reasons.length ? `: ${h.reasons.join("; ")}` : ""}`;
  return (
    <div className={`project-progress ${h.status}`} title={label} role="meter" aria-label="Project progress" aria-valuenow={h.percentDone} aria-valuemin={0} aria-valuemax={100} aria-valuetext={label}>
      <div className="track">
        <span className="done" style={{ width: `${h.percentDone}%` }} />
        {h.percentPlanned > 0 && h.percentPlanned < 100 && <span className="planned" style={{ left: `${h.percentPlanned}%` }} />}
      </div>
      <span className="small">
        {h.percentDone}% <span className="secondary">· {STATUS_LABEL[h.status]}</span>
      </span>
    </div>
  );
}

function TaskList({ rows, empty, onShow, detail, rowNumber }: { rows: Row[]; empty: string; onShow: (row: number) => void; detail: (r: Row) => string; rowNumber: (r: Row) => number }) {
  if (rows.length === 0) return <p className="secondary small">{empty}</p>;
  return (
    <ul className="dash-list">
      {rows.slice(0, 6).map((r) => (
        <li key={r.id}>
          <button className="link" onClick={() => onShow(rowNumber(r))}>
            {r.name}
          </button>
          <span className="secondary small">{detail(r)}</span>
        </li>
      ))}
      {rows.length > 6 && <li className="secondary small">and {rows.length - 6} more</li>}
    </ul>
  );
}

function Card({ title, children, wide }: { title: string; children: React.ReactNode; wide?: boolean }) {
  return (
    <section className={wide ? "dash-card wide" : "dash-card"} aria-label={title}>
      <h2>{title}</h2>
      {children}
    </section>
  );
}

interface Change {
  what: string;
  who: string | null;
  at: string;
}

/** The latest changes: the team's history in a team project, otherwise this Mac's undo history. */
function useRecentChanges(snapshot: Snapshot): Change[] | null {
  const [changes, setChanges] = useState<Change[] | null>(null);
  const shared = snapshot.project.team !== null;
  const key = `${snapshot.undoLabel}|${snapshot.rows.length}`;
  useEffect(() => {
    let live = true;
    const done = (c: Change[]) => live && setChanges(c);
    if (shared) {
      team.history(null).then((page) => done(page.entries.slice(0, 6).map((e) => ({ what: e.summary, who: e.authorName, at: e.at }))), () => done([]));
    } else {
      api.undoHistory().then((steps) => done(steps.filter((s) => !s.undone).slice(0, 6).map((s) => ({ what: s.label, who: null, at: s.at }))), () => done([]));
    }
    return () => {
      live = false;
    };
  }, [shared, key]);
  return changes;
}

function when(at: string): string {
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return "";
  const today = d.toDateString() === new Date().toDateString();
  return today ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }) : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

/**
 * The Dashboard: the project at a glance, built around what project managers say they look for
 * (see docs/iteration-5-plan.md): one health signal with its reasons, progress against plan,
 * what is late, blocked or due this week, milestones, who is overbooked, money against the
 * budget, and what changed lately.
 */
export function Dashboard({ snapshot, onShowTask, onOpen }: { snapshot: Snapshot; onShowTask: (row: number) => void; onOpen: (view: "tasks" | "workload" | "history") => void }) {
  const h: Health = useMemo(() => health(snapshot), [snapshot]);
  const changes = useRecentChanges(snapshot);
  const rowOf = useMemo(() => new Map(snapshot.rows.map((r, i) => [r.id, i + 1])), [snapshot.rows]);
  const rowNumber = (r: Row) => rowOf.get(r.id) ?? 1;
  const p = snapshot.project;
  const ev = p.earnedValue;
  const overbooked = snapshot.resources.filter((r) => r.overloads.length > 0);
  const shared = p.team !== null;

  return (
    <main className="dashboard" aria-label="Dashboard">
      <Card title="Health" wide>
        <div className="dash-health">
          <span className={`status-chip big ${h.status}`}>{STATUS_LABEL[h.status]}</span>
          <div>
            {h.reasons.length ? (
              <ul className="dash-reasons">
                {h.reasons.map((r) => (
                  <li key={r}>{r.charAt(0).toUpperCase() + r.slice(1)}</li>
                ))}
              </ul>
            ) : (
              <p>Nothing needs attention.</p>
            )}
          </div>
        </div>
      </Card>

      <Card title="Progress">
        <div className="dash-progress" aria-hidden>
          <div className="track">
            <span className="done" style={{ width: `${h.percentDone}%` }} />
            {h.percentPlanned > 0 && <span className="planned" style={{ left: `${Math.min(h.percentPlanned, 100)}%` }} />}
          </div>
        </div>
        <p className="dash-big">{h.percentDone}% done</p>
        <p className="secondary small">
          {h.percentPlanned}% planned by now · {h.tasksDone} of {h.tasks} tasks finished
        </p>
      </Card>

      <Card title="Finish">
        <p className="dash-big">{formatDate(p.finish)}</p>
        <p className={h.slipDays && h.slipDays > 0 ? "late-text small" : "secondary small"}>
          {h.slipDays === null ? "Save a baseline to see how far it moves." : h.slipDays === 0 ? "On the baseline date." : h.slipDays > 0 ? `${h.slipDays} days later than the baseline` : `${-h.slipDays} days earlier than the baseline`}
        </p>
      </Card>

      <Card title="Budget">
        <p className="dash-big">{formatMoney(p.costCents, p.currency)}</p>
        <p className="secondary small">
          {h.baselineCostCents > 0 ? `Baseline ${formatMoney(h.baselineCostCents, p.currency)}` : "No baseline cost yet."}
          {ev && ev.cpi !== null && ` · CPI ${ev.cpi.toFixed(2)} · SPI ${ev.spi?.toFixed(2) ?? "—"}`}
        </p>
      </Card>

      <Card title={`Late (${h.late.length})`}>
        <TaskList rows={h.late} empty="Nothing is late." onShow={onShowTask} rowNumber={rowNumber} detail={(r) => `due ${formatDate(r.finish)} · ${r.percentComplete}%`} />
      </Card>

      <Card title={`Due this week (${h.dueThisWeek.length})`}>
        <TaskList rows={h.dueThisWeek} empty="Nothing is due in the next 7 days." onShow={onShowTask} rowNumber={rowNumber} detail={(r) => `${formatDate(r.finish)}${r.resourceNames ? ` · ${r.resourceNames}` : ""}`} />
      </Card>

      <Card title={`Blocked (${h.blocked.length})`}>
        <TaskList rows={h.blocked} empty="Nothing is waiting on a late task." onShow={onShowTask} rowNumber={rowNumber} detail={(r) => `waits on a late task · ${r.predecessors}`} />
      </Card>

      <Card title="Next milestones">
        <TaskList rows={h.milestones} empty="No milestones ahead." onShow={onShowTask} rowNumber={rowNumber} detail={(r) => `${formatDate(r.start)}${r.missedDeadline ? " · after its deadline" : ""}`} />
      </Card>

      {h.missedDeadlines.length > 0 && (
        <Card title={`After their deadline (${h.missedDeadlines.length})`}>
          <TaskList rows={h.missedDeadlines} empty="" onShow={onShowTask} rowNumber={rowNumber} detail={(r) => `deadline ${formatDate(r.deadline)} · finishes ${formatDate(r.finish)}`} />
        </Card>
      )}

      <Card title="People">
        {snapshot.resources.length === 0 ? (
          <p className="secondary small">Nobody is assigned yet. Type names in the Resources column.</p>
        ) : overbooked.length === 0 ? (
          <p className="small">Nobody has more work than time.</p>
        ) : (
          <ul className="dash-list">
            {overbooked.map((r) => (
              <li key={r.id}>
                <button className="link" onClick={() => onOpen("workload")}>
                  {r.name}
                </button>
                <span className="late-text small">up to {Math.max(...r.overloads.map((o) => o.peakPercent))}% from {formatDate(r.overloads[0].start)}</span>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card title="Recent changes">
        {changes === null ? (
          <p className="secondary small">Loading…</p>
        ) : changes.length === 0 ? (
          <p className="secondary small">No changes yet.</p>
        ) : (
          <ul className="dash-list">
            {changes.map((c, i) => (
              <li key={i}>
                <span>
                  {c.who && <strong>{c.who} </strong>}
                  {c.what}
                </span>
                <span className="secondary small">{when(c.at)}</span>
              </li>
            ))}
          </ul>
        )}
        {shared && (
          <button className="link small" onClick={() => onOpen("history")}>
            All changes
          </button>
        )}
      </Card>
    </main>
  );
}
