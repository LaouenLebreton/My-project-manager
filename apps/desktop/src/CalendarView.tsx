import { useMemo, useState } from "react";
import type { Snapshot } from "./api";
import type { Row } from "./bindings/Row";

const DAY_MS = 86_400_000;
/** A week shows this many rows of bars before "+ more". */
const LANES = 5;

/** Days since 1970 for `YYYY-MM-DD…`, in UTC so time zones never shift a day. */
const dayOf = (iso: string) => Date.UTC(Number(iso.slice(0, 4)), Number(iso.slice(5, 7)) - 1, Number(iso.slice(8, 10))) / DAY_MS;
const isoOf = (day: number) => new Date(day * DAY_MS).toISOString().slice(0, 10);
const mondayOf = (day: number) => day - ((new Date(day * DAY_MS).getUTCDay() + 6) % 7);

interface Bar {
  row: Row;
  number: number;
  first: number;
  last: number;
}

/**
 * Calendar (Microsoft Project's Calendar view): a month at a time, each task a bar across the
 * days it runs. Click a bar to find the task in the task list.
 */
export function CalendarView({ snapshot, onShowTask }: { snapshot: Snapshot; onShowTask: (row: number) => void }) {
  const todayIso = new Date().toISOString().slice(0, 10);
  const [month, setMonth] = useState(() => {
    const start = snapshot.project.start.slice(0, 10);
    const finish = (snapshot.project.finish ?? snapshot.project.start).slice(0, 10);
    const at = todayIso >= start && todayIso <= finish ? todayIso : start;
    return { year: Number(at.slice(0, 4)), month: Number(at.slice(5, 7)) - 1 };
  });
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const bars = useMemo(
    () =>
      snapshot.rows
        .map((row, i) => ({ row, number: i + 1 }))
        .filter(({ row }) => !row.isSummary && row.start && row.finish)
        .map(({ row, number }): Bar => ({ row, number, first: dayOf(row.start!), last: Math.max(dayOf(row.start!), dayOf(row.finish!)) }))
        .sort((a, b) => a.first - b.first || b.last - a.last || a.number - b.number),
    [snapshot.rows],
  );

  const working = useMemo(() => {
    const exceptions = new Map(snapshot.project.calendarExceptions.map((e) => [e.date, e.working]));
    return (day: number) => exceptions.get(isoOf(day)) ?? snapshot.project.workingWeekdays[(new Date(day * DAY_MS).getUTCDay() + 6) % 7];
  }, [snapshot.project.calendarExceptions, snapshot.project.workingWeekdays]);

  const firstOfMonth = Date.UTC(month.year, month.month, 1) / DAY_MS;
  const lastOfMonth = Date.UTC(month.year, month.month + 1, 0) / DAY_MS;
  const weeks: number[] = [];
  for (let w = mondayOf(firstOfMonth); w <= lastOfMonth; w += 7) weeks.push(w);

  const title = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric", timeZone: "UTC" }).format(new Date(firstOfMonth * DAY_MS));
  const weekdayFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", timeZone: "UTC" });
  const shift = (by: number) => setMonth((m) => ({ year: m.year + Math.floor((m.month + by) / 12), month: (((m.month + by) % 12) + 12) % 12 }));
  const goToday = () => setMonth({ year: Number(todayIso.slice(0, 4)), month: Number(todayIso.slice(5, 7)) - 1 });

  return (
    <div className="calendar">
      <div className="workload-bar">
        <div className="calendar-nav">
          <button onClick={() => shift(-1)} aria-label="Previous month">
            ‹
          </button>
          <button onClick={goToday}>Today</button>
          <button onClick={() => shift(1)} aria-label="Next month">
            ›
          </button>
          <h2>{title}</h2>
        </div>
        <p className="secondary small">Each bar is a task across the days it runs. Click one to find it in the task list.</p>
      </div>
      <div className="calendar-grid" role="grid" aria-label={title}>
        <div className="calendar-week calendar-head" role="row">
          {[0, 1, 2, 3, 4, 5, 6].map((d) => (
            <div key={d} role="columnheader" className="calendar-dayname">
              {weekdayFmt.format(new Date((weeks[0] + d) * DAY_MS))}
            </div>
          ))}
        </div>
        {weeks.map((w) => {
          // Bars in this week, packed into lanes so they never overlap.
          const inWeek = bars.filter((b) => b.first <= w + 6 && b.last >= w);
          const laneEnds: number[] = [];
          const placed = inWeek.map((b) => {
            const from = Math.max(b.first, w);
            let lane = laneEnds.findIndex((end) => end < from);
            if (lane < 0) lane = laneEnds.push(0) - 1;
            laneEnds[lane] = Math.min(b.last, w + 6);
            return { ...b, lane, from, to: Math.min(b.last, w + 6) };
          });
          const open = expanded.has(w);
          const shown = open ? placed : placed.filter((b) => b.lane < LANES);
          const hidden = placed.length - shown.length;
          return (
            <div key={w} className="calendar-week" role="row">
              <div className="calendar-days">
              {[0, 1, 2, 3, 4, 5, 6].map((d) => {
                const day = w + d;
                const iso = isoOf(day);
                const classes = ["calendar-day", day < firstOfMonth || day > lastOfMonth ? "outside" : "", working(day) ? "" : "off", iso === todayIso ? "today" : ""];
                return (
                  <div key={d} role="gridcell" className={classes.filter(Boolean).join(" ")}>
                    <span className="calendar-date">{new Date(day * DAY_MS).getUTCDate()}</span>
                  </div>
                );
              })}
              </div>
              {shown.map((b) => (
                <button
                  key={b.row.id}
                  className={["calendar-bar", b.row.critical ? "critical" : "", b.row.percentComplete >= 100 ? "done" : "", b.row.isMilestone ? "milestone" : "", b.first < w ? "cont-left" : "", b.last > w + 6 ? "cont-right" : ""].filter(Boolean).join(" ")}
                  style={{ gridColumn: `${b.from - w + 1} / ${b.to - w + 2}`, gridRow: b.lane + 2 }}
                  onClick={() => onShowTask(b.number)}
                  title={`${b.number}. ${b.row.name}${b.row.resourceNames ? ` (${b.row.resourceNames})` : ""}${b.row.critical ? ", on the critical path" : ""}`}
                >
                  {b.row.isMilestone ? "◆ " : ""}
                  {b.row.name}
                </button>
              ))}
              {hidden > 0 && (
                <button className="link calendar-more" style={{ gridRow: LANES + 2 }} onClick={() => setExpanded((s) => new Set(s).add(w))}>
                  + {hidden} more
                </button>
              )}
              {open && placed.some((b) => b.lane >= LANES) && (
                <button
                  className="link calendar-more"
                  style={{ gridRow: Math.max(...placed.map((b) => b.lane)) + 3 }}
                  onClick={() =>
                    setExpanded((s) => {
                      const n = new Set(s);
                      n.delete(w);
                      return n;
                    })
                  }
                >
                  Show less
                </button>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
