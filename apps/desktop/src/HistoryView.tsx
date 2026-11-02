import { useCallback, useEffect, useMemo, useState } from "react";
import { errorMessage, team, type Snapshot } from "./api";
import type { HistoryEntry } from "./bindings/HistoryEntry";

/** "Today at 10:42", "Yesterday at 16:05", "Mon 5 Oct at 09:12". */
function when(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const day = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (day === today) return `Today at ${time}`;
  if (day === today - 86_400_000) return `Yesterday at ${time}`;
  const date = d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: d.getFullYear() === now.getFullYear() ? undefined : "numeric" });
  return `${date} at ${time}`;
}

/**
 * History (team projects): every change anyone made, newest first, in plain words. Filter by
 * person or by words; show the task a change touched; the owner can revert a change, which is
 * recorded as a new change.
 */
export function HistoryView({
  snapshot,
  onShowTask,
  onConfirm,
}: {
  snapshot: Snapshot;
  onShowTask: (row: number) => void;
  onConfirm: (title: string, message: string, action: string) => Promise<boolean>;
}) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [canRevert, setCanRevert] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [person, setPerson] = useState("");
  const [words, setWords] = useState("");

  const rowOf = useMemo(() => new Map(snapshot.rows.map((r, i) => [r.id, i + 1])), [snapshot.rows]);

  // The newest page again whenever the project changes, keeping older pages already loaded.
  const refresh = useCallback(() => {
    team.history(null).then(
      (page) => {
        setProblem(null);
        setCanRevert(page.canRevert);
        setEntries((old) => {
          const newest = page.entries;
          const oldest = newest.at(-1)?.version ?? Infinity;
          const kept = old.filter((e) => e.version < oldest);
          if (kept.length === 0) setHasMore(page.hasMore);
          return [...newest, ...kept];
        });
        setLoading(false);
      },
      (e) => {
        setProblem(errorMessage(e));
        setLoading(false);
      },
    );
  }, []);
  useEffect(() => {
    const t = setTimeout(refresh, 250);
    return () => clearTimeout(t);
  }, [snapshot, refresh]);

  const loadOlder = () => {
    const oldest = entries.at(-1)?.version ?? null;
    team.history(oldest).then(
      (page) => {
        setEntries((old) => [...old, ...page.entries]);
        setHasMore(page.hasMore);
      },
      (e) => setProblem(errorMessage(e)),
    );
  };

  const revert = async (e: HistoryEntry) => {
    if (!(await onConfirm("Revert this change?", `${e.authorName} ${e.summary}. Reverting undoes it now, for everyone; the history keeps both.`, "Revert"))) return;
    try {
      await team.revert(e.version);
    } catch (err) {
      setProblem(errorMessage(err));
    }
  };

  const people = [...new Set(entries.map((e) => e.authorName))].sort();
  const needle = words.trim().toLocaleLowerCase();
  const shown = entries.filter((e) => (!person || e.authorName === person) && (!needle || e.summary.toLocaleLowerCase().includes(needle)));

  return (
    <div className="history">
      <div className="workload-bar">
        <p className="secondary small">Every change anyone made to this team project, newest first.{canRevert ? " As the owner you can revert a change." : ""}</p>
        <div className="history-filters">
          <select aria-label="Person" value={person} onChange={(e) => setPerson(e.target.value)}>
            <option value="">Everyone</option>
            {people.map((p) => (
              <option key={p}>{p}</option>
            ))}
          </select>
          <input type="search" aria-label="Find in history" placeholder="Find a task or change" value={words} onChange={(e) => setWords(e.target.value)} />
        </div>
      </div>
      {problem && (
        <p role="alert" className="banner error">
          {problem}
        </p>
      )}
      <div className="history-scroll">
        {loading && <p className="secondary history-empty">Loading…</p>}
        {!loading && shown.length === 0 && !problem && (
          <p className="secondary history-empty">{entries.length ? "No change matches." : "No changes yet. Every change anyone makes from now on is listed here."}</p>
        )}
        <ol className="history-list">
          {shown.map((e) => {
            const row = e.taskIds.map((id) => rowOf.get(id)).find((r) => r !== undefined);
            return (
              <li key={e.version}>
                <span className="avatar" aria-hidden>
                  {e.authorName.slice(0, 1).toUpperCase()}
                </span>
                <div className="history-text">
                  <div>
                    <strong>{e.authorName}</strong> {e.summary}
                  </div>
                  <div className="secondary small">
                    {when(e.at)} · change {e.version}
                  </div>
                </div>
                <div className="history-actions">
                  {row !== undefined && (
                    <button className="link" onClick={() => onShowTask(row)}>
                      Show task
                    </button>
                  )}
                  {canRevert && (
                    <button onClick={() => void revert(e)} title="Undo this change for everyone">
                      Revert
                    </button>
                  )}
                </div>
              </li>
            );
          })}
        </ol>
        {hasMore && !needle && !person && (
          <button className="link history-more" onClick={loadOlder}>
            Show older changes
          </button>
        )}
      </div>
    </div>
  );
}
