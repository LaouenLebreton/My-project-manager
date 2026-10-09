// Development only: a stand-in for the Rust backend so the UI can be exercised in a plain
// browser (http://localhost:1420) and by UI tests. It records the edits it receives and
// imitates just enough behaviour to drive the grid; dates here are fake and never computed.
// Loaded by main.tsx only in dev mode when not running inside Tauri.

import type { Edit } from "../bindings/Edit";
import type { Link } from "../bindings/Link";
import type { ResourceRow } from "../bindings/ResourceRow";
import type { Row } from "../bindings/Row";
import type { Snapshot } from "../bindings/Snapshot";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
    __mock?: { edits: Edit[]; textEditing: boolean[]; rows: Row[]; links: Link[]; resources: ResourceRow[] };
    /** Test hook: send an event as the Rust side would, e.g. a menu item id. */
    __mockEmit?: (event: string, payload: unknown) => void;
  }
}

let next = 0;
function row(name: string, depth = 0): Row {
  return {
    id: `t${++next}`,
    parentId: null,
    depth,
    wbs: "",
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
    constraint: { kind: "ASAP", date: null },
    notes: "",
    resourceNames: "",
    overloaded: false,
    levelingDelayMinutes: 0,
    manual: false,
    leveledFrom: null,
    costCents: 36_400,
    fixedCostCents: 0,
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
  // Parents and WBS codes from the generated outline.
  const stack: Row[] = [];
  const counters: number[] = [];
  for (const r of rows) {
    stack.length = r.depth;
    r.parentId = stack[r.depth - 1]?.id ?? null;
    stack[r.depth] = r;
    counters.length = r.depth + 1;
    counters[r.depth] = (counters[r.depth] ?? 0) + 1;
    r.wbs = counters.join(".");
  }
  return { rows, links };
}

/** `?report` in the URL shows a sample import report, to check the dialog. */
function sampleReport() {
  return {
    source: "Website Relaunch",
    imported: [
      { what: "Tasks, with names, outline, durations, milestones, notes and % complete", count: 7, examples: [] },
      { what: "Links, with type and lag", count: 5, examples: [] },
    ],
    approximated: [
      { what: "Elapsed durations (counted in calendar days) were converted to working time", count: 1, examples: ["Visual design"] },
      { what: "“As late as possible” is not supported yet; these tasks are scheduled as soon as possible", count: 1, examples: ["Visual design"] },
    ],
    ignored: [
      { what: "Resources (arrive in iteration 2)", count: 2, examples: [] },
      { what: "Baselines (arrive in iteration 2)", count: 1, examples: ["Wireframes & sitemap"] },
    ],
    recalculated: ["Critical", "Finish", "Start", "TotalSlack"],
  };
}

export function install() {
  const n = Number(new URLSearchParams(location.search).get("n") ?? 0);
  const generated = n > 0 ? generate(n) : { rows: [row("Alpha"), row("Beta"), row("Gamma")], links: [] };
  const person = (name: string, overloaded: boolean): ResourceRow => ({
    id: `r-${name}`,
    name,
    initials: name[0],
    resourceType: "Work",
    maxUnitsText: "100%",
    materialLabel: "",
    totalText: "40h",
    peakPercent: overloaded ? 200 : 100,
    taskCount: 2,
    rateCents: 4_550,
    costCents: 182_000,
    overloads: overloaded ? [{ start: "2026-10-06T08:00:00", finish: "2026-10-07T17:00:00", peakPercent: 200, tasks: ["Task 1", "Task 2"] }] : [],
  });
  const state = { edits: [] as Edit[], textEditing: [] as boolean[], ...generated, resources: [person("Alex", true), person("Sam", false)] };
  if (state.rows[1]) {
    state.rows[1].resourceNames = "Alex";
    state.rows[1].overloaded = true;
  }
  window.__mock = state;
  const snapshot = (): Snapshot => ({
    project: {
      name: "Mock Project",
      path: "/mock.pmproj",
      start: "2026-10-05T08:00:00",
      finish: state.rows.reduce((f, r) => (r.finish && r.finish > f ? r.finish : f), "2026-10-09T17:00:00"),
      minutesPerDay: 480,
      workingWeekdays: [true, true, true, true, true, false, false],
      calendarExceptions: [{ date: "2026-10-14", working: false, ranges: [] }],
      workingWeek: [0, 1, 2, 3, 4].map(() => [{ start: 480, end: 720 }, { start: 780, end: 1020 }]).concat([[], []]),
      currency: "€",
      costCents: 1_234_550,
    },
    rows: state.rows.map((r) => ({ ...r })),
    links: state.links.map((l) => ({ ...l })),
    problem: null,
    conflicts: [],
    undoLabel: state.edits.length ? "Edit" : null,
    redoLabel: null,
    resources: state.resources.map((r) => ({ ...r })),
  });
  const at = (id: string) => state.rows.findIndex((r) => r.id === id);
  const fail = (message: string) => {
    throw { message };
  };

  // Events: enough of Tauri's event plumbing for `listen()` to work.
  const callbacks = new Map<number, (e: unknown) => void>();
  const listeners = new Map<number, { event: string; handler: number }>();
  let nextListener = 1;
  let nextCallback = 1;
  const emit = (event: string, payload: unknown) => {
    for (const l of listeners.values()) if (l.event === event) callbacks.get(l.handler)?.({ event, id: 0, payload });
  };
  window.__mockEmit = emit;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event: string, id: number) => void listeners.delete(id) };
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb: (e: unknown) => void) => {
      const id = nextCallback++;
      callbacks.set(id, cb);
      return id;
    },
    invoke: async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "plugin:event|listen") {
        const id = nextListener++;
        listeners.set(id, { event: args.event as string, handler: args.handler as number });
        return id;
      }
      if (cmd.startsWith("plugin:event")) return 1;
      switch (cmd) {
        case "get_snapshot":
          return null;
        case "take_import_report":
          return null;
        case "recent_files":
          return [{ name: "Office Move", path: "/Users/me/Documents/Office Move.pmproj" }];
        case "new_project":
        case "open_project":
        case "import_project":
        case "export_project":
          return null; // as if the user cancelled the dialog
        case "open_recent":
        case "open_sample":
          if (new URLSearchParams(location.search).has("report")) {
            setTimeout(() => emit("import-report", sampleReport()), 50);
          }
          return snapshot();
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
              const after = e.afterId ? state.rows[at(e.afterId)] : null;
              const r = { ...row(e.name, after?.depth ?? 0), parentId: after?.parentId ?? null, wbs: "new" };
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
            case "UpdateTask": {
              const r = state.rows[at(e.id)];
              if (e.name !== null) r.name = e.name;
              if (e.notes !== null) r.notes = e.notes;
              if (e.percentComplete !== null) r.percentComplete = e.percentComplete;
              if (e.constraint) r.constraint = e.constraint;
              if (e.milestone !== null) r.isMilestone = e.milestone;
              break;
            }
            case "AddResource":
              if (state.resources.some((r) => r.name.toLowerCase() === e.name.toLowerCase())) fail(`there is already a resource called “${e.name}”`);
              state.resources.push(person(e.name, false));
              break;
            case "UpdateResource": {
              const r = state.resources.find((x) => x.id === e.id)!;
              if (e.name !== null) r.name = e.name;
              if (e.initials !== null) r.initials = e.initials;
              if (e.resourceType !== null) r.resourceType = e.resourceType;
              if (e.maxUnits !== null) r.maxUnitsText = /%$/.test(e.maxUnits) ? e.maxUnits : `${Number(e.maxUnits) * 100}%`;
              break;
            }
            case "DeleteResources":
              state.resources = state.resources.filter((r) => !e.ids.includes(r.id));
              break;
            case "SetTaskResources":
              state.rows[at(e.id)].resourceNames = e.text;
              break;
            case "AddSubtask": {
              const parent = state.rows[at(e.parentId)];
              const r = { ...row(e.name, parent.depth + 1), parentId: parent.id, wbs: `${parent.wbs}.${state.rows.filter((x) => x.parentId === parent.id).length + 1}` };
              let i = at(e.parentId) + 1;
              while (i < state.rows.length && state.rows[i].depth > parent.depth) i++;
              state.rows.splice(i, 0, r);
              parent.isSummary = true;
              createdId = r.id;
              break;
            }
            case "MoveUnder":
              state.rows[at(e.id)].parentId = e.parentId;
              break;
            case "SetCalendar":
              if (e.week.every((d) => d.length === 0)) fail("the working week has no working time, so no task could ever be scheduled");
              break;
            case "AddLink":
              state.links.push({ id: `l${Math.random()}`, predecessorId: e.predecessorId, successorId: e.successorId, kind: "FS", lagMinutes: 0 });
              break;
            case "DeleteTasks":
              state.rows = state.rows.filter((r) => !e.ids.includes(r.id));
              break;
          }
          const message = e.kind === "LevelResources" ? "1 task now starts later so nobody is over-allocated." : null;
          return { snapshot: snapshot(), createdId, message };
        }
      }
      fail(`mock backend has no command ${cmd}`);
    },
  };
}
