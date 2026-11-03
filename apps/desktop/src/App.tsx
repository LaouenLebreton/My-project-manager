import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, chat, errorMessage, team, type Edit, type IpcError, type RecentFile, type Snapshot, type SyncStatus, type TeamStatus } from "./api";
import { AddAccountSheet, ConnectSheet, CreateTeamSheet, InvitePeopleSheet, InvitesSheet, JoinSheet, OpenSharedSheet, SignInSheet, SyncPill } from "./TeamSheets";
import type { Invite } from "./api";
import { formatDate, formatMoney } from "./format";
import { CommandBar, type CommandItem } from "./CommandBar";
import { Gantt, type GanttHandle } from "./Gantt";
import { FindBar } from "./FindBar";
import { HelpSheet } from "./HelpSheet";
import { ResourceSheet } from "./ResourceSheet";
import { ConfirmSheet, ProjectInfo, ProjectStatistics, TaskInfo, UpdateProject, WorkingTime } from "./Sheets";
import type { Row } from "./bindings/Row";
import { Tour } from "./Tour";
import { WbsChart } from "./WbsChart";
import { Workload } from "./Workload";
import { TabStrip } from "./TabStrip";
import { Board } from "./Board";
import { CalendarView } from "./CalendarView";
import { HistoryView } from "./HistoryView";
import { ChatView } from "./ChatView";
import { MembersSheet } from "./MembersSheet";
import { accessLabel } from "./permissions";
import { Grid, type GridHandle } from "./Grid";
import { ImportReport } from "./ImportReport";
import type { ReportView } from "./bindings/ReportView";

/** Window > Portfolio, from inside the page (start screen, command bar). */
const openPortfolio = () => invoke("open_portfolio").catch(() => {});
/** Home: the start page tab (made if none is open). */
const openHome = () => invoke("show_home", { newTab: false }).catch(() => {});
/** The + and File > New Tab: the start page in a new tab, to open or create another project. */
const openNewTab = () => invoke("show_home", { newTab: true }).catch(() => {});

export function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  // For menu events, whose listener is registered once.
  const selectedRef = useRef(selectedIds);
  selectedRef.current = selectedIds;
  const [message, setMessage] = useState<string | null>(null);
  const [gridWidth, setGridWidth] = useState(720);
  const [timing, setTiming] = useState<string | null>(null);
  const gridRef = useRef<GridHandle>(null);

  const [recent, setRecent] = useState<RecentFile[]>([]);
  const [report, setReport] = useState<ReportView | null>(null);
  const [overlay, setOverlay] = useState<"help" | "tour" | "commands" | "project" | "working" | "update" | "stats" | null>(null);
  const [infoRow, setInfoRow] = useState<Row | null>(null);
  const [finding, setFinding] = useState(false);
  const [view, setView] = useState<"tasks" | "resources" | "wbs" | "workload" | "calendar" | "board" | "history" | "chat">("tasks");
  const [notice, setNotice] = useState<string | null>(null);
  // The team server account, and this window's sync state (shared projects only).
  const [teamStatus, setTeamStatus] = useState<TeamStatus | null>(null);
  const [syncStatus, setSyncStatus] = useState<SyncStatus | null>(null);
  // New chat posts by others while the Chat tab is not showing.
  const [chatUnread, setChatUnread] = useState(0);
  const viewRef = useRef<string>("tasks");
  const [teamSheet, setTeamSheet] = useState<"signin" | "share" | "open" | "account" | "join" | "invite" | "invites" | "members" | "connect" | null>(null);
  const [invites, setInvites] = useState<Invite[]>([]);
  /** Closes a team sheet unless it has already been replaced by the next one. */
  const closeTeamSheet = (which: string) => setTeamSheet((s) => (s === which ? null : s));
  const signOut = useCallback(() => {
    team.signOut().then(
      (s) => {
        setTeamStatus(s);
        setNotice("Signed out of the team server. Shared projects stay on this Mac and sync again when you sign in.");
      },
      (e) => setError(errorMessage(e)),
    );
  }, []);
  const ganttRef = useRef<GanttHandle>(null);

  const run = useCallback(async (action: () => Promise<Snapshot | null>) => {
    try {
      const result = await action();
      if (result) setSnapshot(result); // null: the user cancelled a dialog
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  /** For the grid: errors are shown next to the cell, so they are passed back, not shown here. */
  const edit = useCallback(async (e: Edit) => {
    const t0 = performance.now();
    const result = await api.applyEdit(e);
    const t1 = performance.now();
    setSnapshot(result.snapshot);
    setError(null);
    if (result.message) setNotice(result.message);
    if (import.meta.env.DEV) {
      // Development readout for the performance check: round trip, then until painted.
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          const t2 = performance.now();
          setTiming(`last edit: ${Math.round(t1 - t0)} ms round trip + ${Math.round(t2 - t1)} ms to draw, ${result.snapshot.rows.length} tasks`);
        }),
      );
    }
    return result;
  }, []);

  const startSplitDrag = (e: React.PointerEvent) => {
    const startX = e.clientX;
    const startWidth = gridWidth;
    const move = (ev: PointerEvent) => setGridWidth(Math.max(320, Math.min(window.innerWidth - 200, startWidth + ev.clientX - startX)));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  // Menu events are subscribed once; the ref always reaches the current edit function.
  const editRef = useRef(edit);
  editRef.current = edit;

  /** From another view: back to the task list with that row (1-based) selected. */
  // One question at a time, answered by ConfirmSheet.
  const [question, setQuestion] = useState<{ title: string; message: string; action: string; answer: (yes: boolean) => void } | null>(null);
  const confirm = useCallback(
    (title: string, message: string, action: string) => new Promise<boolean>((answer) => setQuestion({ title, message, action, answer })),
    [],
  );
  useEffect(() => {
    viewRef.current = view;
    if (view === "chat") setChatUnread(0);
  }, [view]);
  const pendingRow = useRef<number | null>(null);
  const showTask = useCallback((row: number) => {
    pendingRow.current = row - 1;
    setView("tasks");
  }, []);
  // Runs once the task list is mounted again.
  useEffect(() => {
    if (view === "tasks" && pendingRow.current !== null) {
      gridRef.current?.selectRow(pendingRow.current);
      pendingRow.current = null;
    }
  }, [view]);
  const textEditing = useCallback((editing: boolean) => {
    void api.setTextEditing(editing);
  }, []);

  useEffect(() => {
    api.getSnapshot().then(async (open) => {
      setSnapshot(open);
      // First run: open the sample and show the tour once (spec 3, item 11).
      if (!open && !firstRunDone()) {
        markFirstRunDone();
        try {
          setSnapshot(await api.openSample());
          setOverlay("tour");
        } catch (e) {
          setError(errorMessage(e));
        }
      }
    }, (e) => setError(errorMessage(e)));
    api.recentFiles().then(setRecent, () => {});
    team.status().then(setTeamStatus, () => {});
    // A window opened by an import collects its report when it loads.
    api.takeImportReport().then((r) => r && setReport(r), () => {});
    const subscriptions = [
      // Undo and Redo from the Edit menu run in Rust; the result arrives here.
      listen<Snapshot>("document-changed", (e) => {
        setSnapshot(e.payload);
        setError(null);
      }),
      listen<IpcError>("error", (e) => setError(e.payload.message)),
      listen<SyncStatus>("team-status", (e) => setSyncStatus(e.payload)),
      listen<string>("team-notice", (e) => setNotice(e.payload)),
      listen<{ kind: string; post?: { authorId: string; editedAt: string | null; deleted: boolean; reactions: unknown[] } }>("chat-event", (e) => {
        const p = e.payload.post;
        if (!p || p.editedAt || p.deleted || p.reactions.length || viewRef.current === "chat") return;
        void chat.me().then((me) => {
          if (p.authorId !== me) setChatUnread((n) => n + 1);
        });
      }),
      listen<ReportView>("import-report", (e) => {
        setReport(e.payload);
        void api.takeImportReport(); // shown now, so it is no longer waiting
      }),
      listen("document-closed", () => {
        setSnapshot(null);
        api.recentFiles().then(setRecent, () => {});
      }),
      listen<string>("menu", (e) => {
        // While a cell is being edited, the menu's Undo and Redo apply to the typing.
        if (e.payload === "edit.undo") document.execCommand("undo");
        if (e.payload === "edit.redo") document.execCommand("redo");
        if (e.payload === "help.contents") setOverlay("help");
        if (e.payload === "help.tour") setOverlay("tour");
        if (e.payload === "view.command_bar") setOverlay((o) => (o === "commands" ? null : "commands"));
        if (e.payload === "edit.find") setFinding(true);
        if (e.payload === "view.tasks") setView("tasks");
        if (e.payload === "view.resources") setView("resources");
        if (e.payload === "view.wbs") setView("wbs");
        if (e.payload === "view.workload") setView("workload");
        if (e.payload === "view.calendar") setView("calendar");
        if (e.payload === "view.board") setView("board");
        if (e.payload === "view.history") setView("history");
        if (e.payload === "view.chat") setView("chat");
        if (e.payload === "task.info") setInfoRow(gridRef.current?.activeRow() ?? null);
        if (e.payload === "task.indent") gridRef.current?.indent(false);
        if (e.payload === "task.outdent") gridRef.current?.indent(true);
        if (e.payload === "project.info") setOverlay("project");
        if (e.payload === "project.working_time") setOverlay("working");
        if (e.payload === "project.level") void editRef.current({ kind: "LevelResources" }).catch((err) => setError(errorMessage(err)));
        if (e.payload === "project.clear_leveling") void editRef.current({ kind: "ClearLeveling" }).catch((err) => setError(errorMessage(err)));
        if (e.payload === "project.set_baseline") void editRef.current({ kind: "SetBaseline" }).catch((err) => setError(errorMessage(err)));
        if (e.payload === "project.update") setOverlay("update");
        if (e.payload === "team.sign_in") setTeamSheet("signin");
        if (e.payload === "team.open") setTeamSheet("open");
        if (e.payload === "team.share") setTeamSheet("share");
        if (e.payload === "team.sign_out") signOut();
        if (e.payload === "team.add_account") setTeamSheet("account");
        if (e.payload === "team.join") setTeamSheet("join");
        if (e.payload === "team.invite") setTeamSheet("invite");
        if (e.payload === "team.members") setTeamSheet("members");
        if (e.payload === "team.move") setTeamSheet("connect");
        if (e.payload === "project.statistics") setOverlay("stats");
        const percent = /^task\.percent_(\d+)$/.exec(e.payload);
        if (percent && selectedRef.current.size)
          void editRef.current({ kind: "SetPercentComplete", ids: [...selectedRef.current], percent: Number(percent[1]) }).catch((err) => setError(errorMessage(err)));
        if (e.payload === "project.clear_baseline") void editRef.current({ kind: "ClearBaseline" }).catch((err) => setError(errorMessage(err)));
      }),
    ];
    return () => {
      subscriptions.forEach((s) => s.then((unlisten) => unlisten()));
    };
  }, []);

  const commands: CommandItem[] = [
    { name: "New Project…", shortcut: "⌘N", run: () => run(api.newProject) },
    { name: "Open…", shortcut: "⌘O", run: () => run(api.openProject) },
    { name: "Open Sample Project", run: () => run(api.openSample) },
    { name: "Import Microsoft Project XML…", shortcut: "⇧⌘I", run: () => run(api.importProject) },
    { name: "Show Help", shortcut: "⌘?", run: () => setOverlay("help") },
    { name: "Show Tour", run: () => setOverlay("tour") },
    { name: "Sign In to Team Server…", run: () => setTeamSheet("signin") },
    { name: "Open Team Project…", run: () => setTeamSheet("open") },
    { name: "Join a Team…", run: () => setTeamSheet("join") },
    { name: "Invite People to a Team…", run: () => setTeamSheet("invite") },
    { name: "Add Account to Team Server…", run: () => setTeamSheet("account") },
    { name: "Connect Teams to a Server… (put your teams online)", run: () => setTeamSheet("connect") },
  ];
  if (snapshot) {
    commands.push(
      { name: "Show Tasks", shortcut: "⌘1", run: () => setView("tasks") },
      { name: "Show Resource Sheet", shortcut: "⌘2", run: () => setView("resources") },
      { name: "Show Work Breakdown Structure (WBS)", shortcut: "⌘3", run: () => setView("wbs") },
      { name: "Show Workload (hours per person per day)", shortcut: "⌘4", run: () => setView("workload") },
      { name: "Show Calendar", shortcut: "⌘5", run: () => setView("calendar") },
      { name: "Show Board (Not started / In progress / Done)", shortcut: "⌘6", run: () => setView("board") },
      { name: "Show History (every change in a team project)", shortcut: "⌘7", run: () => setView("history") },
      { name: "Show Chat (teams, channels, posts)", shortcut: "⌘8", run: () => setView("chat") },
      { name: "Open Portfolio (all projects)", shortcut: "⌘0", run: () => void openPortfolio() },
      { name: "New Tab (open or create another project)", shortcut: "⌘T", run: () => void openNewTab() },
      { name: "Home (start page)", run: () => void openHome() },
      { name: "Add Task", run: () => gridRef.current?.addTaskBelow() },
      { name: "Find Task…", shortcut: "⌘F", run: () => setFinding(true) },
      { name: "Task Info…", shortcut: "⌘I", run: () => setInfoRow(gridRef.current?.activeRow() ?? null) },
      { name: "Project Info…", run: () => setOverlay("project") },
      { name: "Level Resources", run: () => void edit({ kind: "LevelResources" }).catch((e) => setError(errorMessage(e))) },
      { name: "Set Baseline", run: () => void edit({ kind: "SetBaseline" }).catch((e) => setError(errorMessage(e))) },
      { name: "Clear Baseline", run: () => void edit({ kind: "ClearBaseline" }).catch((e) => setError(errorMessage(e))) },
      { name: "Create Team Project…", run: () => setTeamSheet("share") },
      { name: "Members & Permissions…", run: () => setTeamSheet("members") },
      { name: "Project Statistics… (earned value)", run: () => setOverlay("stats") },
      { name: "Update Project… (progress up to a day)", run: () => setOverlay("update") },
      ...[0, 25, 50, 75, 100].map((n) => ({
        name: `Mark Selected Tasks ${n}% Complete`,
        run: () => void edit({ kind: "SetPercentComplete", ids: [...selectedIds], percent: n }).catch((e) => setError(errorMessage(e))),
      })),
      { name: "Clear Leveling", run: () => void edit({ kind: "ClearLeveling" }).catch((e) => setError(errorMessage(e))) },
      { name: "Working Time… (calendar and holidays)", run: () => setOverlay("working") },
      { name: "Indent Selected Tasks", shortcut: "Tab", run: () => gridRef.current?.indent(false) },
      { name: "Outdent Selected Tasks", shortcut: "⇧Tab", run: () => gridRef.current?.indent(true) },
      { name: "Delete Selected Tasks", shortcut: "⌫", run: () => gridRef.current?.deleteSelection() },
      { name: snapshot.undoLabel ? `Undo ${snapshot.undoLabel}` : "Undo", shortcut: "⌘Z", run: () => run(api.undo) },
      { name: snapshot.redoLabel ? `Redo ${snapshot.redoLabel}` : "Redo", shortcut: "⇧⌘Z", run: () => run(api.redo) },
      { name: "Zoom In", run: () => ganttRef.current?.zoom(1.5) },
      { name: "Zoom Out", run: () => ganttRef.current?.zoom(1 / 1.5) },
      { name: "Export as Microsoft Project XML…", shortcut: "⇧⌘E", run: () => void api.exportProject().catch((e) => setError(errorMessage(e))) },
    );
  }
  const overlays = (
    <>
      {overlay === "help" && <HelpSheet onClose={() => setOverlay(null)} onTour={() => setOverlay(snapshot ? "tour" : null)} />}
      {overlay === "tour" && snapshot && <Tour onDone={() => setOverlay(null)} />}
      {overlay === "commands" && <CommandBar commands={commands} onClose={() => setOverlay(null)} />}
      {overlay === "project" && snapshot && <ProjectInfo snapshot={snapshot} send={edit} onClose={() => setOverlay(null)} />}
      {teamSheet === "signin" && (
        <SignInSheet
          status={teamStatus}
          onClose={() => setTeamSheet(null)}
          onSignedIn={(st) => {
            setTeamStatus(st);
            setNotice(`Signed in to ${st.server} as ${st.user?.name}.`);
          }}
        />
      )}
      {teamSheet === "account" && <AddAccountSheet status={teamStatus} onClose={() => setTeamSheet(null)} onAdded={setNotice} />}
      {teamSheet === "open" && <OpenSharedSheet onClose={() => setTeamSheet(null)} onOpened={(snap) => snap && setSnapshot(snap)} onSignIn={() => setTeamSheet("join")} />}
      {teamSheet === "join" && (
        <JoinSheet
          onClose={() => closeTeamSheet("join")}
          onJoined={(st) => {
            setTeamStatus(st);
            setTeamSheet("open");
          }}
        />
      )}
      {teamSheet === "invite" && (
        <InvitePeopleSheet
          onClose={() => closeTeamSheet("invite")}
          onInvited={(list) => {
            setInvites(list);
            setTeamSheet("invites");
          }}
        />
      )}
      {teamSheet === "connect" && (
        <ConnectSheet
          status={teamStatus}
          onClose={() => closeTeamSheet("connect")}
          onMoved={(r) => {
            setTeamStatus(r.status);
            setInvites(r.invites);
            setNotice(`${r.teams.join(", ")} moved to ${r.status.server}. Send the new invites below; your copies already carry on there.`);
            setTeamSheet("invites");
          }}
        />
      )}
      {teamSheet === "members" && snapshot && <MembersSheet snapshot={snapshot} onClose={() => closeTeamSheet("members")} />}
      {teamSheet === "invites" && <InvitesSheet invites={invites} hostedHere={teamStatus?.hostedHere ?? false} onClose={() => closeTeamSheet("invites")} />}
      {teamSheet === "share" && snapshot && (
        <CreateTeamSheet
          snapshot={snapshot}
          status={teamStatus}
          onClose={() => closeTeamSheet("share")}
          onCreated={(result) => {
            setSnapshot(result.snapshot);
            setInvites(result.invites);
            team.status().then(setTeamStatus, () => {});
            setTeamSheet("invites");
          }}
        />
      )}
      {question && (
        <ConfirmSheet
          title={question.title}
          message={question.message}
          action={question.action}
          onAnswer={(yes) => {
            setQuestion(null);
            question.answer(yes);
          }}
        />
      )}
      {overlay === "stats" && snapshot && <ProjectStatistics snapshot={snapshot} onClose={() => setOverlay(null)} />}
      {overlay === "update" && snapshot && <UpdateProject snapshot={snapshot} send={edit} onClose={() => setOverlay(null)} />}
      {overlay === "working" && snapshot && <WorkingTime snapshot={snapshot} send={edit} onClose={() => setOverlay(null)} />}
      {infoRow && <TaskInfo key={infoRow.id} row={infoRow} send={edit} onClose={() => setInfoRow(null)} />}
    </>
  );

  if (!snapshot) {
    // The start screen always offers the next step (spec 5.1.5).
    return (
      <main className="empty">
        <TabStrip />
        {overlays}
        <h1>Super PM</h1>
        <p>Plan a project with tasks, links and a calendar. Every change is saved as you go.</p>
        <div className="start-actions">
          <button className="primary" onClick={() => run(api.newProject)}>
            New Project…
          </button>
          <button onClick={() => run(api.openProject)}>Open…</button>
          <button onClick={() => run(api.openSample)}>Open Sample Project</button>
          <button onClick={() => void openPortfolio()}>Portfolio</button>
          <button onClick={() => setTeamSheet(teamStatus?.user ? "open" : "join")}>{teamStatus?.user ? "Open Team Project…" : "Join a Team…"}</button>
        </div>
        {recent.length > 0 && (
          <nav className="recent" aria-label="Recent projects">
            <h2>Recent</h2>
            <ul>
              {recent.map((f) => (
                <li key={f.path}>
                  <button className="link" title={f.path} onClick={() => run(() => api.openRecent(f.path))}>
                    {f.name}
                  </button>
                </li>
              ))}
            </ul>
          </nav>
        )}
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
      </main>
    );
  }

  return (
    <main className="document">
      <TabStrip refreshKey={snapshot.project.name} />
      <header className="toolbar">
        <button className="home-button" onClick={() => void openHome()} title="Home: the start page, to open or create projects" aria-label="Home">
          ⌂
        </button>
        <div>
          <h1>{snapshot.project.name}</h1>
          <p className="secondary">
            {formatDate(snapshot.project.start)} – {formatDate(snapshot.project.finish)} · {snapshot.rows.length} tasks
            {snapshot.project.costCents !== 0 && ` · ${formatMoney(snapshot.project.costCents, snapshot.project.currency)}`}
          </p>
        </div>
        {snapshot.project.team && <SyncPill status={syncStatus} pending={snapshot.project.team.pending} />}
        {(() => {
          const label = accessLabel(snapshot);
          return (
            label && (
              <span className="access-pill" title={label.detail} role="note">
                {label.text}
              </span>
            )
          );
        })()}
        <div className="segmented" role="tablist" aria-label="View">
          <button role="tab" aria-selected={view === "tasks"} className={view === "tasks" ? "on" : undefined} onClick={() => setView("tasks")}>
            Tasks
          </button>
          <button role="tab" aria-selected={view === "resources"} className={view === "resources" ? "on" : undefined} onClick={() => setView("resources")}>
            Resources
            {snapshot.resources.some((r) => r.overloads.length > 0) && (
              <span className="overload" aria-label="(some over-allocated)">
                {" "}⚠
              </span>
            )}
          </button>
          <button role="tab" aria-selected={view === "wbs"} className={view === "wbs" ? "on" : undefined} onClick={() => setView("wbs")}>
            WBS
          </button>
          <button role="tab" aria-selected={view === "workload"} className={view === "workload" ? "on" : undefined} onClick={() => setView("workload")}>
            Workload
          </button>
          <button role="tab" aria-selected={view === "calendar"} className={view === "calendar" ? "on" : undefined} onClick={() => setView("calendar")}>
            Calendar
          </button>
          <button role="tab" aria-selected={view === "board"} className={view === "board" ? "on" : undefined} onClick={() => setView("board")}>
            Board
          </button>
          {snapshot.project.team && (
            <button role="tab" aria-selected={view === "history"} className={view === "history" ? "on" : undefined} onClick={() => setView("history")}>
              History
            </button>
          )}
          {teamStatus?.user && (
            <button role="tab" aria-selected={view === "chat"} className={view === "chat" ? "on" : undefined} onClick={() => setView("chat")}>
              Chat
              {chatUnread > 0 && view !== "chat" && (
                <span className="tab-badge" aria-label={`${chatUnread} new`}>
                  {chatUnread}
                </span>
              )}
            </button>
          )}
        </div>
      </header>
      {error && (
        <p role="alert" className="banner error">
          {error}
        </p>
      )}
      {notice && (
        <div role="status" className="banner info">
          <span>{notice}</span>
          <button className="link" onClick={() => setNotice(null)} aria-label="Dismiss">
            OK
          </button>
        </div>
      )}
      {snapshot.problem && (
        <p role="alert" className="banner error">
          {snapshot.problem.message}
        </p>
      )}
      {snapshot.conflicts.map((c) => (
        <p key={c.taskId} role="status" className="banner warning">
          {snapshot.rows.find((r) => r.id === c.taskId)?.name}: {c.message}
        </p>
      ))}
      {view === "chat" ? (
        <ChatView snapshot={snapshot} onShowTask={showTask} onConfirm={confirm} />
      ) : view === "history" && snapshot.project.team ? (
        <HistoryView snapshot={snapshot} onShowTask={showTask} onConfirm={confirm} />
      ) : view === "board" ? (
        <Board snapshot={snapshot} onEdit={edit} onShowTask={showTask} />
      ) : view === "calendar" ? (
        <CalendarView snapshot={snapshot} onShowTask={showTask} />
      ) : view === "workload" ? (
        <Workload snapshot={snapshot} onShowTask={showTask} />
      ) : view === "wbs" ? (
        <WbsChart snapshot={snapshot} onEdit={edit} onTextEditing={textEditing} />
      ) : view === "resources" ? (
        <ResourceSheet snapshot={snapshot} onEdit={edit} onTextEditing={textEditing} />
      ) : (
        <>
      {finding && <FindBar onFind={(t, fromCurrent) => gridRef.current?.findNext(t, fromCurrent) ?? false} onClose={() => setFinding(false)} />}
        <div className="workspace">
          <div className="pane" style={{ width: gridWidth }}>
            <Grid
              ref={gridRef}
              snapshot={snapshot}
              onEdit={edit}
              onTextEditing={textEditing}
              onScrollTop={setScrollTop}
              onSelectionChange={setSelectedIds}
              onBlocked={setNotice}
            />
          </div>
          <div
            className="splitter"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize task list"
            onPointerDown={startSplitDrag}
          />
          <Gantt
            onConfirm={confirm}
            snapshot={snapshot}
            scrollTop={scrollTop}
            selectedIds={selectedIds}
            onEdit={edit}
            onSelectRow={(r) => gridRef.current?.selectRow(r)}
            onScrollBy={(dy) => gridRef.current?.scrollBy(dy)}
            onMessage={setMessage}
            ref={ganttRef}
          />
        </div>
        </>
      )}
      {report && <ImportReport report={report} onClose={() => setReport(null)} />}
      {overlays}
      <footer className="status" role="status" aria-live="polite" data-tour="status">
        {message ?? (import.meta.env.DEV ? timing : null) ?? "\u00a0"}
      </footer>
    </main>
  );
}

// The first-run flag is a per-machine convenience, so browser storage is enough; if it is
// unavailable the tour simply shows again.
const FIRST_RUN_KEY = "superpm.firstRunDone";
function firstRunDone(): boolean {
  try {
    return localStorage.getItem(FIRST_RUN_KEY) === "1";
  } catch {
    return false;
  }
}
function markFirstRunDone() {
  try {
    localStorage.setItem(FIRST_RUN_KEY, "1");
  } catch {
    // ignore
  }
}
