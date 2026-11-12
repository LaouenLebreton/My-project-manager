import { useState } from "react";

/** The project's views, in the drawer's order. */
export type View = "dashboard" | "tasks" | "wbs" | "board" | "calendar" | "resources" | "workload" | "history" | "chat";

interface Item {
  view: View;
  label: string;
  shortcut: string;
  /** What it shows, for the tooltip. */
  hint: string;
  icon: React.ReactNode;
}

const stroke = { fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round", strokeLinejoin: "round" } as const;
const svg = (children: React.ReactNode) => (
  <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden>
    {children}
  </svg>
);

const ICONS = {
  dashboard: svg(
    <g {...stroke}>
      <rect x="2.5" y="2.5" width="5.5" height="6" rx="1" />
      <rect x="10" y="2.5" width="5.5" height="3.5" rx="1" />
      <rect x="2.5" y="10.5" width="5.5" height="5" rx="1" />
      <rect x="10" y="8" width="5.5" height="7.5" rx="1" />
    </g>,
  ),
  tasks: svg(
    <g {...stroke}>
      <path d="M2.5 4h6M5 9h8M8 14h7.5" strokeWidth="2.6" />
    </g>,
  ),
  wbs: svg(
    <g {...stroke}>
      <rect x="6.5" y="2" width="5" height="3.5" rx="0.8" />
      <rect x="1.5" y="12.5" width="5" height="3.5" rx="0.8" />
      <rect x="11.5" y="12.5" width="5" height="3.5" rx="0.8" />
      <path d="M9 5.5v3.5M4 12.5V9h10v3.5" />
    </g>,
  ),
  board: svg(
    <g {...stroke}>
      <rect x="2" y="2.5" width="4" height="13" rx="1" />
      <rect x="7" y="2.5" width="4" height="9" rx="1" />
      <rect x="12" y="2.5" width="4" height="6" rx="1" />
    </g>,
  ),
  calendar: svg(
    <g {...stroke}>
      <rect x="2.5" y="3.5" width="13" height="12" rx="1.5" />
      <path d="M2.5 7.5h13M6 2v3M12 2v3" />
    </g>,
  ),
  resources: svg(
    <g {...stroke}>
      <circle cx="6.5" cy="6" r="2.5" />
      <path d="M2 15c0-2.6 2-4.5 4.5-4.5S11 12.4 11 15" />
      <circle cx="12.5" cy="6.5" r="2" />
      <path d="M12 10.6c2.3 0 4 1.7 4 4.2" />
    </g>,
  ),
  workload: svg(
    <g {...stroke}>
      <path d="M2.5 15.5h13" />
      <path d="M4.5 15V9M8 15V5M11.5 15v-7M15 15V3" strokeWidth="2.2" />
    </g>,
  ),
  history: svg(
    <g {...stroke}>
      <path d="M3 9a6 6 0 1 0 1.8-4.3" />
      <path d="M3 3v3.2h3.2M9 5.5V9l2.5 1.8" />
    </g>,
  ),
  chat: svg(
    <g {...stroke}>
      <path d="M3 4.5A1.5 1.5 0 0 1 4.5 3h9A1.5 1.5 0 0 1 15 4.5v6a1.5 1.5 0 0 1-1.5 1.5H8l-3.5 3v-3h0A1.5 1.5 0 0 1 3 10.5z" />
    </g>,
  ),
} satisfies Record<View, React.ReactNode>;

const GROUPS: { title: string; items: Item[] }[] = [
  {
    title: "Overview",
    items: [{ view: "dashboard", label: "Dashboard", shortcut: "⌘9", hint: "The project at a glance: health, progress, what is late", icon: ICONS.dashboard }],
  },
  {
    title: "Plan",
    items: [
      { view: "tasks", label: "Gantt Chart", shortcut: "⌘1", hint: "The task list with its timeline", icon: ICONS.tasks },
      { view: "wbs", label: "WBS", shortcut: "⌘3", hint: "Work breakdown structure: the project as a tree of boxes", icon: ICONS.wbs },
      { view: "board", label: "Board", shortcut: "⌘6", hint: "Tasks as cards: not started, in progress, done", icon: ICONS.board },
      { view: "calendar", label: "Calendar", shortcut: "⌘5", hint: "Tasks on a month calendar", icon: ICONS.calendar },
    ],
  },
  {
    title: "Resources",
    items: [
      { view: "resources", label: "Resource Sheet", shortcut: "⌘2", hint: "People, their rates and working time", icon: ICONS.resources },
      { view: "workload", label: "Workload", shortcut: "⌘4", hint: "Hours per person per day", icon: ICONS.workload },
    ],
  },
  {
    title: "Team",
    items: [
      { view: "chat", label: "Chat", shortcut: "⌘8", hint: "Your teams' channels and posts", icon: ICONS.chat },
      { view: "history", label: "History", shortcut: "⌘7", hint: "Every change in this team project, and who made it", icon: ICONS.history },
    ],
  },
];

const COLLAPSED_KEY = "superpm.drawerCollapsed";
function storedCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * The views of the project in one place, down the left side (like Microsoft Project's View
 * Bar): one click to any view. It folds down to icons for more room.
 */
export function ViewDrawer({
  view,
  onView,
  available,
  badges,
}: {
  view: View;
  onView: (v: View) => void;
  /** Views not offered here (e.g. History outside a team project) are left out. */
  available: (v: View) => boolean;
  /** A count or mark shown next to a view (new chat posts, over-allocated people). */
  badges: Partial<Record<View, string>>;
}) {
  const [collapsed, setCollapsed] = useState(storedCollapsed);
  const toggle = () => {
    setCollapsed((c) => {
      try {
        localStorage.setItem(COLLAPSED_KEY, c ? "0" : "1");
      } catch {
        // a convenience only
      }
      return !c;
    });
  };
  return (
    <nav className={collapsed ? "drawer collapsed" : "drawer"} aria-label="Views">
      {GROUPS.map((g) => {
        const items = g.items.filter((i) => available(i.view));
        if (!items.length) return null;
        return (
          <div className="drawer-group" key={g.title} role="group" aria-label={g.title}>
            <h2>{g.title}</h2>
            {items.map((i) => (
              <button
                key={i.view}
                className={view === i.view ? "drawer-item on" : "drawer-item"}
                aria-current={view === i.view ? "page" : undefined}
                title={`${i.hint} (${i.shortcut})`}
                onClick={() => onView(i.view)}
              >
                {i.icon}
                <span className="drawer-label">{i.label}</span>
                {badges[i.view] && <span className="drawer-badge">{badges[i.view]}</span>}
              </button>
            ))}
          </div>
        );
      })}
      <button className="drawer-toggle" onClick={toggle} aria-label={collapsed ? "Show view names" : "Hide view names"} title={collapsed ? "Show view names" : "Hide view names"}>
        {collapsed ? "»" : "«"}
      </button>
    </nav>
  );
}
