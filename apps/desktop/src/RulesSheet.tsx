import { useEffect, useState } from "react";
import { errorMessage, team } from "./api";
import type { ProjectRules } from "./bindings/ProjectRules";
import type { Snapshot } from "./bindings/Snapshot";
import { Sheet } from "./Sheets";

const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/**
 * Team > Automation Rules: what the team server does on its own for this project. The owner
 * changes them; everyone else sees them.
 */
export function RulesSheet({ snapshot, onClose }: { snapshot: Snapshot; onClose: () => void }) {
  const [rules, setRules] = useState<ProjectRules | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const shared = snapshot.project.team !== null;
  const owner = snapshot.project.team?.access?.role === "owner";

  useEffect(() => {
    if (!shared) return;
    team.rules().then(setRules, (e) => setProblem(errorMessage(e)));
  }, [shared]);

  if (!shared) {
    return (
      <Sheet title="Automation Rules" onClose={onClose}>
        <p>Rules run on the team server, so they are for team projects. Make this one a team project first (Team &gt; Create Team Project).</p>
      </Sheet>
    );
  }
  if (!rules) {
    return (
      <Sheet title="Automation Rules" onClose={onClose}>
        <p className="secondary">{problem ?? "Loading…"}</p>
      </Sheet>
    );
  }
  const weekly = rules.weeklySummary;
  const set = (r: Partial<ProjectRules>) => setRules({ ...rules, ...r });
  const save = owner
    ? async () => {
        try {
          await team.setRules(rules);
          return null;
        } catch (e) {
          return errorMessage(e);
        }
      }
    : undefined;

  return (
    <Sheet title="Automation Rules" onClose={onClose} onSave={save}>
      <p className="secondary small">
        The team server runs these once for everyone, whether or not anyone has the project open. A team hosted on a Mac runs them while Super PM is open
        there. Posts go to the project's channel in Chat, from Super PM.
        {!owner && " Only the project's owner can change them."}
      </p>
      <fieldset className="rules" disabled={!owner}>
        <label className="rule">
          <input type="checkbox" checked={rules.postWhenDone} onChange={(e) => set({ postWhenDone: e.target.checked })} />
          <span>
            <strong>When a task is done</strong>
            <span className="secondary small">When a task reaches 100%, post for example: ✅ “Survey” is done (marked by Alex).</span>
          </span>
        </label>
        <label className="rule">
          <input type="checkbox" checked={weekly !== null} onChange={(e) => set({ weeklySummary: e.target.checked ? { weekday: 0, hour: 9 } : null })} />
          <span>
            <strong>Weekly status summary</strong>
            <span className="secondary small">Status, % done, finish date, late tasks, what is due that week and the next milestone.</span>
          </span>
        </label>
        {weekly && (
          <div className="rule-when">
            Every{" "}
            <select value={weekly.weekday} onChange={(e) => set({ weeklySummary: { ...weekly, weekday: Number(e.target.value) } })} aria-label="Day of the week">
              {WEEKDAYS.map((w, i) => (
                <option key={w} value={i}>
                  {w}
                </option>
              ))}
            </select>{" "}
            at{" "}
            <select value={weekly.hour} onChange={(e) => set({ weeklySummary: { ...weekly, hour: Number(e.target.value) } })} aria-label="Hour">
              {Array.from({ length: 24 }, (_, h) => (
                <option key={h} value={h}>
                  {String(h).padStart(2, "0")}:00
                </option>
              ))}
            </select>
            <span className="secondary small"> (the team server's time)</span>
          </div>
        )}
      </fieldset>
    </Sheet>
  );
}
