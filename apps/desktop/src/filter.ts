import type { Row } from "./bindings/Row";

/** What the task list shows (View > Show, or the Show menu above the list). */
export type TaskFilter = "all" | "incomplete" | "late" | "deadline" | "critical" | "milestones" | `resource:${string}`;

export const FILTERS: { value: TaskFilter; label: string }[] = [
  { value: "all", label: "All tasks" },
  { value: "incomplete", label: "Not finished" },
  { value: "late", label: "Late (should have finished)" },
  { value: "deadline", label: "Past their deadline" },
  { value: "critical", label: "Critical path" },
  { value: "milestones", label: "Milestones" },
];

/** "2026-11-11T10:00:00" for now, comparable with rows' dates. */
function nowText(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:00`;
}

function matches(r: Row, filter: TaskFilter, now: string): boolean {
  if (r.isSummary) return false; // summaries show when something inside them does
  switch (filter) {
    case "all":
      return true;
    case "incomplete":
      return r.percentComplete < 100;
    case "late":
      return r.percentComplete < 100 && r.finish !== null && r.finish <= now;
    case "deadline":
      return r.missedDeadline;
    case "critical":
      return r.critical;
    case "milestones":
      return r.isMilestone;
    default: {
      const name = filter.slice("resource:".length).toLocaleLowerCase();
      return r.resourceNames
        .split(",")
        .map((n) => n.replace(/\[.*\]/, "").trim().toLocaleLowerCase())
        .includes(name);
    }
  }
}

/**
 * The rows a filter keeps, in order: matching tasks and the summaries above them (so each task
 * shows where it belongs), as Microsoft Project does.
 */
export function filterRows(rows: Row[], filter: TaskFilter, now = nowText()): Row[] {
  if (filter === "all") return rows;
  const byId = new Map(rows.map((r) => [r.id, r]));
  const keep = new Set<string>();
  for (const r of rows) {
    if (!matches(r, filter, now)) continue;
    let cur: Row | undefined = r;
    while (cur && !keep.has(cur.id)) {
      keep.add(cur.id);
      cur = cur.parentId ? byId.get(cur.parentId) : undefined;
    }
  }
  return rows.filter((r) => keep.has(r.id));
}

/** The rows left once the tasks inside collapsed summaries are hidden (the outline, as in
 * Microsoft Project). */
export function collapseRows(rows: Row[], collapsed: ReadonlySet<string>): Row[] {
  if (!collapsed.size) return rows;
  const out: Row[] = [];
  let hideBelow: number | null = null;
  for (const r of rows) {
    if (hideBelow !== null) {
      if (r.depth > hideBelow) continue;
      hideBelow = null;
    }
    out.push(r);
    if (r.isSummary && collapsed.has(r.id)) hideBelow = r.depth;
  }
  return out;
}

/** The summaries to collapse so that `levels` outline levels show (Show > Level 1, 2…). */
export function collapsedForLevels(rows: Row[], levels: number): Set<string> {
  return new Set(rows.filter((r) => r.isSummary && r.depth >= levels - 1).map((r) => r.id));
}
