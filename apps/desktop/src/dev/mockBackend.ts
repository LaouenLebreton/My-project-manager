// Development only: a stand-in for the Rust backend so the UI can be exercised in a plain
// browser (http://localhost:1420) and by UI tests. It records the edits it receives and
// imitates just enough behaviour to drive the grid; dates here are fake and never computed.
// Loaded by main.tsx only in dev mode when not running inside Tauri.

import type { Edit } from "../bindings/Edit";
import type { Link } from "../bindings/Link";
import type { Row } from "../bindings/Row";
import type { Snapshot } from "../bindings/Snapshot";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
    __mock?: { edits: Edit[]; textEditing: boolean[]; rows: Row[]; links: Link[] };
  }
}

let next = 0;
function row(name: string, depth = 0): Row {
  return {
    id: `t${++next}`,
    parentId: null,
    depth,
    isSummary: false,
    isMilestone: false,
    name,
    durationMinutes: 480,
    start: "2026-10-05T08:00:00",
    finish: "2026-10-05T17:00:00",
    lateStart: "2026-10-05T08:00:00",
    lateFinish: "2026-10-05T17:00:00",
    totalSlackMinutes: 0,
    critical: true,
    percentComplete: 0,
    durationText: "1d",
    predecessors: "",
  };
}

/** Fake dates: `day` days after Monday 2026-10-05, at 08:00 or 17:00. */
function iso(day: number, finish: boolean): string {
  const d = new Date(Date.UTC(2026, 9, 5) + Math.floor(day) * 86_400_000);
  return `${d.toISOString().slice(0, 10)}T${finish ? "17" : "08"}:00:00`;
}

/** `?n=5000` in the URL generates that many tasks in phases of ten, chained by links. */
function generate(n: number): { rows: Row[]; links: Link[] } {
  const rows: Row[] = [];
  const links: Link[] = [];
  let phaseStart = 0;
  let day = 0;
  for (let k = 0; k < n; k++) {
    if (k % 11 === 0) {
      phaseStart = rows.length;
      rows.push({ ...row(`Phase ${k / 11 + 1}`), isSummary: true });
      day = (k / 11) * 3;
      continue;
    }
    const days = 1 + (k % 4);
    const r = row(`Task ${k}`, 1);
    r.isMilestone = k % 17 === 0;
    r.start = iso(day, false);
    r.finish = iso(day + (r.isMilestone ? 0 : days - 1), !r.isMilestone);
    r.critical = k % 3 === 0;
    r.totalSlackMinutes = r.critical ? 0 : 960;
    if (rows.length - 1 > phaseStart) {
      links.push({ id: `l${k}`, predecessorId: rows[rows.length - 1].id, successorId: r.id, kind: "FS", lagMinutes: 0 });
    }
    rows.push(r);
    day += r.isMilestone ? 0 : days;
    const phase = rows[phaseStart];
    phase.start ??= r.start;
    phase.finish = r.finish;
  }
  return { rows, links };
}

export function install() {
  const n = Number(new URLSearchParams(location.search).get("n") ?? 0);
  const generated = n > 0 ? generate(n) : { rows: [row("Alpha"), row("Beta"), row("Gamma")], links: [] };
  const state = { edits: [] as Edit[], textEditing: [] as boolean[], ...generated };
  window.__mock = state;
  const snapshot = (): Snapshot => ({
    project: {
      name: "Mock Project",
      path: "/mock.pmproj",
      start: "2026-10-05T08:00:00",
      finish: state.rows.reduce((f, r) => (r.finish && r.finish > f ? r.finish : f), "2026-10-09T17:00:00"),
      minutesPerDay: 480,
      workingWeekdays: [true, true, true, true, true, false, false],
      calendarExceptions: [{ date: "2026-10-14", working: false }],
    },
    rows: state.rows.map((r) => ({ ...r })),
    links: state.links.map((l) => ({ ...l })),
    problem: null,
    conflicts: [],
    undoLabel: state.edits.length ? "Edit" : null,
    redoLabel: null,
  });
  const at = (id: string) => state.rows.findIndex((r) => r.id === id);
  const fail = (message: string) => {
    throw { message };
  };

  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => Math.random(),
    invoke: async (cmd: string, args: Record<string, unknown>) => {
      if (cmd.startsWith("plugin:event")) return 1;
      switch (cmd) {
        case "get_snapshot":
          return null;
        case "open_sample":
        case "undo":
        case "redo":
          return snapshot();
        case "set_text_editing":
          state.textEditing.push(args.editing as boolean);
          return null;
        case "apply_edit": {
          const e = args.edit as Edit;
          state.edits.push(e);
          let createdId: string | null = null;
          switch (e.kind) {
            case "AddTask": {
              const r = row(e.name);
              state.rows.splice(e.afterId ? at(e.afterId) + 1 : state.rows.length, 0, r);
              createdId = r.id;
              break;
            }
            case "RenameTask":
              state.rows[at(e.id)].name = e.name;
              break;
            case "SetDuration":
              if (!/^\d/.test(e.text)) fail("type a duration like 3d, 4h or 2w");
              state.rows[at(e.id)].durationText = e.text;
              break;
            case "SetPredecessors":
              if (/\b9\b/.test(e.text)) fail("there is no task 9");
              state.rows[at(e.id)].predecessors = e.text;
              break;
            case "Indent":
            case "Outdent":
              for (const id of e.ids) state.rows[at(id)].depth = Math.max(0, state.rows[at(id)].depth + (e.kind === "Indent" ? 1 : -1));
              break;
            case "MoveTask": {
              const r = state.rows[at(e.id)];
              const length = Date.parse(r.finish!) - Date.parse(r.start!);
              r.start = `${e.startDate}T08:00:00`;
              r.finish = new Date(Date.parse(`${e.startDate}T08:00:00Z`) + length).toISOString().slice(0, 19);
              break;
            }
            case "SetFinish":
              if (e.finishDate < state.rows[at(e.id)].start!.slice(0, 10)) fail("the finish cannot be before the start");
              state.rows[at(e.id)].finish = `${e.finishDate}T17:00:00`;
              break;
            case "AddLink":
              state.links.push({ id: `l${Math.random()}`, predecessorId: e.predecessorId, successorId: e.successorId, kind: "FS", lagMinutes: 0 });
              break;
            case "DeleteTasks":
              state.rows = state.rows.filter((r) => !e.ids.includes(r.id));
              break;
          }
          return { snapshot: snapshot(), createdId };
        }
      }
      fail(`mock backend has no command ${cmd}`);
    },
  };
}
