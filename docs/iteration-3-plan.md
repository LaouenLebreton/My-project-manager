# Iteration 3: team server, live collaboration, chat

Owner decisions (2026-10-09):

1. **Team server first, able to scale to a cloud provider.** One program a team runs on its own
   machine; the same program later runs on a cloud provider with a managed database and several
   instances behind a load balancer.
2. **Several people edit at the same time.** Every change is logged and shown in its own view.
   The project owner limits what each member may change.
3. **Chat modelled on Microsoft Teams:** teams, channels, posts (with replies).

The app stays local-first: everything in iterations 1–2 keeps working without a server, and a
shared project keeps working offline, catching up when the connection returns.

## Architecture

```
Super PM app (Tauri)                         pm-server (Rust, axum)
  pm-core engine                               pm-core engine (same code)
  local .pmproj (SQLite) ── HTTPS + WebSocket ──  Storage trait
  outbox of unsent changes                       ├─ SQLite   (team server, default)
                                                 └─ Postgres (cloud, later step)
```

- **One ordering authority per project: the server.** The engine already expresses every edit as
  a validated command with an inverse. A client sends `(base version, command)`; the server
  applies commands to a project one at a time, re-validates them against the latest state (and
  the sender's permissions), gives each accepted change the next version number, stores it in
  the change log and broadcasts it to everyone connected.
- **Clients apply their own edits at once** (optimistic). When a change from someone else
  arrives, the client undoes its unconfirmed edits, applies the remote change, and re-applies
  its own on top (rebase). An edit that no longer fits (its task was deleted, a link would now
  form a loop) is dropped and the user is told in plain words.
- **Offline:** unsent changes wait in an outbox inside the `.pmproj` and go out on reconnect,
  through the same rebase.
- **Storage behind a trait** so the server can use SQLite (one file, nothing to install) or
  Postgres. The server keeps no state that matters between requests other than open
  WebSockets, so several instances can run behind a load balancer; instances tell each other
  about new versions through the database (Postgres `LISTEN/NOTIFY`).
- The server uses rusqlite (as `pm-store` does) rather than sqlx: two SQLite bindings in one
  workspace would link SQLite twice, which Cargo refuses. Postgres comes via `tokio-postgres`.

## People, roles and permissions

- **Accounts** live on the server: name, email, password (Argon2id). Sessions are bearer tokens.
  Single sign-on (Microsoft Entra ID, Google) is added for the cloud step.
- **Teams** group people (Microsoft Teams model). A project is shared with a team; each shared
  project gets a channel in that team automatically.
- **Project roles:** Owner, Editor, Viewer, set per member by the owner. On top of the role, the
  owner can limit an Editor:
  - what they may change: schedule (tasks, durations, links, constraints), progress (% done,
    actual dates), resources and assignments, costs, project settings (calendar, baseline);
  - which tasks: all tasks, or only tasks assigned to them (as a resource), or only inside
    chosen phases.
  The server enforces this on every command; the app greys out what a member cannot change and
  explains why.

## Change log view

A new view listing every accepted change, newest first: who, when, what in plain words
("Alex changed the duration of Survey from 3 days to 5 days"), and the version. Filters by
person, by task and by date. Selecting a task-related change shows the task. The owner can
revert a change (applied as a new change, so the log stays complete).

## Chat (Teams, channels, posts)

- **Team:** members, a name, channels. A **General** channel always exists.
- **Channel:** a stream of **posts**; each post can have **replies** (threads), as in Teams.
- Posts support @mentions of people, links to tasks (`#12` or by name, shown as chips that open
  the task), and reactions.
- Unread counts per channel; notifications for mentions.
- Shown in a Chat pane in the app, and a project's own channel is one click away from the
  project window.

## Steps

1. **[Done]** **Server skeleton:** `crates/pm-server` (axum 0.8, tokio 1.53), Storage trait with SQLite,
   accounts, sign-in, tokens. Integration tests over HTTP. `super-pm-server` binary with a
   config file.
2. **[Done]** **Sharing a project:** upload a local project to the server, list and download shared
   projects, members and roles.
3. **[Done]** **Live sync:** WebSocket per open project, server ordering, client rebase, outbox, reconnect.
   Property test: any interleaving of edits from several clients ends with every client equal
   to the server. In the app: Team menu (Sign In, Open Shared Project, Share Project with
   Team, Sign Out), a sync indicator in the header, notices when an edit is refused or no
   longer fits. Shared projects are kept in the app's Shared folder.
4. **[Done]** **Change log view** and owner revert.
5. **[Done]** **Permissions** per member, enforced on the server, reflected in the app. Team > Members & Permissions
   for the owner; refused edits come back as a notice saying what is not allowed. The app also
   greys out what a person may not change and says why (View only / Limited editing).
6. **[Done]** **Teams, channels, posts:** server API and storage, then the Chat pane. Chat tab (⌘8): teams and channels on the
   left (General, one per team project, and any added), conversations with threaded replies,
   live delivery, unread counts, @mentions highlighted, #12 linking to task 12. Since then:
   reactions (👍 ❤️ 😄 🎉 👀 ✅), editing and deleting posts (authors; team owners may delete
   any), conversations that mention you highlighted, and a new-post count on the Chat tab.
7. **Online (in progress):** *Done:* team owners invite through the server; optional open
   sign-up (`--open-signup`); `--public-url` for invites; Dockerfile and `docs/deploy.md`
   (Fly.io recommended, or any Linux server with Caddy); the app's sign-in offers "create my
   account". *To do:* move a team hosted on a Mac to an online server ("connect it to a
   server afterwards"); invite emails sent by the server (needs an email provider);
   session token in the Keychain; Postgres storage and several server instances; single
   sign-on. Needs the owner's hosting and email accounts to go live.
