import { Fragment, useMemo, useState } from "react";
import type { Snapshot } from "./api";
import type { DayWork } from "./bindings/DayWork";
import { GLOSSARY } from "./help";

const DAY_MS = 86_400_000;

/** Days since 1970 for `YYYY-MM-DD…`, in UTC so time zones never shift a column. */
const dayOf = (iso: string) => Date.UTC(Number(iso.slice(0, 4)), Number(iso.slice(5, 7)) - 1, Number(iso.slice(8, 10))) / DAY_MS;
const isoOf = (day: number) => new Date(day * DAY_MS).toISOString().slice(0, 10);

/** "8h", "2.5h"; empty for nothing. */
function hours(minutes: number): string {
  if (!minutes) return "";
  return `${Math.round((minutes / 60) * 10) / 10}h`;
}

interface Column {
  /** First day (days since 1970) and how many days it covers. */
  day: number;
  span: number;
  label: string;
  /** Month or year shown above the first column of each group. */
  group: string;
  nonWorking: boolean;
}

/**
 * Workload (Microsoft Project's Resource Usage): for each work resource, the hours asked of it
 * day by day or week by week, red where that is more than it has; under it, the hours per task.
 */
export function Workload({ snapshot, onShowTask }: { snapshot: Snapshot; onShowTask: (row: number) => void }) {
  const people = snapshot.resources.filter((r) => r.resourceType === "Work");
  const start = dayOf(snapshot.project.start);
  const finish = dayOf(snapshot.project.finish ?? snapshot.project.start);
  const [scale, setScale] = useState<"days" | "weeks">(finish - start > 120 ? "weeks" : "days");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const columns = useMemo(() => {
    const exceptions = new Map(snapshot.project.calendarExceptions.map((e) => [e.date, e.working]));
    const working = (day: number) => exceptions.get(isoOf(day)) ?? snapshot.project.workingWeekdays[(new Date(day * DAY_MS).getUTCDay() + 6) % 7];
    const cols: Column[] = [];
    const monthFmt = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric", timeZone: "UTC" });
    if (scale === "days") {
      for (let d = start; d <= finish; d++) {
        const date = new Date(d * DAY_MS);
        cols.push({ day: d, span: 1, label: String(date.getUTCDate()), group: monthFmt.format(date), nonWorking: !working(d) });
      }
    } else {
      // Weeks start on Monday.
      let d = start - ((new Date(start * DAY_MS).getUTCDay() + 6) % 7);
      const dayFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", timeZone: "UTC" });
      for (; d <= finish; d += 7) cols.push({ day: d, span: 7, label: dayFmt.format(new Date(d * DAY_MS)), group: String(new Date(d * DAY_MS).getUTCFullYear()), nonWorking: false });
    }
    return cols;
  }, [scale, start, finish, snapshot.project.calendarExceptions, snapshot.project.workingWeekdays]);

  /** Runs of columns in the same month (days) or year (weeks), for the top header row. */
  const groups = useMemo(() => {
    const out: { day: number; label: string; span: number }[] = [];
    for (const c of columns) {
      const last = out.at(-1);
      if (last && last.label === c.group) last.span++;
      else out.push({ day: c.day, label: c.group, span: 1 });
    }
    return out;
  }, [columns]);

  /** Minutes and capacity per column. */
  const bucket = (days: DayWork[]) => {
    const out = columns.map(() => ({ minutes: 0, capacity: 0 }));
    for (const d of days) {
      const day = dayOf(d.date);
      const i = columns.findIndex((c) => day >= c.day && day < c.day + c.span);
      if (i >= 0) {
        out[i].minutes += d.minutes;
        out[i].capacity += d.capacity;
      }
    }
    return out;
  };

  const toggle = (id: string) =>
    setCollapsed((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  if (people.length === 0) {
    return (
      <div className="workload-empty">
        <p>No one to show yet. Add people in the Resource Sheet, then assign them to tasks.</p>
      </div>
    );
  }

  return (
    <div className="workload">
      <div className="workload-bar">
        <p className="secondary small">{GLOSSARY.workload.text}</p>
        <div className="segmented" role="radiogroup" aria-label="Time scale">
          {(["days", "weeks"] as const).map((s) => (
            <button key={s} role="radio" aria-checked={scale === s} className={scale === s ? "on" : undefined} onClick={() => setScale(s)}>
              {s === "days" ? "Days" : "Weeks"}
            </button>
          ))}
        </div>
      </div>
      <div className="workload-scroll">
        <table className="workload-table">
          <thead>
            <tr>
              <th className="name" rowSpan={2} scope="col">
                Resource / task
              </th>
              {groups.map((g) => (
                <th key={g.day} className="group" colSpan={g.span}>
                  {g.label}
                </th>
              ))}
            </tr>
            <tr>
              {columns.map((c) => (
                <th key={c.day} className={c.nonWorking ? "day off" : "day"} scope="col">
                  {c.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {people.map((r) => {
              const open = !collapsed.has(r.id);
              return (
                <Fragment key={r.id}>
                  <tr className="resource">
                    <th className="name" scope="row">
                      <button className="disclosure" aria-expanded={open} onClick={() => toggle(r.id)} aria-label={open ? `Hide ${r.name}'s tasks` : `Show ${r.name}'s tasks`}>
                        {open ? "▾" : "▸"}
                      </button>
                      {r.name}
                    </th>
                    {bucket(r.days).map((b, i) => {
                      const over = b.minutes > b.capacity;
                      return (
                        <td key={columns[i].day} className={over ? "over" : columns[i].nonWorking ? "off" : undefined} title={over ? `Over-allocated: ${hours(b.minutes)} asked, ${hours(b.capacity) || "0h"} available` : undefined}>
                          {hours(b.minutes)}
                        </td>
                      );
                    })}
                  </tr>
                  {open &&
                    r.assignments.map((a) => (
                      <tr key={a.taskId} className="task">
                        <th className="name" scope="row">
                          <button className="link" onClick={() => onShowTask(a.row)} title="Show in the task list">
                            {a.row}. {a.taskName}
                          </button>
                          {a.unitsText !== "100%" && <span className="secondary"> {a.unitsText}</span>}
                        </th>
                        {bucket(a.days).map((b, i) => (
                          <td key={columns[i].day} className={columns[i].nonWorking ? "off" : undefined}>
                            {hours(b.minutes)}
                          </td>
                        ))}
                      </tr>
                    ))}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
