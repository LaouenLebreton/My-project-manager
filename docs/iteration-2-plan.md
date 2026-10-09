# Iteration 2: resources and assignments

Owner decision (2026-10-09): start iteration 2 with resources and assignments. Baselines,
progress tracking and the other views come in later iterations.

## Scope, in steps (each one committed and runnable)

Status 2026-10-09: steps 1 to 5 done; step 6 next.

1. **Model, commands, file format.** Resources (people, equipment, materials) and assignments
   (who works on which task, at what percentage). Commands with inverses, as for tasks.
   File format version 2, with a migration from version 1 files. Test-first.
2. **Resource phase of the engine** (spec 4.10: "engine split into dates and resource phases").
   From the scheduled dates, work out each resource's load over time, its peak, and the
   periods where it is over-allocated. Pure and tested like the dates phase.
3. **Resource Sheet.** A second view in each project window (Tasks | Resources): a grid of
   resources with name, initials, type, maximum units, and an over-allocation marker.
4. **Assigning from the task grid.** A "Resources" column typed as in Microsoft Project
   (`Alex, Sam[50%]`), the same list in Task Info, and resource names next to the bars.
5. **Over-allocation shown where it matters:** a marker on tasks and resources that are
   overloaded, with the dates and how much over, in plain words.
6. **Microsoft Project XML:** import and export resources and assignments (now reported as
   ignored).

After these, to confirm with the owner before starting: **leveling** (delaying tasks to
resolve over-allocation) and **costs** (rates, fixed costs, task and project totals).

## Choices made (say if any should change)

- **Durations stay the driver ("fixed duration").** Assigning people does not change a task's
  duration; work is duration × units (2 days at 50% = 8 hours). This matches Microsoft
  Project's default behaviour when resources are added to a task (fixed units, not effort
  driven). Project's other task types (fixed work, effort-driven) can follow later.
- **Units are percentages of a resource's time:** 100% = full time; a resource can have more
  than 100% maximum units (a team of three painters = 300%).
- **Resources use the project calendar** for now. Personal calendars (holidays per person)
  come with leveling.
- **No assignments on summary tasks**, which Microsoft Project discourages too: work belongs
  to the tasks inside.
- **Material resources** (concrete, licences) have a quantity instead of a percentage and are
  never over-allocated.
