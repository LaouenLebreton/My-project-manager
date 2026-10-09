import { useEffect, useState } from "react";
import { errorMessage, team, type SharedProject, type Snapshot, type TeamStatus, type TeamView } from "./api";
import type { SyncStatus } from "./bindings/SyncStatus";
import { formatDateTime } from "./format";
import { Sheet } from "./Sheets";

/** Team > Sign In to Team Server: an existing account, or setting up a brand-new server. */
export function SignInSheet({ status, onClose, onSignedIn }: { status: TeamStatus | null; onClose: () => void; onSignedIn: (s: TeamStatus) => void }) {
  const [server, setServer] = useState(status?.server ?? "");
  const [email, setEmail] = useState(status?.user?.email ?? "");
  const [password, setPassword] = useState("");
  const [setup, setSetup] = useState(false);
  const [name, setName] = useState("");
  const save = async () => {
    try {
      onSignedIn(await team.signIn(server, email, password, setup ? name : null));
      return null;
    } catch (e) {
      return errorMessage(e);
    }
  };
  return (
    <Sheet title="Sign In to Team Server" onClose={onClose} onSave={save}>
      {status?.user && (
        <p className="secondary small">
          Signed in as {status.user.name} on {status.server}. Signing in again replaces this account.
        </p>
      )}
      <div className="form">
        <label htmlFor="ts-server">Server</label>
        <div>
          <input id="ts-server" className="wide" value={server} onChange={(e) => setServer(e.target.value)} placeholder="pm.example.com" autoFocus required />
          <p className="secondary small">The address your team's Super PM server runs at. Ask whoever set it up.</p>
        </div>
        <label htmlFor="ts-email">Email</label>
        <input id="ts-email" type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="username" required />
        <label htmlFor="ts-password">Password</label>
        <input id="ts-password" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete={setup ? "new-password" : "current-password"} required />
        <span className="form-label" />
        <label className="inline">
          <input type="checkbox" checked={setup} onChange={(e) => setSetup(e.target.checked)} /> This is a new server: set it up and make me its administrator
        </label>
        {setup && (
          <>
            <label htmlFor="ts-name">Your name</label>
            <div>
              <input id="ts-name" className="wide" value={name} onChange={(e) => setName(e.target.value)} required />
              <p className="secondary small">Only works on a server nobody has set up yet. Choose a password of at least 10 characters.</p>
            </div>
          </>
        )}
      </div>
    </Sheet>
  );
}

/** Team > Add Account to Server (administrators): someone else's sign-in. */
export function AddAccountSheet({ status, onClose, onAdded }: { status: TeamStatus | null; onClose: () => void; onAdded: (message: string) => void }) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  if (!status?.user?.isAdmin) {
    return (
      <Sheet title="Add Account to Server" onClose={onClose}>
        <p>{status?.user ? `Only the administrator of ${status.server} can add accounts. Ask them to add the person.` : "Sign in to the team server as its administrator first."}</p>
      </Sheet>
    );
  }
  const save = async () => {
    try {
      const user = await team.createAccount(name, email, password);
      onAdded(`${user.name} (${user.email}) can now sign in to ${status.server}. Give them the server address and the password you chose; they can be added to teams by email.`);
      return null;
    } catch (e) {
      return errorMessage(e);
    }
  };
  return (
    <Sheet title="Add Account to Server" onClose={onClose} onSave={save}>
      <p className="secondary small">A new person who can sign in to {status.server}. Then add them to a team when you share a project.</p>
      <div className="form">
        <label htmlFor="aa-name">Name</label>
        <input id="aa-name" className="wide" value={name} onChange={(e) => setName(e.target.value)} autoFocus required />
        <label htmlFor="aa-email">Email</label>
        <input id="aa-email" className="wide" type="email" value={email} onChange={(e) => setEmail(e.target.value)} required />
        <label htmlFor="aa-password">Password</label>
        <div>
          <input id="aa-password" className="wide" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" required />
          <p className="secondary small">At least 10 characters. Tell them, so they can sign in.</p>
        </div>
      </div>
    </Sheet>
  );
}

/** Team > Share Project with Team: pick or create a team, optionally add people, share. */
export function ShareSheet({ snapshot, onClose, onShared, onSignIn }: { snapshot: Snapshot; onClose: () => void; onShared: (s: Snapshot) => void; onSignIn: () => void }) {
  const [teams, setTeams] = useState<TeamView[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [choice, setChoice] = useState<string>("new");
  const [newName, setNewName] = useState(snapshot.project.name);
  const [emails, setEmails] = useState("");
  useEffect(() => {
    team.teams().then(
      (t) => {
        setTeams(t);
        if (t.length) setChoice(t[0].id);
      },
      (e) => setProblem(errorMessage(e)),
    );
  }, []);
  if (snapshot.project.team) {
    return (
      <Sheet title="Share Project with Team" onClose={onClose}>
        <p>This project is already shared through {snapshot.project.team.server}. Changes sync with your team as you work.</p>
      </Sheet>
    );
  }
  const save = async () => {
    try {
      const teamId = choice === "new" ? (await team.createTeam(newName)).id : choice;
      const failed: string[] = [];
      for (const email of emails.split(/[\s,;]+/).filter(Boolean)) {
        try {
          await team.addMember(teamId, email);
        } catch (e) {
          failed.push(`${email}: ${errorMessage(e)}`);
        }
      }
      onShared(await team.share(teamId));
      return failed.length ? `Shared. These people could not be added: ${failed.join("; ")}` : null;
    } catch (e) {
      return errorMessage(e);
    }
  };
  if (problem && teams === null) {
    return (
      <Sheet title="Share Project with Team" onClose={onClose}>
        <p role="alert">{problem}</p>
        <button
          type="button"
          onClick={() => {
            onClose();
            onSignIn();
          }}
        >
          Sign In…
        </button>
      </Sheet>
    );
  }
  return (
    <Sheet title="Share Project with Team" onClose={onClose} onSave={save}>
      <p className="secondary small">Everyone in the team can open the project and change it at the same time; every change is logged. As the owner you can limit what each person may change.</p>
      <div className="form">
        <span className="form-label">Team</span>
        <div role="radiogroup" aria-label="Team">
          {(teams ?? []).map((t) => (
            <label key={t.id} className="inline block">
              <input type="radio" name="team" checked={choice === t.id} onChange={() => setChoice(t.id)} /> {t.name}
            </label>
          ))}
          <label className="inline block">
            <input type="radio" name="team" checked={choice === "new"} onChange={() => setChoice("new")} /> New team:{" "}
            <input aria-label="New team name" value={newName} onChange={(e) => setNewName(e.target.value)} disabled={choice !== "new"} />
          </label>
        </div>
        <label htmlFor="sh-emails">Add people</label>
        <div>
          <textarea id="sh-emails" rows={3} value={emails} onChange={(e) => setEmails(e.target.value)} placeholder="alex@example.com, sam@example.com" />
          <p className="secondary small">Their email addresses, if they are not in the team yet. They need an account on the server (the administrator creates accounts).</p>
        </div>
      </div>
    </Sheet>
  );
}

/** Team > Open Shared Project: every project shared with your teams. */
export function OpenSharedSheet({ onClose, onOpened, onSignIn }: { onClose: () => void; onOpened: (s: Snapshot | null) => void; onSignIn: () => void }) {
  const [projects, setProjects] = useState<SharedProject[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  useEffect(() => {
    team.projects().then(setProjects, (e) => setProblem(errorMessage(e)));
  }, []);
  const open = async (p: SharedProject) => {
    setOpening(p.id);
    setProblem(null);
    try {
      onOpened(await team.open(p.id));
      onClose();
    } catch (e) {
      setProblem(errorMessage(e));
    } finally {
      setOpening(null);
    }
  };
  return (
    <Sheet title="Open Shared Project" onClose={onClose}>
      {problem && (
        <p role="alert" className="error">
          {problem}
        </p>
      )}
      {projects === null && !problem && <p className="secondary">Loading…</p>}
      {projects === null && problem && (
        <button
          type="button"
          onClick={() => {
            onClose();
            onSignIn();
          }}
        >
          Sign In…
        </button>
      )}
      {projects?.length === 0 && <p>No projects are shared with your teams yet. Open a project and choose Team &gt; Share Project with Team.</p>}
      {projects && projects.length > 0 && (
        <ul className="shared-list">
          {projects.map((p) => (
            <li key={p.id}>
              <span>
                <strong>{p.name}</strong>
                <span className="secondary small"> · changed {formatDateTime(p.updatedAt.slice(0, 19))}</span>
              </span>
              <button type="button" onClick={() => void open(p)} disabled={opening !== null}>
                {opening === p.id ? "Opening…" : "Open"}
              </button>
            </li>
          ))}
        </ul>
      )}
    </Sheet>
  );
}

/** The header's sync indicator for a shared project. */
export function SyncPill({ status, pending }: { status: SyncStatus | null; pending: number }) {
  const state = status?.state ?? "connecting";
  const unsent = status?.pending ?? pending;
  const text =
    state === "online"
      ? unsent
        ? `Sending ${unsent}…`
        : "Synced"
      : state === "connecting"
        ? "Connecting…"
        : state === "offline"
          ? `Offline${unsent ? ` · ${unsent} unsent` : ""}`
          : "Not syncing";
  return (
    <span className={`sync-pill ${state}`} role="status" title={status?.message ?? text}>
      <span className="dot" aria-hidden /> {text}
    </span>
  );
}
