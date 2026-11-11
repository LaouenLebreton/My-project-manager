import { useEffect, useState } from "react";
import { errorMessage, team, type Member, type MemberLimits, type Snapshot } from "./api";
import { Sheet } from "./Sheets";

const KINDS: [keyof Omit<MemberLimits, "tasks">, string][] = [
  ["schedule", "Schedule (tasks, durations, links)"],
  ["progress", "Progress (% done, actual dates)"],
  ["resources", "Resources and assignments"],
  ["costs", "Costs"],
  ["project", "Project settings (calendar, baseline, leveling)"],
];

/** Team > Members & Permissions: the owner decides what each person may change. */
export function MembersSheet({ snapshot, onClose }: { snapshot: Snapshot; onClose: () => void }) {
  const [members, setMembers] = useState<Member[] | null>(null);
  const [original, setOriginal] = useState<string>("");
  const [problem, setProblem] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [removeError, setRemoveError] = useState<string | null>(null);
  const shared = !!snapshot.project.team;
  useEffect(() => {
    if (!shared) return;
    team.members().then(
      (m) => {
        setMembers(m);
        setOriginal(JSON.stringify(m));
      },
      (e) => setProblem(errorMessage(e)),
    );
  }, [shared]);
  const phases = snapshot.rows.filter((r) => r.isSummary && r.depth === 0);
  const iOwn = members?.some((m) => m.isMe && m.role === "owner") ?? false;

  const update = (userId: string, f: (m: Member) => Member) => setMembers((list) => list && list.map((m) => (m.userId === userId ? f(m) : m)));
  const setLimit = (m: Member, key: keyof Omit<MemberLimits, "tasks">, on: boolean) => update(m.userId, (x) => ({ ...x, limits: { ...x.limits, [key]: on } }));

  const remove = async (m: Member) => {
    try {
      await team.removeMember(m.userId);
      setMembers((list) => list && list.filter((x) => x.userId !== m.userId));
      setOriginal((o) => JSON.stringify((JSON.parse(o) as Member[]).filter((x) => x.userId !== m.userId)));
      setRemoveError(null);
    } catch (e) {
      setRemoveError(errorMessage(e));
    }
    setRemoving(null);
  };

  const save = async () => {
    if (!members) return null;
    const before: Member[] = JSON.parse(original);
    try {
      for (const m of members) {
        const was = before.find((b) => b.userId === m.userId);
        if (m.role !== "owner" && JSON.stringify(was) !== JSON.stringify(m)) await team.setMember(m.userId, m.role, m.limits);
      }
      return null;
    } catch (e) {
      return errorMessage(e);
    }
  };

  if (!shared) {
    return (
      <Sheet title="Members & Permissions" onClose={onClose}>
        <p>This project is not a team project yet. Choose Team &gt; Create Team Project to share it and invite people.</p>
      </Sheet>
    );
  }
  if (problem || !members) {
    return (
      <Sheet title="Members & Permissions" onClose={onClose}>
        <p className={problem ? "error" : "secondary"}>{problem ?? "Loading…"}</p>
      </Sheet>
    );
  }
  return (
    <Sheet title="Members & Permissions" onClose={onClose} onSave={iOwn ? save : undefined}>
      <p className="secondary small">
        {iOwn
          ? "Everyone in the project's team can open it. Choose what each person may change; the team server holds everyone to it."
          : "Only the project's owner can change what people may do."}
      </p>
      {removeError && (
        <p role="alert" className="error">
          {removeError}
        </p>
      )}
      <div className="members">
        {members.map((m) => (
          <fieldset key={m.userId} className="member" disabled={!iOwn || m.role === "owner"}>
            <legend>
              <strong>{m.name}</strong> <span className="secondary small">{m.email}</span>
              {m.isMe && <span className="secondary small"> (you)</span>}
            </legend>
            {iOwn && m.role !== "owner" && (
              <div className="member-remove">
                {removing === m.userId ? (
                  <>
                    <span className="small">
                      Remove {m.name} from the team? They lose access to all of its projects and chat.
                    </span>{" "}
                    <button type="button" className="destructive" onClick={() => void remove(m)}>
                      Remove
                    </button>{" "}
                    <button type="button" onClick={() => setRemoving(null)}>
                      Keep
                    </button>
                  </>
                ) : (
                  <button type="button" className="link small" onClick={() => setRemoving(m.userId)}>
                    Remove from team…
                  </button>
                )}
              </div>
            )}
            {m.role === "owner" ? (
              <p className="secondary small">Owner: can change everything, revert changes and decide what others may do.</p>
            ) : (
              <>
                <label className="inline block">
                  Role{" "}
                  <select aria-label={`Role of ${m.name}`} value={m.role} onChange={(e) => update(m.userId, (x) => ({ ...x, role: e.target.value }))}>
                    <option value="editor">Editor: can make changes</option>
                    <option value="viewer">Viewer: can only look</option>
                  </select>
                </label>
                {m.role === "editor" && (
                  <div className="member-limits">
                    <div>
                      {KINDS.map(([key, label]) => (
                        <label key={key} className="inline block">
                          <input type="checkbox" checked={m.limits[key]} onChange={(e) => setLimit(m, key, e.target.checked)} /> {label}
                        </label>
                      ))}
                    </div>
                    <div>
                      <label className="inline block">
                        Tasks{" "}
                        <select
                          aria-label={`Tasks ${m.name} may change`}
                          value={m.limits.tasks.kind}
                          onChange={(e) => {
                            const kind = e.target.value;
                            const tasks: MemberLimits["tasks"] = kind === "phases" ? { kind: "phases", ids: [] } : kind === "assigned" ? { kind: "assigned" } : { kind: "all" };
                            update(m.userId, (x) => ({ ...x, limits: { ...x.limits, tasks } }));
                          }}
                        >
                          <option value="all">All tasks</option>
                          <option value="assigned">Only tasks they are assigned to</option>
                          {phases.length > 0 && <option value="phases">Only these phases…</option>}
                        </select>
                      </label>
                      {m.limits.tasks.kind === "phases" &&
                        phases.map((p) => {
                          const scope = m.limits.tasks as { kind: "phases"; ids: string[] };
                          const on = scope.ids.includes(p.id);
                          return (
                            <label key={p.id} className="inline block indent">
                              <input
                                type="checkbox"
                                checked={on}
                                onChange={(e) =>
                                  update(m.userId, (x) => ({
                                    ...x,
                                    limits: { ...x.limits, tasks: { kind: "phases", ids: e.target.checked ? [...scope.ids, p.id] : scope.ids.filter((i) => i !== p.id) } },
                                  }))
                                }
                              />{" "}
                              {p.name}
                            </label>
                          );
                        })}
                      {m.limits.tasks.kind === "assigned" && (
                        <p className="secondary small">Matched by the resource name: “{m.name}” or “{m.email.split("@")[0]}”.</p>
                      )}
                    </div>
                  </div>
                )}
              </>
            )}
          </fieldset>
        ))}
      </div>
    </Sheet>
  );
}
