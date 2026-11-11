# Iteration 5: easier to use, closer to Microsoft Project, better for teams

Owner (2026-11-11): do groups 2, 3 and 4 of the improvement list. Small slices, each usable on
its own, test-first, in this order.

## A. Easier to use

1. **[Done]** **Reorder tasks:** Task > Move Up / Move Down (⌃⌘↑ / ⌃⌘↓) for the selection, and dragging
   rows by their number. Tasks keep their links; summaries move with what is inside them.
2. **[Done]** **Copy, paste, duplicate tasks** (⌘C / ⌘V / ⌘D): tasks with the links between them and their
   resources; across projects too.
3. **[Done]** **Filters** in the task list: all, late, critical, milestones, a resource's tasks.
4. **[Done]** **Undo history** you can see: Edit > History of Changes…, step back to any point.
5. **First steps:** a short checklist on a new project (add tasks, link them, assign people,
   set a baseline), each step ticked as it is done.
6. **Accessibility pass** against `accessibility-checklist.md`.

## B. Closer to Microsoft Project

7. **Deadlines:** a date a task should finish by, without constraining it; a flag when it
   finishes later; read and written in Project XML.
8. **Exports:** CSV of the task list; a printable status report.
9. **Elapsed durations and percentage lags** (`3ed`, `50%`), read from Project XML exactly.
10. **Resource calendars:** each person's days off and part-time weeks.
11. **Task types:** fixed units, fixed work, fixed duration, and effort-driven.

## C. Teams

12. **Portfolio groups shared by the team** (stored on the team server).
13. **More automation:** when a task becomes late, when a milestone is reached.
14. **Invitations by email:** the server sends them when it has a mail account (SMTP settings);
    without one, invites are copied and sent by hand as now.
15. **Hosting the team server online:** needs the owner's hosting account (`docs/deploy.md`).
