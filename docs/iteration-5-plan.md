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
5. **[Done]** **First steps:** a short checklist on a new project (add tasks, link them, assign people,
   set a baseline), each step ticked as it is done.
6. **[Done]** **Accessibility pass** against `accessibility-checklist.md`.

## B. Closer to Microsoft Project

7. **[Done]** **Deadlines:** a date a task should finish by, without constraining it; a flag when it
   finishes later; read and written in Project XML.
8. **[Done]** **Exports:** CSV of the task list; a printable status report.
9. **[Done]** **Elapsed durations and percentage lags** (`3ed`, `50%`), read from Project XML exactly.
10. **[Done]** **Resource calendars:** each person's days off and part-time weeks.
11. **[Done]** **Task types:** fixed units, fixed work, fixed duration, and effort-driven.

## C. Teams

12. **[Done]** **Portfolio groups shared by the team** (stored on the team server).
13. **More automation:** when a task becomes late, when a milestone is reached.
14. **Invitations by email:** the server sends them when it has a mail account (SMTP settings);
    without one, invites are copied and sent by hand as now.
15. **Hosting the team server online:** needs the owner's hosting account (`docs/deploy.md`).

## D. Dashboard and progress (owner, 2026-11-12)

16. **[Done]** **Progress bar** in the header of every view: work done against work planned
    by now (baseline dates if any, else the schedule), coloured by status, with the reasons on
    hover.
17. **[Done]** **Dashboard** (first tab, ⌘9): health with its reasons, progress against plan,
    finish against the baseline, budget (with CPI/SPI once there is a baseline), late, due this
    week, blocked (waiting on a late task), next milestones, missed deadlines, overbooked
    people, recent changes (the team's history in a team project). One calculation
    (`health.ts`) serves the dashboard, the progress bar and the Status Report.

    What it shows comes from what project managers say they need. Reddit itself cannot be read
    by the tools used, so this relies on an analysis of r/projectmanagement posts by a tool
    vendor (oniq-landing.pages.dev/blog/reddit-pm-tools: tool gaps, scattered information and
    no single source of truth, wanting progress pushed to people rather than chased) and on
    practitioner guides (thedigitalprojectmanager.com, plane.so, productive.io: a few metrics
    that drive a decision, a red/amber/green signal first, schedule and budget against plan,
    blockers, workload, milestones). Treat them as vendor-leaning.

