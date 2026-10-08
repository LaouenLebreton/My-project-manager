// Display formatting only. Dates and durations are calculated in Rust.

/** "2026-10-05T08:00:00" (naive local time) to a Date in the user's time zone. */
function parse(iso: string): Date {
  const [d, t = "00:00"] = iso.split("T");
  const [y, m, day] = d.split("-").map(Number);
  const [h, min] = t.split(":").map(Number);
  return new Date(y, m - 1, day, h, min);
}

const dateFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" });
const dateTimeFmt = new Intl.DateTimeFormat(undefined, { dateStyle: "full", timeStyle: "short" });

export function formatDate(iso: string | null): string {
  return iso ? dateFmt.format(parse(iso)) : "—";
}

export function formatDateTime(iso: string | null): string {
  return iso ? dateTimeFmt.format(parse(iso)) : "";
}

const numberFmt = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });

/** Working minutes as days, e.g. "1 day", "2.5 days". */
export function formatDays(minutes: number | null, minutesPerDay: number): string {
  if (minutes === null) return "—";
  const days = minutes / minutesPerDay;
  return `${numberFmt.format(days)} ${Math.abs(days) === 1 ? "day" : "days"}`;
}
