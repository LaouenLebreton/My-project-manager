// Plain-language explanations of scheduling terms (spec 5.1.7). Shared by the column
// headers, the Help window and the tour, so the wording is the same everywhere.

export const GLOSSARY = {
  duration: {
    term: "Duration",
    text: "How much working time a task takes, counted on the project calendar: 3d is three working days, 4h four hours, 2w two weeks. A duration of 0 makes the task a milestone.",
  },
  predecessors: {
    term: "Predecessors and links",
    text: "The tasks that must happen first, by row number: 2 means “start after task 2 finishes”. Add SS to start together (5SS), FF to finish together, or SF. Add a lag to wait (2+1d) or overlap (2-1d).",
  },
  lag: {
    term: "Lag",
    text: "Extra working time between linked tasks. +2d waits two working days after the predecessor; -1d lets the tasks overlap by a day.",
  },
  slack: {
    term: "Slack",
    text: "How long a task can slip without moving the project’s finish date. A task with no slack is critical.",
  },
  critical: {
    term: "Critical path",
    text: "The chain of tasks with no slack. If any of them is late, the whole project finishes late. Critical tasks are marked ▲ in the list and drawn as red striped bars.",
  },
  milestone: {
    term: "Milestone",
    text: "A moment rather than a piece of work, such as “Lease signed”. Shown as ◆. Type 0 as the duration to make one.",
  },
  constraint: {
    term: "Start no earlier than",
    text: "Set when you drag a bar: the task will not start before that date, even if its predecessors finish sooner. Links can still push it later.",
  },
} as const;

export type Term = keyof typeof GLOSSARY;

export const SHORTCUTS: [string, string][] = [
  ["Return", "Edit the cell; while editing, save and go down"],
  ["Escape", "Cancel editing"],
  ["Tab / ⇧Tab", "Indent / outdent the selected tasks"],
  ["↑ ↓ ← →", "Move between cells; ⇧ extends the selection"],
  ["⌘↑ / ⌘↓", "First / last row"],
  ["Delete", "Delete the selected tasks"],
  ["⌘Z / ⇧⌘Z", "Undo / redo any change"],
  ["⌘I", "Task Info: constraint, % complete, notes"],
  ["⌘F", "Find a task by name; Return for the next match"],
  ["⌘K", "Command bar: run any command by name"],
  ["⌘N / ⌘O", "New project / open a project"],
  ["⌘-scroll or pinch", "Zoom the chart"],
];
