// What the person using the app may change in a team project, mirroring the team server's
// rules (crates/pm-server/src/permissions.rs) so the app can grey things out instead of letting
// an edit be refused. The server still checks every change.
import type { Snapshot } from "./api";
import type { Row } from "./bindings/Row";

export type Kind = "schedule" | "progress" | "resources" | "costs" | "project";

const KIND_TEXT: Record<Kind, string> = {
  schedule: "change the schedule (tasks, durations, links)",
  progress: "update progress",
  resources: "change resources or assignments",
  costs: "change costs",
  project: "change project settings",
};

/** "alex" from "alex@example.com". */
const localPart = (email: string) => email.split("@")[0] ?? "";

/** Null if the person may make this kind of change to `row` (or to the project when absent);
 * otherwise why not, in plain words. Projects that are not shared are always open. */
export function blocked(s: Snapshot, kind: Kind, row?: Row | null): string | null {
  const access = s.project.team?.access;
  if (!access || access.role === "owner") return null;
  if (access.role === "viewer") return "You can view this project but not change it.";
  if (!access.limits[kind]) return `The project's owner has not let you ${KIND_TEXT[kind]}.`;
  const scope = access.limits.tasks;
  if (scope.kind === "all") return null;
  if (!row) {
    return kind === "project" || kind === "schedule" ? "The project's owner lets you change only some tasks, so this is not open to you." : null;
  }
  if (scope.kind === "assigned") {
    const me = [access.name, access.email, localPart(access.email)].map((x) => x.trim().toLowerCase()).filter(Boolean);
    const names = row.resourceNames.split(",").map((n) => n.replace(/\[.*\]$/, "").trim().toLowerCase());
    return names.some((n) => me.includes(n)) ? null : `You can change only tasks you are assigned to, and “${row.name}” is not one of them.`;
  }
  // Phases: the row or one of its parents is one of them.
  const byId = new Map(s.rows.map((r) => [r.id, r]));
  for (let at: Row | undefined = row; at; at = at.parentId ? byId.get(at.parentId) : undefined) {
    if (scope.ids.includes(at.id)) return null;
  }
  return `You can change only tasks in some phases, and “${row.name}” is not in them.`;
}

/** For the header: "View only", "Limited editing", or nothing. */
export function accessLabel(s: Snapshot): { text: string; detail: string } | null {
  const access = s.project.team?.access;
  if (!access || access.role === "owner") return null;
  if (access.role === "viewer") return { text: "View only", detail: "You can look at this project but not change it. The owner decides this in Team > Members & Permissions." };
  const l = access.limits;
  const allowed = (Object.keys(KIND_TEXT) as Kind[]).filter((k) => l[k]);
  if (allowed.length === 5 && l.tasks.kind === "all") return null;
  const tasks = l.tasks.kind === "assigned" ? " on tasks you are assigned to" : l.tasks.kind === "phases" ? " in some phases" : "";
  return {
    text: "Limited editing",
    detail: `You may ${allowed.length ? allowed.map((k) => KIND_TEXT[k]).join(", ") : "change nothing"}${tasks}. The owner decides this in Team > Members & Permissions.`,
  };
}
