import { useCallback, useEffect, useMemo, useState } from "react";
import { errorMessage, portfolio } from "./api";
import type { PortfolioRow } from "./bindings/PortfolioRow";
import type { PortfolioView as Data } from "./bindings/PortfolioView";
import { formatDate, formatMoney } from "./format";

const DAY_MS = 86_400_000;
const dayOf = (iso: string) => Date.UTC(Number(iso.slice(0, 4)), Number(iso.slice(5, 7)) - 1, Number(iso.slice(8, 10))) / DAY_MS;

const STATUS: Record<string, { label: string; order: number }> = {
  late: { label: "Late", order: 0 },
  problem: { label: "Problem", order: 1 },
  atRisk: { label: "At risk", order: 2 },
  onTrack: { label: "On track", order: 3 },
  done: { label: "Done", order: 4 },
};

/** "+3 days", "−1 day", "on plan". */
function variance(minutes: number | null): string {
  if (minutes === null) return "—";
  if (minutes === 0) return "on plan";
  const d = Math.round((Math.abs(minutes) / 480) * 10) / 10;
  return `${minutes > 0 ? "+" : "−"}${d} day${d === 1 ? "" : "s"}`;
}

function StatusChip({ row }: { row: PortfolioRow }) {
  return (
    <span className={`status-chip ${row.status}`} title={row.reasons.join("; ") || "Nothing needs attention"}>
      {STATUS[row.status]?.label ?? row.status}
    </span>
  );
}

/** Cost and baseline added up per currency (projects may use different ones). */
/** `budgetedCost` is the cost of the projects that have a baseline, to compare with it. */
type Total = { currency: string; cost: number; baseline: number; budgetedCost: number; unplanned: number };
function totals(rows: PortfolioRow[]): Total[] {
  const by = new Map<string, Total>();
  for (const r of rows) {
    if (r.status === "problem") continue;
    const t = by.get(r.currency) ?? { currency: r.currency, cost: 0, baseline: 0, budgetedCost: 0, unplanned: 0 };
    t.cost += r.costCents;
    if (r.baselineCostCents === null) t.unplanned += 1;
    else {
      t.baseline += r.baselineCostCents;
      t.budgetedCost += r.costCents;
    }
    by.set(r.currency, t);
  }
  return [...by.values()].filter((t) => t.cost || t.baseline);
}

function TotalLine({ rows, label }: { rows: PortfolioRow[]; label: string }) {
  const sums = totals(rows);
  return (
    <tr className="portfolio-total">
      <th scope="row" colSpan={6}>
        {label}
      </th>
      <td className="right">
        {sums.length === 0
          ? "—"
          : sums.map((t) => (
              <div key={t.currency}>
                {formatMoney(t.cost, t.currency)}
                {t.baseline > 0 && (
                  <div
                    className={`small ${t.budgetedCost > t.baseline ? "late-text" : "secondary"}`}
                    title={t.unplanned > 0 ? `Budget: the baseline cost of the projects that have a baseline (${formatMoney(t.budgetedCost, t.currency)} now)` : "Budget: the baseline cost"}
                  >
                    budget {formatMoney(t.baseline, t.currency)}
                    {t.budgetedCost !== t.baseline && ` (${t.budgetedCost > t.baseline ? "over" : "under"} by ${formatMoney(Math.abs(t.budgetedCost - t.baseline), t.currency)})`}
                    {t.unplanned > 0 && <div className="secondary">{t.unplanned} without a budget</div>}
                  </div>
                )}
              </div>
            ))}
      </td>
      <td colSpan={2} />
    </tr>
  );
}

function GroupField({ row, onGroup }: { row: PortfolioRow; onGroup: (path: string, group: string) => void }) {
  const [value, setValue] = useState(row.group ?? "");
  useEffect(() => setValue(row.group ?? ""), [row.group]);
  const commit = () => value.trim() !== (row.group ?? "") && onGroup(row.path, value);
  return (
    <input
      className="group-field"
      list="portfolio-groups"
      value={value}
      placeholder="No group"
      aria-label={`Group of ${row.name}`}
      onChange={(e) => setValue(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        // Return saves even while the suggestion list has the key.
        if (e.key === "Enter") {
          commit();
          e.currentTarget.blur();
        }
        if (e.key === "Escape") setValue(row.group ?? "");
      }}
    />
  );
}

function Projects({ rows: all, groups, grouped, onOpen, onRemove, onGroup }: { rows: PortfolioRow[]; groups: string[]; grouped: boolean; onOpen: (p: string) => void; onRemove: (p: string) => void; onGroup: (path: string, group: string) => void }) {
  const rows = [...all].sort((a, b) => (STATUS[a.status]?.order ?? 9) - (STATUS[b.status]?.order ?? 9) || a.name.localeCompare(b.name));
  // Grouped: each group (A–Z, then projects without one) with its own total.
  const sections: { name: string | null; rows: PortfolioRow[] }[] = grouped
    ? [...new Set(rows.map((r) => r.group ?? null))]
        .sort((a, b) => (a === null ? 1 : b === null ? -1 : a.localeCompare(b)))
        .map((name) => ({ name, rows: rows.filter((r) => (r.group ?? null) === name) }))
    : [{ name: null, rows }];
  return (
    <table className="portfolio-table">
      <datalist id="portfolio-groups">
        {groups.map((g) => (
          <option key={g} value={g} />
        ))}
      </datalist>
      <thead>
        <tr>
          <th>Project</th>
          <th>Group</th>
          <th>Status</th>
          <th>Finish</th>
          <th title="Against the baseline finish">Slip</th>
          <th className="right">Done</th>
          <th className="right">Cost</th>
          <th>Next milestone</th>
          <th />
        </tr>
      </thead>
      {sections.map((sec) => (
      <tbody key={sec.name ?? ""}>
        {grouped && (
          <tr className="portfolio-group">
            <th scope="rowgroup" colSpan={9}>
              {sec.name ?? "No group"} <span className="secondary small">· {sec.rows.length}</span>
            </th>
          </tr>
        )}
        {sec.rows.map((r) => (
          <tr key={r.path}>
            <td>
              <button className="link project-link" onClick={() => onOpen(r.path)} title={r.path}>
                {r.name}
              </button>
              {r.shared && <span className="secondary small"> · team</span>}
            </td>
            <td>
              <GroupField row={r} onGroup={onGroup} />
            </td>
            <td>
              <StatusChip row={r} />
              {r.reasons.length > 0 && <div className="secondary small reasons">{r.reasons.join("; ")}</div>}
            </td>
            <td>{r.finish ? formatDate(r.finish) : "—"}</td>
            <td className={r.finishVarianceMinutes && r.finishVarianceMinutes > 0 ? "late-text" : undefined}>{variance(r.finishVarianceMinutes)}</td>
            <td className="right">
              <div className="mini-progress" role="progressbar" aria-valuenow={r.percentComplete} aria-valuemin={0} aria-valuemax={100} aria-label="% complete">
                <span style={{ width: `${r.percentComplete}%` }} />
              </div>
              {r.percentComplete}%
            </td>
            <td className="right">
              {r.costCents ? formatMoney(r.costCents, r.currency) : "—"}
              {r.baselineCostCents !== null && r.baselineCostCents !== r.costCents && (
                <div className="secondary small">baseline {formatMoney(r.baselineCostCents, r.currency)}</div>
              )}
            </td>
            <td>
              {r.nextMilestone ? (
                <>
                  ◆ {r.nextMilestone.name}
                  <div className="secondary small">{formatDate(r.nextMilestone.date)}</div>
                </>
              ) : (
                "—"
              )}
            </td>
            <td>
              <button className="link small" onClick={() => onRemove(r.path)} title="Take out of the portfolio (the file is not touched)">
                Remove
              </button>
            </td>
          </tr>
        ))}
        {grouped && sections.length > 1 && <TotalLine rows={sec.rows} label={`${sec.name ?? "No group"} total`} />}
      </tbody>
      ))}
      <tfoot>
        <TotalLine rows={rows} label={`Total · ${rows.length} project${rows.length === 1 ? "" : "s"}`} />
      </tfoot>
    </table>
  );
}

/** What the portfolio shows; kept between launches (this Mac only). */
type Filter = { status: string; where: "all" | "team" | "local"; group: string; grouped: boolean };
const FILTER_KEY = "portfolio-filter";
const NO_FILTER: Filter = { status: "all", where: "all", group: "all", grouped: false };
function savedFilter(): Filter {
  try {
    return { ...NO_FILTER, ...JSON.parse(localStorage.getItem(FILTER_KEY) ?? "{}") };
  } catch {
    return NO_FILTER;
  }
}
const NO_GROUP = "\u0000none";
function matches(r: PortfolioRow, f: Filter): boolean {
  if (f.status !== "all" && r.status !== f.status) return false;
  if (f.where === "team" && !r.shared) return false;
  if (f.where === "local" && r.shared) return false;
  if (f.group === NO_GROUP) return !r.group;
  if (f.group !== "all" && r.group !== f.group) return false;
  return true;
}

/** Every project as one bar on a shared time scale, with milestones and today. */
function Timeline({ data, onOpen }: { data: Data; onOpen: (p: string) => void }) {
  const rows = data.projects.filter((p) => p.start && p.finish);
  const range = useMemo(() => {
    const today = dayOf(data.today);
    const days = rows.flatMap((p) => [dayOf(p.start!), dayOf(p.finish!)]);
    if (days.length === 0) return null;
    const from = Math.min(...days, today) - 7;
    const to = Math.max(...days, today) + 7;
    return { from, to, today };
  }, [rows, data.today]);
  if (!range) return <p className="secondary">No dates to show yet.</p>;
  const pct = (day: number) => `${((day - range.from) / (range.to - range.from)) * 100}%`;
  const months: { day: number; label: string }[] = [];
  for (let d = range.from; d <= range.to; d++) {
    const date = new Date(d * DAY_MS);
    // The first label only when the next month is not about to start (they would overlap).
    if (date.getUTCDate() === 1 || (d === range.from && date.getUTCDate() <= 18)) months.push({ day: d, label: date.toLocaleDateString(undefined, { month: "short", year: "2-digit", timeZone: "UTC" }) });
  }
  return (
    <div className="timeline">
      <div className="timeline-scale">
        {months.map((m) => (
          <span key={m.day} style={{ left: pct(m.day) }}>
            {m.label}
          </span>
        ))}
      </div>
      {rows.map((r) => (
        <div key={r.path} className="timeline-row">
          <button className="link timeline-name" onClick={() => onOpen(r.path)}>
            {r.name}
          </button>
          <div className="timeline-track">
            <div className="today-line" style={{ left: pct(range.today) }} />
            <div className={`timeline-bar ${r.status}`} style={{ left: pct(dayOf(r.start!)), width: `calc(${pct(dayOf(r.finish!) + 1)} - ${pct(dayOf(r.start!))})` }} title={`${formatDate(r.start)} – ${formatDate(r.finish)} · ${r.percentComplete}% done`}>
              <span className="timeline-progress" style={{ width: `${r.percentComplete}%` }} />
            </div>
            {r.baselineFinish && <div className="baseline-mark" style={{ left: pct(dayOf(r.baselineFinish) + 1) }} title={`Baseline finish ${formatDate(r.baselineFinish)}`} />}
            {r.milestones.map((m) => (
              <span key={m.name + m.date} className="timeline-milestone" style={{ left: pct(dayOf(m.date)) }} title={`${m.name} · ${formatDate(m.date)}`}>
                ◆
              </span>
            ))}
          </div>
        </div>
      ))}
      <p className="secondary small">Bars run from each project's start to its finish, filled as far as it is done; the thin line is the baseline finish; ◆ milestones; blue line today.</p>
    </div>
  );
}

/** Each person's hours per week across all projects; red where it is more than they have. */
function People({ data }: { data: Data }) {
  if (data.people.length === 0) return <p className="secondary">No one is assigned to tasks in these projects yet.</p>;
  const hours = (m: number) => (m ? `${Math.round(m / 60)}h` : "");
  const sorted = [...data.people].sort((a, b) => {
    const over = (p: typeof a) => p.weeks.filter((w) => w.minutes > w.capacity).length;
    return over(b) - over(a) || a.name.localeCompare(b.name);
  });
  return (
    <div className="workload-scroll">
      <p className="secondary small">
        Hours each person is asked for per week, across every project here (matched by name). Red: more than they have. Hover for the projects.
      </p>
      <table className="workload-table">
        <thead>
          <tr>
            <th className="name">Person</th>
            {data.weeks.map((w) => (
              <th key={w} className="day">
                {new Date(dayOf(w) * DAY_MS).toLocaleDateString(undefined, { day: "numeric", month: "short", timeZone: "UTC" })}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sorted.map((p) => (
            <tr key={p.name} className="resource">
              <th className="name" scope="row">
                {p.name}
                <span className="secondary small"> · {p.projectCount} project{p.projectCount === 1 ? "" : "s"}</span>
              </th>
              {p.weeks.map((w, i) => {
                const over = w.minutes > w.capacity;
                const detail = w.projects.map(([name, m]) => `${name}: ${hours(m)}`).join("\n");
                return (
                  <td key={data.weeks[i]} className={over ? "over" : undefined} title={w.minutes ? `${hours(w.minutes)} of ${hours(w.capacity) || "0h"}\n${detail}` : undefined}>
                    {hours(w.minutes)}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * The Portfolio window: every project at a glance (status, finish, slip, progress, cost, next
 * milestone), a combined timeline, and people's work across projects.
 */
export function PortfolioView() {
  const [data, setData] = useState<Data | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [tab, setTab] = useState<"projects" | "timeline" | "people">("projects");
  const [filter, setFilterState] = useState<Filter>(savedFilter);
  const setFilter = (f: Partial<Filter>) =>
    setFilterState((old) => {
      const next = { ...old, ...f };
      try {
        localStorage.setItem(FILTER_KEY, JSON.stringify(next));
      } catch {
        // Not kept; still applies now.
      }
      return next;
    });

  const load = useCallback(() => {
    portfolio.data().then(
      (d) => {
        setData(d);
        setProblem(null);
      },
      (e) => setProblem(errorMessage(e)),
    );
  }, []);
  useEffect(() => {
    load();
    // Fresh numbers whenever the window comes to the front.
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [load]);

  const open = (path: string) => void portfolio.open(path).catch((e) => setProblem(errorMessage(e)));
  const remove = (path: string) => void portfolio.remove(path).then(load);
  const add = () => void portfolio.add().then((added) => added && load(), (e) => setProblem(errorMessage(e)));
  const setGroup = (path: string, group: string) => void portfolio.setGroup(path, group).then(load, (e) => setProblem(errorMessage(e)));

  // The filter applies to every view, so the timeline and people match the list.
  const shown = useMemo(() => (data ? data.projects.filter((r) => matches(r, filter)) : []), [data, filter]);
  const view = useMemo(() => {
    if (!data) return null;
    const names = new Set(shown.map((p) => p.name));
    const people = data.people
      .map((p) => {
        const weeks = p.weeks.map((w) => {
          const projects = w.projects.filter(([n]) => names.has(n));
          return { ...w, projects, minutes: projects.reduce((s, [, m]) => s + m, 0) };
        });
        return { ...p, weeks, projectCount: new Set(weeks.flatMap((w) => w.projects.map(([n]) => n))).size };
      })
      .filter((p) => p.projectCount > 0);
    return { ...data, projects: shown, people };
  }, [data, shown]);
  const filtered = filter.status !== "all" || filter.where !== "all" || filter.group !== "all";

  const counts = useMemo(() => {
    const c: Record<string, number> = {};
    for (const p of shown) c[p.status] = (c[p.status] ?? 0) + 1;
    return c;
  }, [shown]);
  const overbooked = view?.people.filter((p) => p.weeks.some((w) => w.minutes > w.capacity)).length ?? 0;

  return (
    <main className="portfolio">
      <header className="toolbar">
        <div>
          <h1>Portfolio</h1>
          <p className="secondary">
            {data ? `${shown.length}${filtered ? ` of ${data.projects.length}` : ""} project${data.projects.length === 1 ? "" : "s"}` : "Loading…"}
            {(["late", "atRisk", "onTrack", "done"] as const).filter((k) => counts[k]).map((k) => ` · ${counts[k]} ${STATUS[k].label.toLowerCase()}`)}
            {overbooked > 0 && ` · ${overbooked} overbooked ${overbooked === 1 ? "person" : "people"}`}
          </p>
        </div>
        <div className="portfolio-actions">
          <button onClick={add}>Add Project…</button>
          <button onClick={load} title="Read every project again">
            Refresh
          </button>
          <div className="segmented" role="tablist" aria-label="Portfolio view">
            {(["projects", "timeline", "people"] as const).map((t) => (
              <button key={t} role="tab" aria-selected={tab === t} className={tab === t ? "on" : undefined} onClick={() => setTab(t)}>
                {t === "projects" ? "Projects" : t === "timeline" ? "Timeline" : "People"}
              </button>
            ))}
          </div>
        </div>
      </header>
      {problem && (
        <p role="alert" className="banner error">
          {problem}
        </p>
      )}
      {data && data.projects.length > 0 && (
        <div className="portfolio-filters" role="group" aria-label="Show">
          <label>
            Status{" "}
            <select value={filter.status} onChange={(e) => setFilter({ status: e.target.value })}>
              <option value="all">Any</option>
              {Object.entries(STATUS).map(([k, s]) => (
                <option key={k} value={k}>
                  {s.label}
                </option>
              ))}
            </select>
          </label>
          <label>
            Projects{" "}
            <select value={filter.where} onChange={(e) => setFilter({ where: e.target.value as Filter["where"] })}>
              <option value="all">Team and local</option>
              <option value="team">Team only</option>
              <option value="local">Local only</option>
            </select>
          </label>
          <label>
            Group{" "}
            <select value={filter.group} onChange={(e) => setFilter({ group: e.target.value })}>
              <option value="all">Any</option>
              {data.groups.map((g) => (
                <option key={g} value={g}>
                  {g}
                </option>
              ))}
              <option value={NO_GROUP}>No group</option>
            </select>
          </label>
          {tab === "projects" && (
            <label>
              <input type="checkbox" checked={filter.grouped} onChange={(e) => setFilter({ grouped: e.target.checked })} /> Show by group
            </label>
          )}
          {filtered && (
            <button className="link small" onClick={() => setFilter({ status: "all", where: "all", group: "all" })}>
              Show all
            </button>
          )}
        </div>
      )}
      <div className="portfolio-body">
        {data && data.projects.length === 0 && <p className="secondary">No projects yet. Projects you open appear here; Add Project brings in others.</p>}
        {data && data.projects.length > 0 && shown.length === 0 && <p className="secondary">No projects match. <button className="link" onClick={() => setFilter({ status: "all", where: "all", group: "all" })}>Show all</button></p>}
        {view && shown.length > 0 && tab === "projects" && <Projects rows={shown} groups={data!.groups} grouped={filter.grouped} onOpen={open} onRemove={remove} onGroup={setGroup} />}
        {view && shown.length > 0 && tab === "timeline" && <Timeline data={view} onOpen={open} />}
        {view && shown.length > 0 && tab === "people" && <People data={view} />}
      </div>
    </main>
  );
}
