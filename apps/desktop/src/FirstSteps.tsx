import { useState } from "react";
import type { Snapshot } from "./bindings/Snapshot";

const KEY = (projectId: string) => `first-steps-closed:${projectId}`;

function closed(projectId: string): boolean {
  try {
    return localStorage.getItem(KEY(projectId)) === "1";
  } catch {
    return false;
  }
}

/**
 * First steps on a project: four things that make a plan useful, each ticked as soon as the
 * project shows it is done. Closed for good with ×, per project (this Mac only).
 */
export function FirstSteps({ snapshot, onAddTask, onSetBaseline, onShowResources }: { snapshot: Snapshot; onAddTask: () => void; onSetBaseline: () => void; onShowResources: () => void }) {
  const id = snapshot.project.path; // the file: one checklist per project
  const [hidden, setHidden] = useState(() => closed(id));
  const tasks = snapshot.rows.filter((r) => !r.isSummary).length;
  const steps = [
    { done: tasks >= 2, label: "Add the tasks", how: "Type names in the list; Tab makes a task part of the one above.", action: tasks >= 2 ? null : { text: "Add a task", run: onAddTask } },
    { done: snapshot.links.length > 0, label: "Link tasks that wait for each other", how: "Type the row number of the task it waits for in Predecessors, or drag from one bar to another.", action: null },
    { done: snapshot.rows.some((r) => r.resourceNames.trim() !== ""), label: "Say who does what", how: "Type names in the Resources column; the Workload view then shows who is overbooked.", action: { text: "Open Resources", run: onShowResources } },
    { done: snapshot.project.baselineSavedAt !== null, label: "Save a baseline before you start", how: "It keeps today's plan, so you can see later what moved.", action: tasks >= 2 ? { text: "Save Baseline", run: onSetBaseline } : null },
  ];
  const left = steps.filter((s) => !s.done).length;
  if (hidden || left === 0) return null;
  const close = () => {
    try {
      localStorage.setItem(KEY(id), "1");
    } catch {
      // Not remembered; closed for now.
    }
    setHidden(true);
  };
  return (
    <aside className="first-steps" aria-label="First steps">
      <header>
        <strong>First steps</strong>
        <span className="secondary small">
          {steps.length - left} of {steps.length} done
        </span>
        <button className="link" onClick={close} aria-label="Close first steps" title="Close (it does not come back for this project)">
          ×
        </button>
      </header>
      <ol>
        {steps.map((s) => (
          <li key={s.label} className={s.done ? "done" : undefined}>
            <span aria-hidden>{s.done ? "✓" : "○"}</span>
            <div>
              <div>
                {s.label}
                {s.done && <span className="visually-hidden"> (done)</span>}
              </div>
              {!s.done && <div className="secondary small">{s.how}</div>}
              {!s.done && s.action && (
                <button className="link small" onClick={s.action.run}>
                  {s.action.text}
                </button>
              )}
            </div>
          </li>
        ))}
      </ol>
    </aside>
  );
}
