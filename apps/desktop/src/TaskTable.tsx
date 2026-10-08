import type { Snapshot } from "./api";
import { formatDate, formatDateTime, formatDays } from "./format";

/** A plain, read-only table of the schedule (step 5). The editable grid replaces it in step 6. */
export function TaskTable({ snapshot }: { snapshot: Snapshot }) {
  const perDay = snapshot.project.minutesPerDay;
  const flagged = new Set([...(snapshot.problem?.taskIds ?? []), ...snapshot.conflicts.map((c) => c.taskId)]);
  return (
    <table className="tasks" aria-label="Tasks">
      <thead>
        <tr>
          <th className="num">#</th>
          <th>Task</th>
          <th className="num">Duration</th>
          <th>Start</th>
          <th>Finish</th>
          <th className="num">Slack</th>
          <th>Critical</th>
        </tr>
      </thead>
      <tbody>
        {snapshot.rows.map((r, i) => (
          <tr key={r.id} className={[r.isSummary && "summary", flagged.has(r.id) && "flagged"].filter(Boolean).join(" ")}>
            <td className="num secondary">{i + 1}</td>
            <td style={{ paddingLeft: `${8 + r.depth * 18}px` }}>
              {r.isMilestone && <span aria-label="Milestone" title="Milestone">◆ </span>}
              {r.name}
            </td>
            <td className="num">{r.isMilestone ? "Milestone" : formatDays(r.durationMinutes, perDay)}</td>
            <td title={formatDateTime(r.start)}>{formatDate(r.start)}</td>
            <td title={formatDateTime(r.finish)}>{formatDate(r.finish)}</td>
            <td className="num">{formatDays(r.totalSlackMinutes, perDay)}</td>
            {/* Text and a marker, not colour alone (spec 5.1.9). */}
            <td>{r.critical ? <span className="critical">▲ Critical</span> : ""}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
