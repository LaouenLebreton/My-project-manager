# Performance log

Targets (spec section 3): recalculation under 100 ms for 5,000 tasks; 20,000 as a stress test.

Benchmark: `cargo run --release -p pm-core --example bench_schedule`. It generates a project of
phases with 10 chained tasks each, links between recent phases, all four link types, lags
from -1 to +1 day, milestones, and 40 holidays. Times are one full `schedule()` call,
averaged over 20 runs.

| Date | Change | 5,000 tasks | 20,000 tasks | Machine |
| --- | --- | --- | --- | --- |
| 2026-10-08 | First engine | 20.9 ms | 310.6 ms | Apple silicon Mac (owner's) |
| 2026-10-08 | `working_between` skips whole weeks without exceptions | 6.7 ms | 59.3 ms | same |

The first version measured slack by walking every day between early and late finish, and
tasks in parallel phases had years of slack. Skipping whole weeks fixed it; a property test
checks the fast count against the day-by-day reference.

This is the engine alone. IPC and rendering are measured in steps 5 and 7.

## Full edit cycle in the app layer (step 7)

`cargo test --release -p super-pm edit_cycle -- --ignored --nocapture`: one duration edit on a
generated project, averaged over 10 edits. "Save + schedule" includes the SQLite transaction
(WAL, `synchronous = FULL`).

| Date | Tasks | Save + schedule | Build snapshot | JSON | Snapshot size |
| --- | --- | --- | --- | --- | --- |
| 2026-10-09 | 5,000 | 2.8 ms | 6.0 ms | 2.4 ms | 3.0 MB |
| 2026-10-09 | 20,000 | 12.7 ms | 25.2 ms | 10.6 ms | 12.2 MB |

The Rust side is well inside the 100 ms target. Still to measure, in the real app: IPC transfer
of the snapshot, JSON parsing in the web view, and drawing. Use Help > Open Test Project
(debug builds); the status bar shows each edit's round trip and time to draw. If the 3 MB
snapshot turns out to dominate, the next step is sending only what changed (spec 4.9).

The grid renders only visible rows, and the Gantt chart is one canvas that draws only visible
rows and the links that cross them, so scrolling cost does not grow with project size.

## UI side of an edit (2026-10-09)

Measured in the built-in browser (Chromium) against the dev mock backend with generated
projects (`?n=5000`, `?n=20000`), editing durations from the keyboard. "JSON" is a
stringify-and-parse of the snapshot, standing in for the IPC transfer; "to grid updated" runs
from the edit request to React's DOM update. Chart drawing was timed separately (one canvas
frame). The app itself uses Safari's engine (WKWebView), so these are estimates until measured
in the app (Help > Open Test Project shows the same numbers in the status bar).

| Tasks | Snapshot | JSON | Edit to grid updated | Chart frame |
| --- | --- | --- | --- | --- |
| 5,000 | 2.1 MB (mock) / 3.0 MB (Rust) | ~5 ms | ~28 ms (79 ms first edit) | < 1 ms |
| 20,000 | 8.4 MB (mock) / 12.2 MB (Rust) | ~20 ms | ~115–125 ms, inflated by a second JSON pass in the harness | ~3 ms |

**Estimated total per edit:** about 45 ms at 5,000 tasks (target: under 100 ms, met), about
150 ms at 20,000 (stress test, no target). If 20,000-task projects matter, the next steps are,
in order: drop fields the UI does not use from the snapshot (late dates), memoise per-snapshot
lookups in the grid and chart, then send only what changed (spec 4.9).
