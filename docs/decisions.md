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

## 2026-10-08, step 5 (IPC)

- **ts-rs 12 generates the TypeScript types** in `apps/desktop/src/bindings` (run `cargo test -p super-pm`). specta and tauri-specta would also generate the invoke wrappers, but after two years they are still release candidates; ts-rs is stable and far more widely used. The small cost is `apps/desktop/src/api.ts`, a hand-written typed wrapper around the commands.
- **The UI sends edits, not core commands.** `Edit` is user intent ("add a task after this one"); the app layer chooses UUID v7 ids and sort keys and builds the core `Command`. The UI never makes ids, positions or dates.
- **A full snapshot after every change**, as the spec allows for the first version. Snapshot size and IPC time at 5,000 tasks are measured in step 7, before deciding on deltas.
- **Undo and Redo in the Edit menu run in Rust** and push the new snapshot to the UI. The menu items show the action ("Undo Change Duration") and are greyed out when there is nothing to undo.
- **Issue for step 6:** ⌘Z is a menu shortcut, so while the user is typing in a cell it would undo the last project edit instead of the typing. A Mac text field should undo its own typing first. Plan: while a cell editor is open, the UI handles ⌘Z itself, and the menu action is skipped.
- **Sample project** "Office Move (Sample)" is created in `~/Library/Application Support/app.superpm.desktop/` on first use. Until step 8 it is the only way to open a project.

## 2026-10-08, step 6 (task grid)

- **Keyboard model:** arrows move; Return edits, and while editing saves and moves down; Escape cancels; typing a character starts editing; Delete removes the selected rows; Shift extends the selection. **Tab and Shift-Tab always indent and outdent** (spec 5.1.2), even while editing, as in Mac outliners. Moving between cells uses the arrow keys.
- **Fast entry:** the blank last row accepts a name; Return adds the task and moves to the new blank row, so 50 tasks can be typed without the mouse.
- **Linking from the keyboard:** a Predecessors column in Microsoft Project syntax (`2, 5SS, 7FF+2d`, row numbers). Typing replaces the task's incoming links, changing only what differs (existing links keep their ids), as one undo step.
- **Typed durations** (`3d`, `4h`, `2w`, a bare number means days) are parsed in `pm-core::text`, so file import can reuse it. `0` makes a milestone and keeps the stored duration, so a later non-zero value restores a regular task. Formatting is exact (decimals only when they read back the same).
- **Outdent** moves the task to just below its summary; the following siblings stay where they are. Microsoft Project instead makes them children of the outdented task. The simpler rule is easier to predict; revisit if the owner prefers MSP's behaviour.
- **Start and Finish are read-only in the grid.** Typing a start date in Microsoft Project silently adds a "start no earlier than" constraint, a common source of confusion. Constraints get their own visible control later (progressive disclosure, spec 5.1.8).
- **Virtualized DOM rows** (only visible rows rendered) with ARIA grid roles and `aria-activedescendant`, so VoiceOver follows the active cell.
- **⌘Z while typing in a cell** undoes the typing: the grid tells Rust when an editor is open, and the Undo/Redo menu actions are then forwarded to the text field.
- **Errors appear under the cell** that caused them, and the editor stays open with the text so it can be fixed (spec 5.1.5).

## 2026-10-09, step 7 (Gantt chart)

- **One canvas, visible rows only,** sharing row height and vertical scroll with the grid. Colours come from CSS variables, so the chart follows light and dark mode.
- **Critical bars are red and hatched**, and critical links are red, so the critical path is clear without colour vision (spec 5.1.9).
- **Dragging a bar sets "start no earlier than"** on the day it is dropped (snapped to working time by Rust), as Microsoft Project does. The undo name is "Move Task". Dragging earlier than the links allow has no effect on the dates, because links still win; showing a hint when that happens is a follow-up.
- **Dragging the right end** sets the duration so the task finishes at the end of work on that day.
- **Dragging from one bar onto another** creates a finish-to-start link. A mostly horizontal drag moves; a mostly vertical one links.
- **Zoom:** pinch, ⌘-scroll, or the − and + buttons. Shift-scroll moves sideways.
- **The chart is mouse-only for now.** Everything it does can also be done from the keyboard in the grid, except moving a task by date, which arrives with the constraint control. The canvas has a text description for VoiceOver.
- **Debug builds only:** Help > Open Test Project (5,000 or 20,000 tasks) and an edit timing readout in the status bar, for the performance check.
- **A dev-only mock backend** (`apps/desktop/src/dev/mockBackend.ts`) lets the UI run in a plain browser at `localhost:1420` (`?n=5000` generates a large project). It never loads inside Tauri or in release builds.

## 2026-10-09, step 8 (projects as Mac documents)

- **New Project asks where to save first,** then creates and opens the file. Every edit is saved immediately, so a project needs a home from the start; Final Cut Pro and Logic work this way. An "Untitled" project in a hidden folder that must be moved later was the alternative; it tends to lose track of files.
- **One open project at a time.** Opening another project closes the current one, with a backup first; nothing is lost. A Mac document app normally opens one window per document; that needs per-window state, and is proposed for iteration 2.
- **Closing the window closes the project and hides the window;** the app stays in the Dock. Clicking the Dock icon, File > New Project or File > Open brings the window back.
- **Finder:** `.pmproj` is declared as an exported type (`app.superpm.desktop.project`, conforming to `public.database`), with Super PM as the owner. Double-clicking opens the file, but only in a built app (`tauri build`), not in `tauri dev`. Verified in the built `Info.plist`.
- **Open Recent:** the last 10 projects that still exist, kept in `recent.json` in the app's config folder, with Clear Menu. The start screen lists them too.
- **Local backups:** a timestamped copy in `~/Library/Application Support/app.superpm.desktop/Backups/<project name> (<id>)/` every 15 minutes if the project changed, and when it is closed, replaced or the app quits. The newest 20 per project are kept. File > Open Backup… opens one (it opens as its own file; to restore, use Save a Copy over the original, or keep working in the backup).
- **Save a Copy** uses SQLite's backup API, so the copy is one complete file even while the project is open.
- **tauri-plugin-dialog 2.7.3** rather than 2.8.1, which was released eight days ago. The dialogs run from Rust, so the web view needed no new permissions.

## 2026-10-09, step 9 (Microsoft Project XML)

- **Import reads MSPDI into a small element tree** and takes what it supports; everything else goes into a report in three groups: imported, changed to fit (with how), and not imported (with why), each with counts and example task names. Fields Super PM calculates itself (dates, slack, critical) are listed once as recalculated, not as losses. Nothing is dropped silently.
- **Supported:** outline (from OutlineLevel), names, durations, milestones, notes, % complete, ASAP / Start No Earlier Than / Must Start On, all four link types with lag, the project calendar (week, hours, derived from a base calendar, single and consecutive exception days, in both the old and new exception formats).
- **Approximated, and reported** (open question 3): elapsed durations and lags (converted at 480 working minutes per 24 hours), percentage lags (converted using the predecessor's duration), unsupported constraints (imported as ASAP), manual scheduling, inactive tasks, milestones with a duration, schedule-from-finish.
- **Not imported, and reported:** resources, assignments, costs, work, baselines, actuals, deadlines, custom fields, task calendars, other calendars, recurring exceptions other than consecutive days, alternative work weeks, links to other projects, and links on summary tasks (open question 2). Links go through the same checks as typed links, so anything Super PM cannot hold is reported with the reason.
- **Export** writes the supported subset plus computed start and finish dates, UIDs in outline order, and one base calendar with exceptions in the 2007+ format. Microsoft Project recalculates on open.
- **Tests:** a round-trip property test (random projects with outlines, all link types, lags, constraints, notes with markup-like text and emoji, and calendar exceptions) checks that export then import keeps everything and reports nothing; a second export is byte-identical. A hand-written fixture in Microsoft Project 2016's style checks the report. **Real files from the owner are still needed** (spec section 8); the fixture is not real Microsoft Project output.
- **Imported projects are saved as a new `.pmproj`** next to the XML by default; the XML file is never changed.

## 2026-10-09, step 10 (first run, help, accessibility)

- **First run** (once per machine): opens the sample project and starts a five-step tour that points at the real controls. Escape or Skip ends it; Help > Show Tour repeats it. The "seen" flag is in the web view's local storage: if that is cleared, the tour shows again, which is harmless.
- **"What does this mean?"** (spec 5.1.7): a "?" next to Duration, Predecessors and Slack, and Help > Super PM Help (⌘?) with every term and keyboard shortcut. One source of wording (`src/help.ts`) for headers, Help and tour.
- **⌘K command bar** (spec 5.1.2) with every command by name, including Undo and Redo with the action name. The View menu listed it, so it is built rather than left as a greyed-out item.
- **Accessibility checklist** in `docs/accessibility-checklist.md`, run per release. Not yet run on the real app with VoiceOver; contrast has not been measured.

## 2026-10-09, closing in-scope gaps

- **Calendar and constraints were only half built:** the engine supported them, but the UI could not set working hours, holidays, the project start, "must start on", notes or % complete. Added Task > Task Info (⌘I), Project > Project Info and Project > Working Time. Each saves as one undo step ("Change Task Info", "Change Project Start", "Change Calendar"). Constrained tasks get a ⌖ marker in the grid with the date in its label, so a fixed date is never hidden.
- **Working hours are typed as text** ("08:00–12:00, 13:00–17:00"). Rust validates them and explains what is wrong; a week with no working time is refused.
- **Return selects the cell's text** when editing starts, so typing replaces it, as Finder does with names. Typing a character directly still starts fresh with that character.
- **Dragging a bar earlier than its links allow** now explains itself in the status bar ("cannot start before … : its predecessors finish later"), instead of leaving the bar where it was without a word.
- **Edit > Find (⌘F)**: a find field above the grid; matches task names as you type (from the current row), Return jumps to the next match, Escape closes.
