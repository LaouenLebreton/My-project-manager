# Super PM

A desktop project-management app for macOS (Windows later): Microsoft Project's scheduling
(dependencies, calendars, constraints, critical path) with a modern, Mac-native interface.
Built with Tauri 2 (Rust) and React. This is iteration 1, a prototype; see the spec's section 3
for scope and `docs/` for decisions.

## What works

- Projects are single `.pmproj` files (SQLite). Every edit is saved immediately; undo and redo
  survive closing and reopening (last 500 edits). Timestamped backups every 15 minutes and on close.
- Task grid: keyboard-first editing, outline with indent and outdent, typed durations (`3d`, `4h`),
  predecessors in Microsoft Project syntax (`2, 5SS+1d`).
- Gantt chart: bars, milestones, summaries, link arrows, critical path (red and hatched); drag
  to move, stretch, or link.
- Scheduling: four link types with positive and negative lag, project calendar with holidays,
  "start no earlier than" and "must start on", critical path and slack, loop detection.
- Microsoft Project XML import and export, with a report of anything not imported exactly.
- File > New, Open, Open Recent, Save a Copy, Open Backup; `.pmproj` opens from Finder.
- First run opens a sample project with a short tour; Help explains every term; ⌘K runs any command.

## Requirements

- macOS with Xcode Command Line Tools (`xcode-select --install`)
- Rust (stable) via [rustup](https://rustup.rs)
- Node.js 20 or later with npm

## Build and run

```sh
cd apps/desktop
npm install
npm run tauri dev          # run the app with live reload
npm run tauri build -- --bundles app   # build target/release/bundle/macos/Super PM.app
```

Opening `.pmproj` files from Finder works only in the built app.

## Tests

```sh
cargo test --workspace                 # all Rust tests (core, store, interop, app)
cargo clippy --workspace --all-targets # lints
cd apps/desktop && npx tsc --noEmit    # front-end type check
```

More property-test cases: `PROPTEST_CASES=5000 cargo test --release --workspace`.

Benchmarks (spec section 3; results in `docs/performance.md`):

```sh
cargo run --release -p pm-core --example bench_schedule
cargo test --release -p super-pm edit_cycle -- --ignored --nocapture
```

In a debug build, Help > Open Test Project (5,000 or 20,000 tasks) opens a generated project,
and the status bar shows each edit's round trip and drawing time.

## Working on the UI in a browser

With `npm run dev` (or `npm run tauri dev`) running, http://localhost:1420 shows the UI against
a stand-in backend (`apps/desktop/src/dev/mockBackend.ts`) with fake dates. Add `?n=5000` for a
large project. The stand-in never loads inside the app or in release builds.

## TypeScript types

The IPC types in `apps/desktop/src/bindings/` are generated from Rust with ts-rs. After changing
a type in `apps/desktop/src-tauri/src/ipc.rs` or `edits.rs`, run `cargo test -p super-pm`.

## Layout

```
crates/pm-core      scheduling engine, data model, commands, calendar maths (pure Rust)
crates/pm-store     SQLite file format, change log, undo history
crates/pm-interop   Microsoft Project XML import and export
apps/desktop        Tauri app: src-tauri (Rust) and src (React)
docs                decisions, scheduling rules, performance, open questions, checklists
fixtures            sample and reference files
```

## Open questions for the owner

See `docs/open-questions.md`. Most important: real Microsoft Project XML files with their
calculated dates, to check that Super PM's dates match (spec section 8).
