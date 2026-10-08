// Development only: a stand-in for the Rust backend so the UI can be exercised in a plain
// browser (http://localhost:1420) and by UI tests. It records the edits it receives and
// imitates just enough behaviour to drive the grid; dates here are fake and never computed.
// Loaded by main.tsx only in dev mode when not running inside Tauri.

import type { Edit } from "../bindings/Edit";
import type { Row } from "../bindings/Row";
import type { Snapshot } from "../bindings/Snapshot";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
    __mock?: { edits: Edit[]; textEditing: boolean[]; rows: Row[] };
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

export function install() {
  const state = { edits: [] as Edit[], textEditing: [] as boolean[], rows: [row("Alpha"), row("Beta"), row("Gamma")] };
  window.__mock = state;
  const snapshot = (): Snapshot => ({
    project: { name: "Mock Project", path: "/mock.pmproj", start: "2026-10-05T08:00:00", finish: "2026-10-09T17:00:00", minutesPerDay: 480 },
    rows: state.rows.map((r) => ({ ...r })),
    links: [],
    problem: null,
    conflicts: [],
    undoLabel: state.edits.length ? "Edit" : null,
    redoLabel: null,
  });
  const at = (id: string) => state.rows.findIndex((r) => r.id === id);
  const fail = (message: string) => {
    throw { message };
  };

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
