// Typed wrappers for the Tauri commands in src-tauri/src/ipc.rs.
// The types in ./bindings are generated from Rust: run `cargo test -p super-pm` after changing them.
import { invoke } from "@tauri-apps/api/core";
import type { Edit } from "./bindings/Edit";
import type { EditResult } from "./bindings/EditResult";
import type { IpcError } from "./bindings/IpcError";
import type { Snapshot } from "./bindings/Snapshot";

export type { Edit, EditResult, IpcError, Snapshot };

/** Rejects with an IpcError whose message can be shown to the user as is. */
export const api = {
  getSnapshot: () => invoke<Snapshot | null>("get_snapshot"),
  openSample: () => invoke<Snapshot>("open_sample"),
  applyEdit: (edit: Edit) => invoke<EditResult>("apply_edit", { edit }),
  undo: () => invoke<Snapshot>("undo"),
  redo: () => invoke<Snapshot>("redo"),
  setTextEditing: (editing: boolean) => invoke<void>("set_text_editing", { editing }),
};

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as IpcError).message);
  return String(e);
}
