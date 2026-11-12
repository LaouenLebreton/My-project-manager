# Iteration 6: easier to find, easier to see

The owner's feedback after using v0.1.3 (2026-11-12), and a review of the whole app from first
principles: what a project manager needs to do every day, and how many clicks each takes.

## The owner's feedback

| What was wrong | What changed |
|---|---|
| A task's attributes cannot be seen in the WBS; they should show in the Gantt view too. | **[Done]** A **Task Details** panel beside the Gantt Chart and the WBS (Details button, ⇧⌘D): every attribute of the selected task, the common ones editable in place. WBS boxes now show dates, people, cost and a progress bar. |
| Separate tabs for Gantt and WBS; a drawer with all the views is wanted. | **[Done]** A **views drawer** down the left (like Microsoft Project's View Bar), grouped Overview / Plan / Resources / Team, foldable to icons. "Tasks" is now called **Gantt Chart**. |
| A new task puts itself under the task above. | **[Done]** Typing in the blank row adds a **top-level task at the end**. **Insert Task** (⇧⌘Return, Task menu, right-click) adds above the selected task at its level, as in Project; **Add Subtask** adds inside it. |
| No numbering column to see where tasks go in the WBS. | **[Done]** A **WBS** column (1, 1.2, 1.2.3), shown by default. The **#** column keeps each task's project number even when rows are hidden. |
| A limited number of columns; they should be added and removed at will. | **[Done]** **30 columns** to choose from (⊕ at the top right of the list, or right-click a header), grouped General / Schedule / Progress / Cost / Baseline / Advanced. Drag a header to move a column, drag its edge to resize; kept on this computer. New editable columns: Start, Finish, Work, Deadline, Fixed Cost, Actual Start and Finish, Mode, Task Type, Effort Driven, Notes. |
| Right-click only offers Reload. | **[Done]** **Right-click menus** on rows, bars, WBS boxes, Board cards and column headers. The web view's own menu (with Reload) is gone; text fields keep Cut / Copy / Paste. Double-click a bar or card for Task Info. |
| The progress bar overlaps the view tabs. | **[Done]** It now sits beside the project's name; the views moved to the drawer. |

Also found while reviewing and fixed: summaries could not be **collapsed** (now ▸ ▾, ⌥← ⌥→, and
Levels 1 / 2 / 3 / All above the list); with a filter on, row numbers no longer matched the
predecessors that refer to them; Task menu commands did nothing outside the Gantt Chart (they now
switch to it); the Chat view was hidden until you joined a team (it now explains teams and
offers Create / Join / Sign In).

## First principles

Who uses it: a project manager who plans in Microsoft Project's way (tasks, links, resources,
baseline) and runs the project with a team (updates, chat, status). What matters, in order:

1. **See where the project stands** without building anything: the Dashboard, the progress bar.
2. **Change the plan quickly**: type in the list, drag bars, right-click anything.
3. **See everything about one task** without opening a dialog: the details panel.
4. **Coordinate**: chat, history, automatic posts, invites.
5. **Look simple**: one way into each view (the drawer), menus that name things the way
   Microsoft Project does, nothing shown that is not needed.

### Three clicks or fewer

Counted from the Gantt Chart with a task selected (a menu-bar command is two clicks: the menu,
the item).

| Job | How | Clicks |
|---|---|---|
| Open any view | Drawer | 1 |
| Add a task | Blank row, or right-click > Insert Task Above | 1–2 |
| Add a subtask | Right-click > Add Subtask | 2 |
| Rename, change duration, dates, people | Click the cell (or the details panel) and type | 1 |
| Link two tasks | Drag one bar onto the other, or right-click > Link to the Task Above | 1–2 |
| Mark progress | Right-click > 0% / 50% / 100%, or drag the Board card | 2 |
| See every attribute of a task | Details panel (open by default) | 0–1 |
| Add or remove a column | ⊕ > the column | 2 |
| Collapse a phase / show level 1 | ▸ / Levels > Level 1 | 1–2 |
| Filter late, critical, someone's tasks | Show > the filter | 2 |
| Set a baseline | Project > Set Baseline | 2 |
| Status report, print it | Project > Status Report > Print | 3 |
| Level resources | Project > Level Resources | 2 |
| Working time and holidays | Project > Working Time | 2 |
| A person's days off | Drawer > Resource Sheet > the person | 2–3 |
| Undo to any earlier point | Edit > History of Changes > the step | 3 |
| Chat with the team | Drawer > Chat | 1 |
| Invite someone | People in the header > Invite (or Team > Invite People) | 2 |
| Export (Project XML, CSV) | File > Export | 2 |
| Any command by name | ⌘K and type | 1 + typing |

Everything a project manager does daily is within three clicks.

## Next (proposed, in order)

1. **Comments on a task**, with @mentions that notify the person: coordination where the work
   is, not only in channels. Shown in the details panel.
2. **My Tasks**: one list of everything assigned to me across projects, with what is due this
   week (the most asked-for view in PM forums).
3. **Timescale** presets (days / weeks / months / quarters) and **Scroll to Task** on the Gantt
   Chart; today/status-date line labels.
4. **Custom fields** (text, number, date, yes/no, list) as columns, as Project's Text1…/Flag1…
5. **Column layouts saved in the project** (and shared with the team), with named tables
   (Entry, Cost, Tracking, Schedule) as in Project; today they are kept per computer.
6. **Templates** for new projects (software release, event, construction phase).
7. The details panel docked **under** the chart on narrow windows (Project's Task Form), so the
   timeline keeps its width.
