// Typed wrappers for the Tauri commands in src-tauri/src/ipc.rs.
// The types in ./bindings are generated from Rust: run `cargo test -p super-pm` after changing them.
import { invoke } from "@tauri-apps/api/core";
import type { Edit } from "./bindings/Edit";
import type { EditResult } from "./bindings/EditResult";
import type { IpcError } from "./bindings/IpcError";
import type { RecentFile } from "./bindings/RecentFile";
import type { ReportView } from "./bindings/ReportView";
import type { Snapshot } from "./bindings/Snapshot";
import type { SharedProject } from "./bindings/SharedProject";
import type { SyncStatus } from "./bindings/SyncStatus";
import type { TeamStatus } from "./bindings/TeamStatus";
import type { TeamUser } from "./bindings/TeamUser";
import type { TeamView } from "./bindings/TeamView";

export type { Edit, EditResult, IpcError, RecentFile, SharedProject, Snapshot, SyncStatus, TeamStatus, TeamView };

/** Rejects with an IpcError whose message can be shown to the user as is. */
export const api = {
  getSnapshot: () => invoke<Snapshot | null>("get_snapshot"),
  /** Opening a project resolves with null if it opened in another window (or was cancelled). */
  openSample: () => invoke<Snapshot | null>("open_sample"),
  applyEdit: (edit: Edit) => invoke<EditResult>("apply_edit", { edit }),
  undo: () => invoke<Snapshot>("undo"),
  redo: () => invoke<Snapshot>("redo"),
  /** Resolve with null when the user cancels the dialog. */
  newProject: () => invoke<Snapshot | null>("new_project"),
  openProject: () => invoke<Snapshot | null>("open_project"),
  importProject: () => invoke<Snapshot | null>("import_project"),
  exportProject: () => invoke<void>("export_project"),
  openRecent: (path: string) => invoke<Snapshot | null>("open_recent", { path }),
  takeImportReport: () => invoke<ReportView | null>("take_import_report"),
  recentFiles: () => invoke<RecentFile[]>("recent_files"),
  setTextEditing: (editing: boolean) => invoke<void>("set_text_editing", { editing }),
};

/** The team server (iteration 3). */
export const team = {
  status: () => invoke<TeamStatus>("team_status"),
  /** With `setupName`, sets up a new server and becomes its administrator. */
  signIn: (server: string, email: string, password: string, setupName: string | null) =>
    invoke<TeamStatus>("team_sign_in", { server, email, password, setupName }),
  signOut: () => invoke<TeamStatus>("team_sign_out"),
  /** Administrators only. */
  createAccount: (name: string, email: string, password: string) => invoke<TeamUser>("team_create_account", { name, email, password }),
  teams: () => invoke<TeamView[]>("team_teams"),
  createTeam: (name: string) => invoke<TeamView>("team_create_team", { name }),
  addMember: (teamId: string, email: string) => invoke<void>("team_add_member", { teamId, email }),
  projects: () => invoke<SharedProject[]>("team_projects"),
  share: (teamId: string) => invoke<Snapshot>("team_share", { teamId }),
  open: (projectId: string) => invoke<Snapshot | null>("team_open", { projectId }),
};

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as IpcError).message);
  return String(e);
}
