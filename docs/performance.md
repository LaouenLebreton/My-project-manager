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
