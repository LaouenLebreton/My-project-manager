// Typed wrappers for the Tauri commands in src-tauri/src/ipc.rs.
// The types in ./bindings are generated from Rust: run `cargo test -p super-pm` after changing them.
import { invoke } from "@tauri-apps/api/core";
import type { UndoStep } from "./bindings/UndoStep";
import type { ProjectRules } from "./bindings/ProjectRules";
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
import type { Invite } from "./bindings/Invite";
import type { TeamProject } from "./bindings/TeamProject";
import type { HistoryPage } from "./bindings/HistoryPage";
import type { MoveResult } from "./bindings/MoveResult";
import type { PortfolioView } from "./bindings/PortfolioView";
import type { Member } from "./bindings/Member";
import type { MemberLimits } from "./bindings/MemberLimits";
import type { ChatChannel } from "./bindings/ChatChannel";
import type { ChatPage } from "./bindings/ChatPage";
import type { ChatPost } from "./bindings/ChatPost";
import type { ChatTeam } from "./bindings/ChatTeam";

export type { Edit, EditResult, HistoryPage, Invite, Member, MemberLimits, MoveResult, IpcError, RecentFile, SharedProject, Snapshot, SyncStatus, TeamProject, TeamStatus, TeamView };

/** Rejects with an IpcError whose message can be shown to the user as is. */
export const api = {
  getSnapshot: () => invoke<Snapshot | null>("get_snapshot"),
  /** Opening a project resolves with null if it opened in another window (or was cancelled). */
  openSample: () => invoke<Snapshot | null>("open_sample"),
  applyEdit: (edit: Edit) => invoke<EditResult>("apply_edit", { edit }),
  /** File > Print: the system print panel (the page prints only the status report). */
  printWindow: () => invoke<void>("print_window"),
  /** Edit > Undo History: the steps, newest first (undone ones first). */
  undoHistory: () => invoke<UndoStep[]>("undo_history"),
  /** Undo `undo` steps, or redo `redo` steps. */
  stepHistory: (undo: number, redo: number) => invoke<Snapshot>("step_history", { undo, redo }),
  /** Edit > Copy: the app keeps the tasks; `text` is what went on the system clipboard. */
  copyTasks: (ids: string[], text: string) => invoke<void>("copy_tasks", { ids, text }),
  /** Edit > Paste: the copied tasks if `text` is still theirs, otherwise one task per line. */
  pasteTasks: (afterId: string | null, text: string) => invoke<EditResult>("paste_tasks", { afterId, text }),
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
  /** `mode` "setup": set up a new server (you become its administrator); "signup": create your
   * account on a server that takes sign-ups; null: sign in. */
  signIn: (server: string, email: string, password: string, name: string | null, mode: "setup" | "signup" | null) =>
    invoke<TeamStatus>("team_sign_in", { server, email, password, name, mode }),
  signOut: () => invoke<TeamStatus>("team_sign_out"),
  /** Administrators only. */
  createAccount: (name: string, email: string, password: string) => invoke<TeamUser>("team_create_account", { name, email, password }),
  teams: () => invoke<TeamView[]>("team_teams"),
  createTeam: (name: string) => invoke<TeamView>("team_create_team", { name }),
  addMember: (teamId: string, email: string) => invoke<void>("team_add_member", { teamId, email }),
  projects: () => invoke<SharedProject[]>("team_projects"),
  share: (teamId: string) => invoke<Snapshot>("team_share", { teamId }),
  /** Makes the open project a team project. The first time, pass your name and email: the app
   * then hosts teams itself. `teamId` null creates a team called `teamName`. */
  createProject: (myName: string | null, myEmail: string | null, teamId: string | null, teamName: string, emails: string[]) =>
    invoke<TeamProject>("team_create_project", { myName, myEmail, teamId, teamName, emails }),
  invite: (teamId: string, emails: string[]) => invoke<Invite[]>("team_invite", { teamId, emails }),
  /** With the invite text someone sent. */
  join: (invite: string) => invoke<TeamStatus>("team_join", { invite }),
  /** Moves every team you own to another server and carries on there. */
  move: (server: string, email: string, password: string, name: string | null, mode: "setup" | "signup" | null) =>
    invoke<MoveResult>("team_move", { server, email, password, name, mode }),
  /** The History view, newest first; `before` is the oldest version already shown. */
  history: (before: number | null) => invoke<HistoryPage>("team_history", { before }),
  revert: (version: number) => invoke<void>("team_revert", { version }),
  members: () => invoke<Member[]>("team_members"),
  rules: () => invoke<ProjectRules>("team_rules"),
  setRules: (rules: ProjectRules) => invoke<ProjectRules>("team_set_rules", { rules }),
  setMember: (userId: string, role: string, limits: MemberLimits) => invoke<void>("team_set_member", { userId, role, limits }),
  /** Takes someone out of the project's team (team owners only). */
  removeMember: (userId: string) => invoke<void>("team_remove_member", { userId }),
  open: (projectId: string) => invoke<Snapshot | null>("team_open", { projectId }),
};

/** The Portfolio window. */
export const portfolio = {
  data: () => invoke<PortfolioView>("portfolio_data"),
  /** Resolves true if a project was chosen. */
  add: () => invoke<boolean>("portfolio_add"),
  remove: (path: string) => invoke<void>("portfolio_remove", { path }),
  /** A blank group takes the project out of its group. */
  setGroup: (path: string, group: string) => invoke<void>("portfolio_set_group", { path, group }),
  open: (path: string) => invoke<void>("portfolio_open", { path }),
};

/** Chat (Teams model): teams, channels, posts and replies. */
export const chat = {
  me: () => invoke<string | null>("chat_me"),
  teams: () => invoke<ChatTeam[]>("chat_teams"),
  /** Conversations newest first; `before` is the oldest post id already shown. */
  posts: (channelId: string, before: string | null) => invoke<ChatPage>("chat_posts", { channelId, before }),
  send: (channelId: string, body: string, parentId: string | null) => invoke<ChatPost>("chat_send", { channelId, body, parentId }),
  read: (channelId: string) => invoke<void>("chat_read", { channelId }),
  createChannel: (teamId: string, name: string) => invoke<ChatChannel>("chat_create_channel", { teamId, name }),
  edit: (postId: string, body: string) => invoke<ChatPost>("chat_edit", { postId, body }),
  remove: (postId: string) => invoke<ChatPost>("chat_delete", { postId }),
  react: (postId: string, emoji: string) => invoke<ChatPost>("chat_react", { postId, emoji }),
};

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as IpcError).message);
  return String(e);
}
