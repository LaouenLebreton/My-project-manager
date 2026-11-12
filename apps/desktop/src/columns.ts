// The task list's columns: every one the column chooser offers, which are shown, in what order
// and how wide. The choice is kept on this computer (like Finder's view options), so each
// person arranges the list their way.

import type { Edit } from "./api";
import type { Row } from "./bindings/Row";
import type { TaskTypeEdit } from "./bindings/TaskTypeEdit";
import { formatDate, formatDateTime, formatDays, formatMoney } from "./format";
import type { Term } from "./help";
import type { Kind } from "./permissions";
import { CONSTRAINTS } from "./Sheets";

export interface Column {
  id: string;
  label: string;
  /** Width in pixels (the Task Name column also takes any spare room). */
  width: number;
  align?: "right";
  /** Where it is listed in the column chooser. */
  group: "General" | "Schedule" | "Progress" | "Cost" | "Baseline" | "Advanced";
  /** Text shown in the cell. */
  text: (r: Row, perDay: number, currency: string) => string;
  /** Text placed in the editor, and the edit to send. Absent for read-only columns. */
  edit?: { initial: (r: Row) => string; make: (r: Row, text: string) => Edit };
  title?: (r: Row) => string;
  /** "What does this mean?" for the header. */
  help?: Term;
  /** Anchor for the first-run tour. */
  tour?: string;
  /** What kind of change editing this column makes, for team limits. */
  kind?: Kind;
  /** Always shown, in this place (row number, indicators, name). */
  fixed?: boolean;
}

/** Task Info's edit with only the given fields changed. */
function update(r: Row, fields: Partial<Extract<Edit, { kind: "UpdateTask" }>>): Edit {
  return {
    kind: "UpdateTask",
    id: r.id,
    name: null,
    durationText: null,
    milestone: null,
    constraint: null,
    percentComplete: null,
    notes: null,
    resources: null,
    fixedCost: null,
    manual: null,
    actualStart: null,
    actualFinish: null,
    deadline: null,
    taskType: null,
    effortDriven: null,
    workText: null,
    ...fields,
  };
}

/** "2026-11-12", "12 Nov 2026", "Nov 12" (this year)… as `YYYY-MM-DD`; empty stays empty (clears).
 * Anything else is passed on as typed, and refused with a message saying what to type. */
export function typedDate(text: string): string {
  const t = text.trim();
  if (t === "" || /^\d{4}-\d{2}-\d{2}$/.test(t)) return t;
  const withYear = /\d{4}/.test(t) ? t : `${t} ${new Date().getFullYear()}`;
  const d = new Date(withYear);
  if (Number.isNaN(d.getTime())) return t;
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

const day = (iso: string | null) => (iso ? iso.slice(0, 10) : "");
const yesNo = (v: boolean) => (v ? "Yes" : "No");
const isYes = (text: string) => /^(y|yes|true|1|on|oui)$/i.test(text.trim());
const TASK_TYPES: Record<TaskTypeEdit, string> = { FixedUnits: "Fixed Units", FixedWork: "Fixed Work", FixedDuration: "Fixed Duration" };
function typedTaskType(text: string): TaskTypeEdit {
  const t = text.toLowerCase();
  if (t.includes("work")) return "FixedWork";
  if (t.includes("dur")) return "FixedDuration";
  return "FixedUnits";
}
function parsePercent(text: string): number {
  // "50", "50%" or " 50 % "; anything else becomes 255, which the app refuses with a clear message.
  const n = Math.round(Number(text.replace("%", "").trim()));
  return text.trim() !== "" && Number.isFinite(n) && n >= 0 && n <= 100 ? n : 255;
}
const money = (cents: number | null, currency: string) => (cents ? formatMoney(cents, currency) : "");

export const ALL_COLUMNS: Column[] = [
  { id: "row", label: "#", width: 44, align: "right", group: "General", fixed: true, text: () => "" },
  { id: "indicators", label: "", width: 40, group: "General", fixed: true, text: () => "" },
  { id: "wbs", label: "WBS", width: 64, group: "General", help: "wbs", text: (r) => r.wbs, title: () => "Where the task sits in the work breakdown structure" },
  {
    id: "name",
    label: "Task Name",
    width: 220,
    group: "General",
    tour: "name",
    fixed: true,
    text: (r) => r.name,
    edit: { initial: (r) => r.name, make: (r, text) => ({ kind: "RenameTask", id: r.id, name: text }) },
    kind: "schedule",
  },
  {
    id: "duration",
    label: "Duration",
    width: 84,
    align: "right",
    group: "Schedule",
    help: "duration",
    text: (r, perDay) => (r.isMilestone ? "0 days" : formatDays(r.durationMinutes, perDay)),
    edit: { initial: (r) => r.durationText, make: (r, text) => ({ kind: "SetDuration", id: r.id, text }) },
    kind: "schedule",
  },
  {
    id: "start",
    label: "Start",
    width: 112,
    group: "Schedule",
    text: (r) => formatDate(r.start),
    title: (r) => formatDateTime(r.start),
    // Typing a start moves the task there (Start No Earlier Than that day), as dragging its bar does.
    edit: { initial: (r) => day(r.start), make: (r, text) => ({ kind: "MoveTask", id: r.id, startDate: typedDate(text) }) },
    kind: "schedule",
  },
  {
    id: "finish",
    label: "Finish",
    width: 112,
    group: "Schedule",
    text: (r) => formatDate(r.finish),
    title: (r) => formatDateTime(r.finish),
    edit: { initial: (r) => day(r.finish), make: (r, text) => ({ kind: "SetFinish", id: r.id, finishDate: typedDate(text) }) },
    kind: "schedule",
  },
  {
    id: "percent",
    label: "% Done",
    width: 80,
    align: "right",
    group: "Progress",
    help: "progress",
    text: (r) => `${r.percentComplete}%`,
    title: (r) => (r.actualFinish ? `Finished ${formatDateTime(r.actualFinish)}` : r.actualStart ? `Started ${formatDateTime(r.actualStart)}` : "Not started"),
    edit: { initial: (r) => String(r.percentComplete), make: (r, text) => ({ kind: "SetPercentComplete", ids: [r.id], percent: parsePercent(text) }) },
    kind: "progress",
  },
  {
    id: "predecessors",
    label: "Predecessors",
    width: 116,
    group: "Schedule",
    help: "predecessors",
    tour: "predecessors",
    text: (r) => r.predecessors,
    edit: { initial: (r) => r.predecessors, make: (r, text) => ({ kind: "SetPredecessors", id: r.id, text }) },
    kind: "schedule",
  },
  {
    id: "resources",
    label: "Resources",
    width: 140,
    group: "General",
    help: "resources",
    text: (r) => r.resourceNames,
    edit: { initial: (r) => r.resourceNames, make: (r, text) => ({ kind: "SetTaskResources", id: r.id, text }) },
    kind: "resources",
  },
  {
    id: "work",
    label: "Work",
    width: 80,
    align: "right",
    group: "Schedule",
    text: (r) => r.workText,
    title: () => "Hours of work for the task's people, e.g. 40h",
    edit: { initial: (r) => r.workText, make: (r, text) => ({ kind: "SetWork", id: r.id, text }) },
    kind: "resources",
  },
  { id: "cost", label: "Cost", width: 96, align: "right", group: "Cost", help: "cost", text: (r, _p, c) => money(r.costCents, c) },
  {
    id: "fixedCost",
    label: "Fixed Cost",
    width: 96,
    align: "right",
    group: "Cost",
    text: (r, _p, c) => money(r.fixedCostCents, c),
    edit: { initial: (r) => (r.fixedCostCents ? (r.fixedCostCents / 100).toFixed(2) : ""), make: (r, text) => update(r, { fixedCost: text }) },
    kind: "schedule",
  },
  { id: "slack", label: "Slack", width: 76, align: "right", group: "Schedule", help: "slack", text: (r, perDay) => formatDays(r.totalSlackMinutes, perDay) },
  { id: "lateStart", label: "Late Start", width: 112, group: "Schedule", text: (r) => formatDate(r.lateStart), title: () => "The latest it can start without moving the project finish" },
  { id: "lateFinish", label: "Late Finish", width: 112, group: "Schedule", text: (r) => formatDate(r.lateFinish) },
  {
    id: "deadline",
    label: "Deadline",
    width: 112,
    group: "Schedule",
    text: (r) => formatDate(r.deadline),
    title: () => "A date it should finish by; flagged when it finishes later, without moving it",
    edit: { initial: (r) => day(r.deadline), make: (r, text) => update(r, { deadline: typedDate(text) }) },
    kind: "schedule",
  },
  {
    id: "constraint",
    label: "Constraint",
    width: 150,
    group: "Advanced",
    text: (r) => (r.constraint.kind === "ASAP" ? "" : `${CONSTRAINTS[r.constraint.kind].label}${r.constraint.date ? ` ${formatDate(r.constraint.date)}` : ""}`),
    title: () => "Change it in Task Info (⌘I)",
  },
  {
    id: "mode",
    label: "Mode",
    width: 84,
    group: "Advanced",
    text: (r) => (r.isSummary ? "" : r.manual ? "Manual" : "Auto"),
    title: () => "Auto: scheduled from its links. Manual: keeps the dates you give it. Type Auto or Manual.",
    edit: { initial: (r) => (r.manual ? "Manual" : "Auto"), make: (r, text) => update(r, { manual: /^m/i.test(text.trim()) }) },
    kind: "schedule",
  },
  {
    id: "taskType",
    label: "Task Type",
    width: 116,
    group: "Advanced",
    text: (r) => (r.isSummary ? "" : TASK_TYPES[r.taskType]),
    title: () => "Fixed Units, Fixed Work or Fixed Duration: what stays the same when you change the others",
    edit: { initial: (r) => TASK_TYPES[r.taskType], make: (r, text) => update(r, { taskType: typedTaskType(text) }) },
    kind: "schedule",
  },
  {
    id: "effortDriven",
    label: "Effort Driven",
    width: 96,
    group: "Advanced",
    text: (r) => (r.isSummary ? "" : yesNo(r.effortDriven)),
    edit: { initial: (r) => yesNo(r.effortDriven), make: (r, text) => update(r, { effortDriven: isYes(text) }) },
    kind: "schedule",
  },
  { id: "critical", label: "Critical", width: 72, group: "Schedule", text: (r) => yesNo(r.critical) },
  { id: "outlineLevel", label: "Outline Level", width: 92, align: "right", group: "General", text: (r) => String(r.depth + 1) },
  {
    id: "actualStart",
    label: "Actual Start",
    width: 112,
    group: "Progress",
    text: (r) => formatDate(r.actualStart),
    edit: { initial: (r) => day(r.actualStart), make: (r, text) => update(r, { actualStart: typedDate(text) }) },
    kind: "progress",
  },
  {
    id: "actualFinish",
    label: "Actual Finish",
    width: 112,
    group: "Progress",
    text: (r) => formatDate(r.actualFinish),
    edit: { initial: (r) => day(r.actualFinish), make: (r, text) => update(r, { actualFinish: typedDate(text) }) },
    kind: "progress",
  },
  { id: "baselineStart", label: "Baseline Start", width: 112, group: "Baseline", help: "baseline", text: (r) => formatDate(r.baselineStart) },
  { id: "baselineFinish", label: "Baseline Finish", width: 112, group: "Baseline", help: "baseline", text: (r) => formatDate(r.baselineFinish) },
  { id: "baselineCost", label: "Baseline Cost", width: 104, align: "right", group: "Baseline", text: (r, _p, c) => money(r.baselineCostCents, c) },
  {
    id: "finishVar",
    label: "Finish Var.",
    width: 92,
    align: "right",
    group: "Baseline",
    help: "baseline",
    text: (r, perDay) => {
      const v = r.finishVarianceMinutes;
      if (v === null) return "";
      if (v === 0) return "on plan";
      return `${v > 0 ? "+" : "−"}${formatDays(Math.abs(v), perDay)}`;
    },
  },
  { id: "bcwp", label: "Earned Value", width: 104, align: "right", group: "Cost", help: "earned", text: (r, _p, c) => (r.earnedValue ? money(r.earnedValue.bcwp, c) : "") },
  { id: "cpi", label: "CPI", width: 60, align: "right", group: "Cost", help: "earned", text: (r) => r.earnedValue?.cpi?.toFixed(2) ?? "" },
  { id: "spi", label: "SPI", width: 60, align: "right", group: "Cost", help: "earned", text: (r) => r.earnedValue?.spi?.toFixed(2) ?? "" },
  {
    id: "levelingDelay",
    label: "Leveling Delay",
    width: 104,
    align: "right",
    group: "Advanced",
    text: (r, perDay) => (r.levelingDelayMinutes ? formatDays(r.levelingDelayMinutes, perDay) : ""),
  },
  {
    id: "notes",
    label: "Notes",
    width: 200,
    group: "General",
    text: (r) => r.notes.split("\n")[0],
    title: (r) => r.notes,
    edit: { initial: (r) => r.notes, make: (r, text) => update(r, { notes: text }) },
    kind: "schedule",
  },
];

const BY_ID = new Map(ALL_COLUMNS.map((c) => [c.id, c]));
export const columnById = (id: string) => BY_ID.get(id);

/** The columns a new list shows (Microsoft Project's Entry table, plus WBS, % done and cost). */
export function defaultColumns(hasBaseline: boolean): string[] {
  const ids = ["row", "indicators", "wbs", "name", "duration", "start", "finish", "percent", "predecessors", "resources", "cost", "slack"];
  return hasBaseline ? [...ids, "finishVar"] : ids;
}

export interface Layout {
  /** Shown columns, in order. */
  ids: string[];
  /** Widths the person dragged, by column. */
  widths: Record<string, number>;
}

const KEY = "superpm.taskColumns";

export function loadLayout(): Layout | null {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as Layout;
    const ids = Array.isArray(v.ids) ? v.ids.filter((id) => BY_ID.has(id)) : [];
    if (!ids.includes("name")) return null;
    return { ids: withFixed(ids), widths: typeof v.widths === "object" && v.widths ? v.widths : {} };
  } catch {
    return null;
  }
}

export function saveLayout(layout: Layout | null) {
  try {
    if (layout) localStorage.setItem(KEY, JSON.stringify(layout));
    else localStorage.removeItem(KEY);
  } catch {
    // a convenience only: the defaults come back next time
  }
}

/** The row number and indicators always lead. */
function withFixed(ids: string[]): string[] {
  return ["row", "indicators", ...ids.filter((id) => id !== "row" && id !== "indicators")];
}

/** Shows or hides a column; a newly shown one goes after the Task Name (or at the end). */
export function toggled(ids: string[], id: string): string[] {
  if (ids.includes(id)) return ids.filter((x) => x !== id || columnById(x)?.fixed);
  return [...ids, id];
}

/** Moves column `id` to just before `beforeId` (or to the end when null). */
export function moved(ids: string[], id: string, beforeId: string | null): string[] {
  const rest = ids.filter((x) => x !== id);
  const at = beforeId === null ? rest.length : rest.indexOf(beforeId);
  rest.splice(at < 0 ? rest.length : at, 0, id);
  return withFixed(rest);
}
