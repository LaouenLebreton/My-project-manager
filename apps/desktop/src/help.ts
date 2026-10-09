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
  resources: {
    term: "Resources",
    text: "Who (or what) works on a task. Type names separated by commas: Alex, Sam. Add a share of their time in brackets, Sam[50%], or a quantity for materials, Concrete[2.5 tons]. A new name adds that resource to the Resource Sheet. You can also assign from the Resource Sheet: type task names (or row numbers) in a resource's Tasks column.",
  },
  wbs: {
    term: "Work breakdown structure (WBS)",
    text: "The project broken down into phases and the work inside them, drawn as a tree. The codes (1, 1.2, 1.2.3) show where each task sits. Build it in the WBS view with Return (new task) and Tab (subtask), or drag boxes to reorganise.",
  },
  baseline: {
    term: "Baseline",
    text: "A saved copy of the plan (Project > Set Baseline). Grey bars under the tasks show where the baseline had them; Finish Var. shows how many working days later (+) or earlier (−) each task now finishes.",
  },
  cost: {
    term: "Cost",
    text: "What a task costs: its resources' work times their rates (Rate in the Resource Sheet), plus any fixed cost (Task Info). Summaries add up the tasks inside them. Set the currency in Project Info.",
  },
  progress: {
    term: "% Done",
    text: "How much of a task is finished. Typing a percentage records the actual start (and, at 100%, the actual finish) from the schedule; those dates then stay fixed whatever happens to the links. Project > Update Project fills this in for every task up to a day.",
  },
  earned: {
    term: "Earned value",
    text: "Compares the work done with the baseline, in money, as of the status date. Planned value is what should be done by then, earned value what is done, actual cost what it cost. An SPI or CPI below 1 means behind schedule or over budget. Keep % Done up to date for meaningful numbers.",
  },
  board: {
    term: "Board",
    text: "Work tasks as cards. Drag a card (or use its arrows) to In progress or Done: that sets % Done to 50% or 100% and records the actual dates, as typing it in the task list does.",
  },
  workload: {
    term: "Workload",
    text: "Hours each person is asked to work, day by day or week by week, with the tasks underneath. Red means more than they have that day (their working time × Max Units). Click a task to find it in the task list.",
  },
  overallocated: {
    term: "Over-allocated",
    text: "A resource asked for more than it has at the same time, such as one person on two full-time tasks at once. Marked ⚠. Move a task, share the work, or lower the units to fix it.",
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
  ["⌘1 … ⌘6", "Tasks / Resource Sheet / Work breakdown structure / Workload / Calendar / Board"],
  ["Return / Tab (WBS)", "Add a task / a subtask in the WBS chart; Space renames"],
  ["⌘K", "Command bar: run any command by name"],
  ["⌘N / ⌘O", "New project / open a project"],
  ["⌘-scroll or pinch", "Zoom the chart"],
];
