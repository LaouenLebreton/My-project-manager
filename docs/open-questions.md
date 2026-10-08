# Open questions for the owner

Raised by Claude Code on 2026-10-08 while reading the iteration 1 spec. Nothing here has been acted on.

1. **[Decided 2026-10-08: undo survives reopen, last 500 commands]** **Undo after reopening.** Acceptance criteria say reopening restores "everything, including undo-free state". The change log is persisted anyway, so undo could survive closing and reopening (like Pages/Numbers do within a session). Proposal: undo history survives reopen, capped at the last 500 commands. Alternative: undo stack starts empty on open.
2. **Links on summary tasks.** MS Project allows them; the spec is silent. Proposal: reject in iteration 1, support in iteration 2 (see `scheduling-rules.md` R25). MSPDI files containing them would be listed in the import report.
3. **Percentage and elapsed lags/durations.** Common in MS Project files, not in the spec. Proposal: approximate on import and report them.
4. **Bundle identifier and app name** (spec section 8.3). Placeholder until answered: name `Super PM`, id `app.superpm.desktop`.
5. **Reference files** (spec section 8.1) are needed before the golden tests in step 3 can be written.
