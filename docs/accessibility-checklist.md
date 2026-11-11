# Keyboard and accessibility checklist

Run before each release (spec sections 5.1.9 and 6). Turn on VoiceOver (⌘F5) for the marked items.

## Keyboard only (no mouse or trackpad)

- [ ] Start screen: Tab reaches New Project…, Open…, Open Sample Project and each recent project; Return activates.
- [ ] ⌘N, ⌘O, ⇧⌘I and ⌘K work from anywhere.
- [ ] Grid: arrows move between cells; ⌘↑/⌘↓ jump to the first and last row; Page Up/Down page.
- [ ] Return edits; Return saves and moves down; Escape cancels and restores the text.
- [ ] Typing a letter starts editing with that letter; nothing typed quickly is lost.
- [ ] In the blank last row, a name plus Return adds a task and moves to the next blank row.
- [ ] Tab and Shift-Tab indent and outdent the selected rows, including while editing.
- [ ] Shift-arrow selects several rows; Delete removes them; ⌘Z brings them back.
- [ ] ⌘Z while typing in a cell undoes the typing, not the last project change.
- [ ] Predecessors and durations can be typed; mistakes show a message under the cell and keep the text.
- [ ] Each "?" in the column headers is reachable with Tab and opens with Space or Return; its text closes when focus moves on.
- [ ] Command bar: ⌘K opens with focus in the field; ↑↓ choose; Return runs; Escape closes.
- [ ] Help (⌘?) and the import report close with Escape and return focus.
- [ ] ⌘1 and ⌘2 switch between Tasks and the Resource Sheet; the Resource Sheet works from the keyboard like the task grid (arrows, Return, typing, Delete, blank row adds).
- [ ] WBS view (⌘3): announced as a tree; every box is read with its code, name, duration and markers; Return, Tab, Space, Delete and the arrows work without the mouse; +N / − buttons open and close branches.
- [ ] Over-allocation is announced in words (the ⚠ markers have labels; the details under the Resource Sheet are read out).
- [ ] Task Info (⌘I), Project Info and Working Time: every field has a label, Tab moves through them in order, Return saves, Escape cancels, errors are announced.
- [ ] Tour: Escape skips; buttons reachable with Tab.
- [ ] Task > Move Up / Move Down (⌃⌘↑ / ⌃⌘↓; Alt+Shift+↑/↓ on Windows) move the selected rows; dragging rows by their number is never the only way.
- [ ] ⌘C / ⌘X / ⌘V in the task list copy, cut and paste tasks; ⌘D duplicates.
- [ ] The Show menu above the list is reached with Tab and named "Show which tasks"; "Show all" follows it when a filter is on.
- [ ] Edit > Undo History: Tab moves through the changes, Return goes back to the chosen one, Escape closes.
- [ ] First steps: its buttons and its × (named "Close first steps") are reachable with Tab.
- [ ] Members & Permissions: Remove from team… asks again (Remove / Keep) before removing.

## VoiceOver

- [ ] The grid is announced as a grid; moving announces the cell (column and value).
- [ ] Critical tasks are announced as "Critical", milestones as "Milestone".
- [ ] Errors under cells are announced (alert).
- [ ] The chart is announced with its description and points to the grid for editing.
- [ ] The status bar announces drag feedback.

## Visual

- [ ] Light and dark mode: text and bars readable; critical bars recognisable by hatching, not only red.
- [ ] Larger text (System Settings > Accessibility > Display > Text size) does not cut off the grid header.
- [ ] Reduce motion: nothing animates.
- [ ] Contrast of secondary text against the background is at least 4.5:1.

## Known gaps (iteration 1)

- The chart is mouse-only; everything it does is available in the grid except moving a task to a date, which arrives with a constraint editor.
- Contrast of the hatched critical bars has not been measured yet. Secondary text (60% of the
  text colour) was worked out on 2026-11-11: about 5.7:1 on white and 7:1 on the dark
  background, above 4.5:1.
