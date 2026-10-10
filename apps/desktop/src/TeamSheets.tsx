import { useEffect, useState } from "react";
import { errorMessage, team, type Invite, type SharedProject, type Snapshot, type TeamProject, type TeamStatus, type TeamView } from "./api";
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
    <Sheet title="Sign In to Team Server" onClose={onClose} onSave={save} action="Sign In">
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
    <Sheet title="Add Account to Server" onClose={onClose} onSave={save} action="Add Account">
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

const splitEmails = (text: string) => text.split(/[\s,;]+/).filter(Boolean);

/** Team > Create Team Project: a team name and people's emails. No server to set up: the app
 * hosts the team; the first time it asks who you are. */
export function CreateTeamSheet({
  snapshot,
  status,
  onClose,
  onCreated,
}: {
  snapshot: Snapshot;
  status: TeamStatus | null;
  onClose: () => void;
  onCreated: (result: TeamProject) => void;
}) {
  const known = !!status?.user;
  const [teams, setTeams] = useState<TeamView[]>([]);
  const [choice, setChoice] = useState<string>("new");
  const [newName, setNewName] = useState(snapshot.project.name);
  const [myName, setMyName] = useState("");
  const [myEmail, setMyEmail] = useState("");
  const [emails, setEmails] = useState("");
  useEffect(() => {
    if (!known) return;
    team.teams().then(
      (t) => {
        setTeams(t);
        if (t.length) setChoice(t[0].id);
      },
      () => {},
    );
  }, [known]);
  if (snapshot.project.team) {
    return (
      <Sheet title="Create Team Project" onClose={onClose}>
        <p>This is already a team project. Changes sync with your team as you work. To add people, choose Team &gt; Invite People.</p>
      </Sheet>
    );
  }
  const save = async () => {
    try {
      onCreated(await team.createProject(known ? null : myName, known ? null : myEmail, choice === "new" ? null : choice, newName, splitEmails(emails)));
      return null;
    } catch (e) {
      return errorMessage(e);
    }
  };
  return (
    <Sheet title="Create Team Project" onClose={onClose} onSave={save} action="Create Team">
      <p className="secondary small">
        Everyone in the team can open this project and change it at the same time; every change is logged, and as the owner you can limit what each person
        may change.
      </p>
      <div className="form">
        {!known && (
          <>
            <label htmlFor="ct-name">Your name</label>
            <input id="ct-name" className="wide" value={myName} onChange={(e) => setMyName(e.target.value)} autoFocus required />
            <label htmlFor="ct-email">Your email</label>
            <div>
              <input id="ct-email" className="wide" type="email" value={myEmail} onChange={(e) => setMyEmail(e.target.value)} required />
              <p className="secondary small">So your team sees who changed what. Asked once.</p>
            </div>
          </>
        )}
        <span className="form-label">Team</span>
        <div role="radiogroup" aria-label="Team">
          {teams.map((t) => (
            <label key={t.id} className="inline block">
              <input type="radio" name="team" checked={choice === t.id} onChange={() => setChoice(t.id)} /> {t.name}
            </label>
          ))}
          <label className="inline block">
            {teams.length > 0 && (
              <>
                <input type="radio" name="team" checked={choice === "new"} onChange={() => setChoice("new")} /> New team:{" "}
              </>
            )}
            <input aria-label="Team name" value={newName} onChange={(e) => setNewName(e.target.value)} disabled={choice !== "new"} required={choice === "new"} />
          </label>
        </div>
        <label htmlFor="ct-emails">People</label>
        <div>
          <textarea id="ct-emails" rows={3} value={emails} onChange={(e) => setEmails(e.target.value)} placeholder="alex@example.com, sam@example.com" />
          <p className="secondary small">Their email addresses. You get an invite to send each of them. You can add more people later.</p>
        </div>
      </div>
    </Sheet>
  );
}

/** After creating a team or inviting people: the invites to send. */
export function InvitesSheet({ invites, hostedHere, onClose }: { invites: Invite[]; hostedHere: boolean; onClose: () => void }) {
  const [copied, setCopied] = useState<string | null>(null);
  const copy = (i: Invite) => {
    if (!i.invite) return;
    void navigator.clipboard.writeText(i.invite).then(() => setCopied(i.email));
  };
  const toSend = invites.filter((i) => i.invite);
  return (
    <Sheet title={invites.length ? "Invite your team" : "Team project created"} onClose={onClose}>
      {invites.length === 0 && <p>The project is now a team project. Add people any time with Team &gt; Invite People.</p>}
      {toSend.length > 0 && (
        <p>
          Send each person their invite (Messages, Mail, anything). They open Super PM, choose <strong>Team &gt; Join a Team</strong> and paste it.
        </p>
      )}
      <ul className="shared-list">
        {invites.map((i) => (
          <li key={i.email}>
            <span>
              <strong>{i.email}</strong>
              {i.note && <span className="secondary small"> · {i.note}</span>}
            </span>
            {i.invite && (
              <button type="button" onClick={() => copy(i)}>
                {copied === i.email ? "Copied ✓" : "Copy Invite"}
              </button>
            )}
          </li>
        ))}
      </ul>
      {hostedHere && toSend.length > 0 && (
        <p className="secondary small">
          For now your team lives on this Mac: keep Super PM open so the others can reach it, and they need to be on the same network (office or home
          Wi-Fi). Connecting the team to a server later makes it work from anywhere.
        </p>
      )}
    </Sheet>
  );
}

/** Team > Join a Team: paste the invite you were sent. */
export function JoinSheet({ onClose, onJoined }: { onClose: () => void; onJoined: (s: TeamStatus) => void }) {
  const [invite, setInvite] = useState("");
  const save = async () => {
    try {
      onJoined(await team.join(invite));
      return null;
    } catch (e) {
      return errorMessage(e);
    }
  };
  return (
    <Sheet title="Join a Team" onClose={onClose} onSave={save} action="Join">
      <div className="form">
        <label htmlFor="jt-invite">Invite</label>
        <div>
          <textarea id="jt-invite" rows={3} value={invite} onChange={(e) => setInvite(e.target.value)} placeholder="SuperPM-Invite|…" autoFocus required />
          <p className="secondary small">Paste the invite a teammate sent you. Then open the team's projects with Team &gt; Open Team Project.</p>
        </div>
      </div>
    </Sheet>
  );
}

/** Team > Invite People: more people for a team you own. */
export function InvitePeopleSheet({ onClose, onInvited }: { onClose: () => void; onInvited: (invites: Invite[]) => void }) {
  const [teams, setTeams] = useState<TeamView[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [choice, setChoice] = useState("");
  const [emails, setEmails] = useState("");
  useEffect(() => {
    team.teams().then(
      (t) => {
        const mine = t.filter((x) => x.role === "owner");
        setTeams(mine);
        if (mine.length) setChoice(mine[0].id);
      },
      (e) => setProblem(errorMessage(e)),
    );
  }, []);
  if (problem || teams?.length === 0) {
    return (
      <Sheet title="Invite People" onClose={onClose}>
        <p>{problem ?? "You do not own a team yet. Open a project and choose Team > Create Team Project."}</p>
      </Sheet>
    );
  }
  const save = async () => {
    try {
      onInvited(await team.invite(choice, splitEmails(emails)));
      return null;
    } catch (e) {
      return errorMessage(e);
    }
  };
  return (
    <Sheet title="Invite People" onClose={onClose} onSave={save} action="Invite">
      <div className="form">
        <label htmlFor="ip-team">Team</label>
        <select id="ip-team" value={choice} onChange={(e) => setChoice(e.target.value)}>
          {(teams ?? []).map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
        <label htmlFor="ip-emails">People</label>
        <div>
          <textarea id="ip-emails" rows={3} value={emails} onChange={(e) => setEmails(e.target.value)} placeholder="alex@example.com, sam@example.com" autoFocus required />
          <p className="secondary small">Their email addresses. You get an invite to send each of them.</p>
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
    <Sheet title="Open Team Project" onClose={onClose}>
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
          Join a Team…
        </button>
      )}
      {projects?.length === 0 && <p>Your teams have no projects yet. Open a project and choose Team &gt; Create Team Project.</p>}
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
