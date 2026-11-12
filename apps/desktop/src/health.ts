// Where a project stands, worked out once from the snapshot, for the Dashboard, the progress
// bar in the header and the Status Report, so they always agree.

import type { Row } from "./bindings/Row";
import type { Snapshot } from "./bindings/Snapshot";

const DAY_MS = 86_400_000;

export type Status = "onTrack" | "atRisk" | "late" | "done";
export const STATUS_LABEL: Record<Status, string> = { onTrack: "On track", atRisk: "At risk", late: "Late", done: "Done" };

/** "2026-11-12T10:00:00" in local time, comparable with the snapshot's dates. */
export function localNow(offsetDays = 0): string {
  const d = new Date(Date.now() + offsetDays * DAY_MS);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:00`;
}

const time = (iso: string) => Date.parse(iso.length === 19 ? `${iso}Z` : iso);

export interface Health {
  status: Status;
  /** Why, in plain words; empty when on track. */
  reasons: string[];
  /** Work done, weighted by duration (0–100). */
  percentDone: number;
  /** How much should be done by now if work followed the plan (baseline, or the schedule). */
  percentPlanned: number;
  tasks: number;
  tasksDone: number;
  /** Days the finish moved against the baseline (positive: later); null without a baseline. */
  slipDays: number | null;
  late: Row[];
  missedDeadlines: Row[];
  /** Not finished and due in the next 7 days. */
  dueThisWeek: Row[];
  /** Started or due to start, but waiting on a predecessor that is late. */
  blocked: Row[];
  milestones: Row[];
  baselineCostCents: number;
}

/** How far a task should be by `now`, by its plan (baseline dates if any, else scheduled). */
function plannedFraction(r: Row, now: number): number {
  const start = r.baselineStart ?? r.start;
  const finish = r.baselineFinish ?? r.finish;
  if (!start || !finish) return 0;
  const s = time(start);
  const f = time(finish);
  if (now >= f) return 1;
  if (now <= s || f <= s) return 0;
  return (now - s) / (f - s);
}

export function health(snapshot: Snapshot): Health {
  const nowText = localNow();
  const now = time(nowText);
  const week = localNow(7);
  const leaves = snapshot.rows.filter((r) => !r.isSummary);
  const weight = (r: Row) => Math.max(r.durationMinutes, 1);
  const total = leaves.reduce((s, r) => s + weight(r), 0);
  const percentDone = total ? Math.round(leaves.reduce((s, r) => s + weight(r) * r.percentComplete, 0) / total) : 0;
  const percentPlanned = total ? Math.round((leaves.reduce((s, r) => s + weight(r) * plannedFraction(r, now), 0) / total) * 100) : 0;
  const p = snapshot.project;
  const slipDays = p.baselineFinish && p.finish ? Math.round((time(p.finish) - time(p.baselineFinish)) / DAY_MS) : null;
  const late = leaves.filter((r) => r.percentComplete < 100 && r.finish !== null && r.finish <= nowText);
  const lateIds = new Set(late.map((r) => r.id));
  const missedDeadlines = leaves.filter((r) => r.missedDeadline);
  const dueThisWeek = leaves.filter((r) => r.percentComplete < 100 && r.finish !== null && r.finish > nowText && r.finish <= week).sort((a, b) => (a.finish ?? "").localeCompare(b.finish ?? ""));
  const waitsOnLate = new Set(snapshot.links.filter((l) => lateIds.has(l.predecessorId)).map((l) => l.successorId));
  const blocked = leaves.filter((r) => r.percentComplete < 100 && waitsOnLate.has(r.id) && !lateIds.has(r.id));
  const milestones = leaves.filter((r) => r.isMilestone && r.percentComplete < 100).sort((a, b) => (a.start ?? "").localeCompare(b.start ?? ""));
  const overbooked = snapshot.resources.filter((r) => r.overloads.length > 0);
  const baselineCostCents = leaves.reduce((s, r) => s + (r.baselineCostCents ?? 0), 0);

  const reasons: string[] = [];
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  if (slipDays !== null && slipDays > 0) reasons.push(`finishes ${plural(slipDays, "day", "days")} later than the baseline`);
  if (late.length) reasons.push(`${plural(late.length, "task", "tasks")} should have finished by now`);
  if (missedDeadlines.length) reasons.push(`${plural(missedDeadlines.length, "task finishes", "tasks finish")} after its deadline`);
  const behind = percentPlanned - percentDone;
  const isLate = reasons.length > 0;
  if (!isLate) {
    if (behind >= 10) reasons.push(`${behind} points behind plan (${percentDone}% done, ${percentPlanned}% planned by now)`);
    if (overbooked.length) reasons.push(`${plural(overbooked.length, "person has", "people have")} more work than time`);
  }
  const status: Status = leaves.length && percentDone === 100 ? "done" : isLate ? "late" : reasons.length ? "atRisk" : "onTrack";
  return {
    status,
    reasons,
    percentDone,
    percentPlanned,
    tasks: leaves.length,
    tasksDone: leaves.filter((r) => r.percentComplete === 100).length,
    slipDays,
    late,
    missedDeadlines,
    dueThisWeek,
    blocked,
    milestones,
    baselineCostCents,
  };
}
