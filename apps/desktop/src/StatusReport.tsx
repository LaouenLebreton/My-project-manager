import { useEffect, useRef } from "react";
import { api, errorMessage } from "./api";
import type { Row } from "./bindings/Row";
import type { Snapshot } from "./bindings/Snapshot";
import { formatDate, formatMoney } from "./format";
import { filterRows } from "./filter";

const DAY_MS = 86_400_000;

function nowText(offsetDays = 0): string {
  const d = new Date(Date.now() + offsetDays * DAY_MS);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:00`;
}

function TaskTable({ rows, empty, extra }: { rows: Row[]; empty: string; extra?: (r: Row) => string }) {
  if (rows.length === 0) return <p className="secondary">{empty}</p>;
  return (
    <table className="report-table">
      <thead>
        <tr>
          <th>Task</th>
          <th>Finish</th>
          <th className="right">Done</th>
          <th>Who</th>
          {extra && <th />}
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id}>
            <td>{r.name}</td>
            <td>{formatDate(r.finish)}</td>
            <td className="right">{r.percentComplete}%</td>
            <td>{r.resourceNames}</td>
            {extra && <td className="secondary">{extra(r)}</td>}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * Project > Status Report and File > Print: where the project stands, on one page. Printing
 * prints only this report (see `@media print` in styles.css).
 */
export function StatusReport({ snapshot, printNow, onClose }: { snapshot: Snapshot; printNow: boolean; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const print = () => void api.printWindow().catch((e) => alert(errorMessage(e)));
  useEffect(() => {
    ref.current?.showModal();
    if (printNow) setTimeout(print, 50);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const p = snapshot.project;
  const name = p.name || "Project";
  const leaves = snapshot.rows.filter((r) => !r.isSummary);
  const total = leaves.reduce((s, r) => s + Math.max(r.durationMinutes, 1), 0);
  const done = total ? Math.round(leaves.reduce((s, r) => s + Math.max(r.durationMinutes, 1) * r.percentComplete, 0) / total) : 0;
  const baselineCost = leaves.reduce((s, r) => s + (r.baselineCostCents ?? 0), 0);
  const slipDays = p.baselineFinish && p.finish ? Math.round((Date.parse(p.finish) - Date.parse(p.baselineFinish)) / DAY_MS) : null;
  const late = filterRows(snapshot.rows, "late").filter((r) => !r.isSummary);
  const missed = leaves.filter((r) => r.missedDeadline);
  const now = nowText();
  const soon = nowText(14);
  const coming = leaves.filter((r) => r.percentComplete < 100 && r.finish !== null && r.finish > now && r.finish <= soon).sort((a, b) => (a.finish ?? "").localeCompare(b.finish ?? ""));
  const milestones = leaves.filter((r) => r.isMilestone && r.percentComplete < 100 && (r.start ?? "") >= now.slice(0, 10)).slice(0, 6);
  const overbooked = snapshot.resources.filter((r) => r.overloads.length > 0);
  const status = late.length || missed.length || (slipDays ?? 0) > 0 ? "Late" : overbooked.length ? "At risk" : done === 100 ? "Done" : "On track";

  return (
    <dialog ref={ref} className="report sheet status-report" onClose={onClose} aria-labelledby="status-title">
      <header className="status-report-head">
        <div>
          <h2 id="status-title">{name}: status report</h2>
          <p className="secondary">
            {new Date().toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" })}
            {p.statusDate && ` · progress as of ${formatDate(p.statusDate)}`}
          </p>
        </div>
        <span className={`status-chip ${status === "Late" ? "late" : status === "At risk" ? "atRisk" : status === "Done" ? "done" : "onTrack"}`}>{status}</span>
      </header>

      <dl className="status-facts">
        <dt>Dates</dt>
        <dd>
          {formatDate(p.start)} – {formatDate(p.finish)}
          {slipDays !== null && slipDays !== 0 && <span className={slipDays > 0 ? "late-text" : "secondary"}> ({slipDays > 0 ? `${slipDays} days later` : `${-slipDays} days earlier`} than the baseline)</span>}
        </dd>
        <dt>Done</dt>
        <dd>
          {done}% · {leaves.filter((r) => r.percentComplete === 100).length} of {leaves.length} tasks finished
        </dd>
        <dt>Cost</dt>
        <dd>
          {formatMoney(p.costCents, p.currency)}
          {baselineCost > 0 && <span className="secondary"> (baseline {formatMoney(baselineCost, p.currency)})</span>}
        </dd>
      </dl>

      <h3>Late ({late.length})</h3>
      <TaskTable rows={late} empty="Nothing is late." />
      {missed.length > 0 && (
        <>
          <h3>Finishing after their deadline ({missed.length})</h3>
          <TaskTable rows={missed} empty="" extra={(r) => `deadline ${formatDate(r.deadline)}`} />
        </>
      )}
      <h3>Due in the next two weeks ({coming.length})</h3>
      <TaskTable rows={coming} empty="Nothing is due in the next two weeks." />
      <h3>Next milestones</h3>
      {milestones.length === 0 ? (
        <p className="secondary">No milestones ahead.</p>
      ) : (
        <ul>
          {milestones.map((m) => (
            <li key={m.id}>
              ◆ {m.name} · {formatDate(m.start)}
            </li>
          ))}
        </ul>
      )}
      <h3>People with more work than time</h3>
      {overbooked.length === 0 ? <p className="secondary">Nobody is overbooked.</p> : <p>{overbooked.map((r) => r.name).join(", ")}</p>}

      <div className="report-actions no-print">
        <button type="button" onClick={() => ref.current?.close()}>
          Close
        </button>
        <button type="button" className="primary" onClick={print}>
          Print…
        </button>
      </div>
    </dialog>
  );
}
