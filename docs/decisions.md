# Decisions

Short notes, newest last. Owner-approved product decisions (D1–D9) are in the spec; these are implementation decisions.

## 2026-10-08

- **Tauri 2.12.1, not 3.0.** Tauri 3 is in alpha. D1 says Tauri 2; stay on 2.x stable.
- **Pinned front-end versions:** React 19.3.0, Vite 8.3.1, @vitejs/plugin-react 6.1.1, TypeScript 5.9.3. Releases from the last two weeks were skipped (Vite 8.3.4 came out on 2026-10-08). TypeScript 7 exists, but 5.9 is the conservative choice for tooling compatibility.
- **Time in the core is `Moment`: whole minutes since 1970-01-01T00:00, naive local.** All calendar arithmetic is integer math. `jiff` civil types appear only at API edges. Date conversion uses Howard Hinnant's algorithms, checked against `jiff` by a property test.
- **Calendar arithmetic walks day by day.** Simple and correct first. If the 5,000-task benchmark shows it matters, add week skipping.
- **Ids are UUID v7 created outside `pm-core`,** so the core needs no clock or random source.
- **Ordered maps (`BTreeMap`) in the model,** for deterministic output (rule R29).
- **No network in the app:** the capability file grants only `core:event:default`, and the CSP allows only `'self'` and Tauri IPC.
- **Placeholder identity:** product name `Super PM`, bundle id `app.superpm.desktop`, placeholder icon in `apps/desktop/app-icon.svg`. See `open-questions.md`.

## 2026-10-08, step 4 (commands and storage)

- **Undo survives reopening, capped at 500 edits** (owner decision, open question 1). The `undo_history` table holds the stack; `change_log` stays append-only and records undo and redo as entries of their own, which keeps it usable for sync later.
- **Commands live in `pm-core`.** `apply` returns the exact inverse or refuses the command with nothing changed. `Batch` makes several commands one undo step and rolls back fully if any part fails.
- **Commands refuse what would break scheduling:** links that would create a loop, duplicate links between the same two tasks, links on summary tasks, moving a task inside itself, and putting a task under a linked task. A property test applies random command sequences and checks the project always stays schedulable. The engine still reports loops, for imported files.
- **Grouping rapid edits:** the UI sends one command when an edit is finished (Return in a cell, mouse-up after a drag), or a `Batch` for multi-row actions. The store does not merge entries by time.
- **File format:** SQLite with `application_id` "PMPJ" and `user_version` for the schema version; forward-only migrations; a newer file is refused with a clear message. Times are integer minutes (`Moment`); the calendar is one JSON value, validated on load.
- **Durability:** WAL journal with `synchronous = FULL`; each command and its log entry are one transaction. If the write fails, the in-memory change is reverted. A test kills a writer process mid-write three times and checks integrity and that data matches the log.
- **Validation on load:** calendar types validate when deserialized, so a damaged file cannot produce a calendar with no working time, which would make date arithmetic loop forever.
- **Reserved resource, assignment and baseline tables were not created.** A table in a shipped schema is a migration commitment, so the cost is not zero; they get added in iteration 2.
- **Issue for step 8:** while a project is open, WAL keeps two side files next to it (`.pmproj-wal`, `.pmproj-shm`). They disappear on a clean close, and Save a Copy uses SQLite's backup API to write a single standalone file. If the side files look wrong in Finder, the option is to store projects as a macOS package (a folder that looks like one file).
