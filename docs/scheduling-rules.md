# Scheduling rules

Status: **draft**, implemented in `crates/pm-core/src/schedule.rs` (step 3) R28 is enforced by the command layer in `command.rs`. Each rule becomes one or more tests in `pm-core` before it is implemented.
Rules marked *verify* must be checked against the owner's Microsoft Project reference files
(spec section 8) and updated here with the observed result.

## Units and time

- R1. All durations and lags are integers in **working minutes**. No floating point days anywhere in the core.
- R2. Instants are naive local date-times on the project calendar (no time zone in the core). IPC uses ISO 8601 strings without offset.
- R3. A "day" in the UI is the calendar's minutes-per-day setting (default 480). Display conversion only; never stored.

## Calendar arithmetic

- R4. `add_working(t, d)`: the instant reached after consuming `d` working minutes starting at `t`. For `d = 0` it returns `t` unchanged.
- R5. Start-type results snap **forward** to the next working instant (a task never starts at 17:00; it starts at 08:00 next working day).
- R6. Finish-type results snap **backward** to the end of the previous working period (a task that ends exactly at the end of a day finishes at 17:00, not at 08:00 next day). *verify*
- R7. `working_between(a, b)` is signed: negative when `b < a`. `add_working(a, working_between(a, b)) == snap(b)`.
- R8. Exception days override the working week for that date: either non-working, or working with their own ranges.
- R9. A calendar with no working time at all is invalid and rejected when edited, not discovered during scheduling.

## Forward pass

- R10. Tasks with no predecessors start at the project start, snapped forward (R5), subject to constraints.
- R11. Link drivers (lag `L` applied on the **successor's calendar**; *verify*, MS Project may use the predecessor or project calendar — irrelevant while there is one calendar):
  - FS: `succ.ES >= add(pred.EF, L)`
  - SS: `succ.ES >= add(pred.ES, L)`
  - FF: `succ.EF >= add(pred.EF, L)`, so `succ.ES >= sub(that, succ.duration)`
  - SF: `succ.EF >= add(pred.ES, L)`, so `succ.ES >= sub(that, succ.duration)`
- R12. Early start is the maximum over all drivers and the project start.
- R13. Negative lag overlaps tasks: FS with `L = -480` lets the successor start one working day before the predecessor finishes. The successor still cannot start before the project start. *verify*
- R14. Milestones have zero duration; `ES == EF`. A milestone following an FS link sits at the predecessor's finish (end of day, R6), not the next morning. *verify*

## Constraints

- R15. ASAP (default): no extra rule.
- R16. Start No Earlier Than `D`: `ES = max(ES_from_links, D)`.
- R17. Must Start On `D`: `ES = D` even if links demand later. When links demand later, the schedule reports a **constraint conflict** for that task (task id, required date, link-driven date). Never hidden.
- R18. Constraint dates are snapped forward to working time.

- R32. Every Microsoft Project constraint type. Finish dates mean the end of work on that day. Finish No Earlier Than holds the start back until the task can finish on or after the date. Must Finish On fixes the finish. Start/Finish No Later Than only bite when the links would make the task later: the date is honoured anyway and a conflict is reported (negative slack), as for Must Start On. In the backward pass, the "no later than" limits cap the late dates, and the "must" constraints fix them. *verify*
- R33. As Late As Possible: after the forward pass, latest tasks first, each moves as late as it can without moving its successors (at their early dates) or the project's finish. Successors therefore never move. *verify* against Microsoft Project, which may also move ALAP chains as a group.

## Backward pass, slack, critical path

- R19. Project finish = latest EF of all leaf tasks. Backward pass starts there for tasks with no successors.
- R20. Late dates mirror R11 with inequalities reversed.
- R21. Must Start On also fixes `LS = D` in the backward pass.
- R22. Total slack = `working_between(EF, LF)` on the task's calendar. Critical = total slack `<= 0`.
- R23. Negative slack is allowed and shown (results from Must Start On conflicts).

## Hierarchy

- R24. Only leaf tasks are scheduled. Summary start = min child ES, finish = max child EF; summary duration is derived and not stored as input.
- R24a. Summary late dates: min child LS, max child LF. Summary total slack = the **smallest child slack**, so a summary is critical when it contains critical work. *verify*: MS Project may compute it from the summary's own dates.
- R25. Links may start or end at a summary task (owner decision 2026-10-09, as in Microsoft Project).
  - R25a. A link *from* a summary uses the summary's rolled-up dates: it waits for every task inside. Finish-type limits in the backward pass apply to every task inside; start-type limits (SS, SF out of a summary) apply only to the tasks that start the summary, because a summary's start is its earliest task's start. *verify*
  - R25b. A link *into* a summary holds back every task inside it, at any depth. Only finish-to-start and start-to-start links may point into a summary; finish-to-finish and start-to-finish are refused, as Microsoft Project refuses them.
  - R25c. A summary cannot be linked to a task inside it (that would be a loop). Moving or inserting tasks that would cause this is refused too.
  - R25d. A summary's own Start No Earlier Than or Must Start On holds back every task inside it, like a link into it. *verify*
- R26. An empty summary (all children deleted) becomes a normal task with its last derived duration.

## Leveling (iteration 2)

- R30. A task's leveling delay (working minutes) is added after its links and Start No Earlier Than date have set its start. Must Start On tasks ignore it. The delay uses up slack, because late dates do not include it.
- R31. Level Resources starts from no delays and repeats: resolve the earliest over-allocation by delaying its least important task (most slack, then latest start, then later in the outline) until the first task it clashes with finishes. Must Start On tasks never move. A task asking for more than its resource has, and clashes between fixed-date tasks, are reported instead. The result is one undo step; Clear Leveling removes all delays. *verify* against Microsoft Project's "standard" leveling order.

## Errors

- R27. A cycle returns an error listing the task ids in one cycle, in order. No partial schedule.
- R28. Self-links and duplicate links between the same pair are rejected when the command is applied.
- R29. Same input gives byte-identical output on every platform: no hash-map iteration order in results, ties broken by task id.

## Open items to check against reference files

- Lag calendar (R11), finish-time snapping (R6), negative lag at project start (R13), milestone placement (R14).
- Percentage lags (e.g. `FS+50%`): MS Project supports them; not in spec. Proposed: import as approximated working minutes and list in the import report.
- Elapsed durations and lags (`ed`, `elapsed days`): proposed same treatment.
