# Open questions for the owner

Raised by Claude Code on 2026-10-08 while reading the iteration 1 spec. Nothing here has been acted on.

1. **[Decided 2026-10-08: undo survives reopen, last 500 commands]** **Undo after reopening.** Acceptance criteria say reopening restores "everything, including undo-free state". The change log is persisted anyway, so undo could survive closing and reopening (like Pages/Numbers do within a session). Proposal: undo history survives reopen, capped at the last 500 commands. Alternative: undo stack starts empty on open.
2. **[Done 2026-10-09: allowed, like Microsoft Project]** **Links on summary tasks.** MS Project allows them; the spec is silent. Proposal: reject in iteration 1, support in iteration 2 (see `scheduling-rules.md` R25). MSPDI files containing them would be listed in the import report.
3. **Percentage and elapsed lags/durations.** Common in MS Project files, not in the spec. Proposal: approximate on import and report them.
4. **Bundle identifier and app name** (spec section 8.3). Placeholder until answered: name `Super PM`, id `app.superpm.desktop`.
5. **Reference files** (spec section 8.1) are needed before the golden tests in step 3 can be written.

## Decided 2026-10-09 (owner)

6. **One window per project**, like Pages or Keynote (replaces "one project at a time").
7. **[Done 2026-10-09]** **Outdent works like Microsoft Project:** outdenting a task makes the tasks below it in the same summary its subtasks.
8. **Iteration 2 starts with resources and assignments.** Baselines, progress tracking and the other views move to later iterations.
9. **.mpp files:** the owner has `.mpp` files rather than XML. Options: export XML from Microsoft Project (File > Save As > XML Format), or bring forward `.mpp` reading through MPXJ (needs Java). Not decided yet.
