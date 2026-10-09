import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Edit, EditResult, Snapshot } from "./api";
import type { Row } from "./bindings/Row";
import { formatDate } from "./format";

export const ROW_HEIGHT = 26;
export const HEADER_HEIGHT = 34;
const DAY_MS = 86_400_000;
const BAR_HEIGHT = 12;
const EDGE = 5; // px at a bar's right end that resize instead of move

/** Days since the Unix epoch for a naive "YYYY-MM-DDTHH:MM:SS" (UTC math avoids DST jumps). */
function dayValue(iso: string): number {
  const [d, t = "00:00"] = iso.split("T");
  const [y, m, day] = d.split("-").map(Number);
  const [h, min] = t.split(":").map(Number);
  return Date.UTC(y, m - 1, day, h, min) / DAY_MS;
}

function dateOfDay(day: number): string {
  return new Date(Math.floor(day) * DAY_MS).toISOString().slice(0, 10);
}

interface Colors {
  text: string;
  secondary: string;
  grid: string;
  stripe: string;
  nonWorking: string;
  bar: string;
  critical: string;
  summary: string;
  link: string;
  accent: string;
  warning: string;
  background: string;
}

/** Reads system-derived colours from CSS so the chart follows light and dark mode. */
function readColors(el: HTMLElement): Colors {
  const cs = getComputedStyle(el);
  const v = (name: string) => cs.getPropertyValue(name).trim();
  return {
    text: v("--gantt-text"),
    secondary: v("--gantt-secondary"),
    grid: v("--gantt-grid"),
    stripe: v("--gantt-stripe"),
    nonWorking: v("--gantt-nonworking"),
    bar: v("--gantt-bar"),
    critical: v("--gantt-critical"),
    summary: v("--gantt-summary"),
    link: v("--gantt-link"),
    accent: v("--gantt-accent"),
    warning: v("--gantt-warning"),
    background: v("--gantt-background"),
  };
}

type Drag =
  | { kind: "move"; row: number; startX: number; dx: number }
  | { kind: "resize"; row: number; startX: number; dx: number }
  | { kind: "link"; row: number; startX: number; startY: number; x: number; y: number; linking: boolean };

export interface GanttProps {
  snapshot: Snapshot;
  scrollTop: number;
  selectedIds: Set<string>;
  onEdit: (edit: Edit) => Promise<EditResult>;
  onSelectRow: (row: number) => void;
  onScrollBy: (dy: number) => void;
  onMessage: (message: string | null) => void;
}

/**
 * The Gantt chart: one canvas, redrawn for the visible rows only. Bars can be dragged to move
 * a task (start no earlier than), stretched at the right end to change the duration, or
 * dragged onto another row to link the two tasks. All dates come from the snapshot.
 */
export interface GanttHandle {
  zoom: (factor: number) => void;
}

export const Gantt = forwardRef<GanttHandle, GanttProps>(function Gantt(
  { snapshot, scrollTop, selectedIds, onEdit, onSelectRow, onScrollBy, onMessage },
  handle,
) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ width: 600, height: 400 });
  const [scrollLeft, setScrollLeft] = useState(0);
  const [pxPerDay, setPxPerDay] = useState(28);
  const [drag, setDragState] = useState<Drag | null>(null);
  // Pointer events can arrive before React re-renders; the ref is always current, so a late
  // move after release cannot bring a finished drag back.
  const dragRef = useRef<Drag | null>(null);
  const setDrag = (d: Drag | null) => {
    dragRef.current = d;
    setDragState(d);
  };
  const [hover, setHover] = useState<{ row: number; zone: "bar" | "edge" } | null>(null);
  const rows = snapshot.rows;
  useImperativeHandle(handle, () => ({ zoom: (f: number) => setPxPerDay((p) => Math.max(2, Math.min(120, p * f))) }));

  const origin = useMemo(() => Math.floor(dayValue(snapshot.project.start)) - 3, [snapshot.project.start]);
  const end = useMemo(
    () => Math.ceil(dayValue(snapshot.project.finish ?? snapshot.project.start)) + 21,
    [snapshot.project.finish, snapshot.project.start],
  );
  const contentWidth = (end - origin) * pxPerDay;
  const x = (iso: string) => (dayValue(iso) - origin) * pxPerDay - scrollLeft;
  const rowOfId = useMemo(() => new Map(rows.map((r, i) => [r.id, i])), [rows]);
  const nonWorking = useMemo(() => {
    const exceptions = new Map(snapshot.project.calendarExceptions.map((e) => [e.date, e.working]));
    return (day: number) => {
      const iso = dateOfDay(day);
      const exception = exceptions.get(iso);
      if (exception !== undefined) return !exception;
      const weekday = (new Date(day * DAY_MS).getUTCDay() + 6) % 7; // Monday = 0
      return !snapshot.project.workingWeekdays[weekday];
    };
  }, [snapshot.project.calendarExceptions, snapshot.project.workingWeekdays]);

  useLayoutEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setSize({ width: el.clientWidth, height: el.clientHeight }));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Keep the horizontal scroll inside the timeline.
  useEffect(() => {
    setScrollLeft((s) => Math.max(0, Math.min(s, contentWidth - size.width)));
  }, [contentWidth, size.width]);

  // Draw.
  useEffect(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;
    const frame = requestAnimationFrame(() => {
      const dpr = window.devicePixelRatio || 1;
      if (canvas.width !== size.width * dpr || canvas.height !== size.height * dpr) {
        canvas.width = size.width * dpr;
        canvas.height = size.height * dpr;
      }
      const ctx = canvas.getContext("2d")!;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      draw(ctx, readColors(wrap));
    });
    return () => cancelAnimationFrame(frame);
  });

  function draw(ctx: CanvasRenderingContext2D, c: Colors) {
    const { width, height } = size;
    ctx.clearRect(0, 0, width, height);
    ctx.font = "11px -apple-system, system-ui, sans-serif";
    ctx.textBaseline = "middle";
    const firstDay = origin + Math.floor(scrollLeft / pxPerDay);
    const lastDay = origin + Math.ceil((scrollLeft + width) / pxPerDay);
    const dayX = (day: number) => (day - origin) * pxPerDay - scrollLeft;
    const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT));
    const last = Math.min(rows.length, Math.ceil((scrollTop + height - HEADER_HEIGHT) / ROW_HEIGHT));
    const rowY = (i: number) => HEADER_HEIGHT + i * ROW_HEIGHT - scrollTop;

    // Body: non-working days, row stripes, selection.
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, HEADER_HEIGHT, width, height - HEADER_HEIGHT);
    ctx.clip();
    for (let day = firstDay; day <= lastDay; day++) {
      if (nonWorking(day)) {
        ctx.fillStyle = c.nonWorking;
        ctx.fillRect(dayX(day), HEADER_HEIGHT, pxPerDay, height);
      }
    }
    for (let i = first; i <= last; i++) {
      const r = rows[i];
      if (r && selectedIds.has(r.id)) {
        ctx.fillStyle = c.accent;
        ctx.globalAlpha = 0.16;
        ctx.fillRect(0, rowY(i), width, ROW_HEIGHT);
        ctx.globalAlpha = 1;
      } else if (i % 2) {
        ctx.fillStyle = c.stripe;
        ctx.fillRect(0, rowY(i), width, ROW_HEIGHT);
      }
    }
    // Today.
    const today = new Date();
    const todayX = dayX(Date.UTC(today.getFullYear(), today.getMonth(), today.getDate()) / DAY_MS + 0.5);
    ctx.strokeStyle = c.accent;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(Math.round(todayX) + 0.5, HEADER_HEIGHT);
    ctx.lineTo(Math.round(todayX) + 0.5, height);
    ctx.stroke();

    drawLinks(ctx, c, first, last);
    const hatch = hatchPattern(ctx, c.critical);
    const flagged = new Set([...(snapshot.problem?.taskIds ?? []), ...snapshot.conflicts.map((k) => k.taskId)]);
    for (let i = first; i < last; i++) drawBar(ctx, c, rows[i], rowY(i) + ROW_HEIGHT / 2, hatch, flagged.has(rows[i].id));
    drawDrag(ctx, c, rowY);
    ctx.restore();

    // Header: months (or weeks) above, days below.
    ctx.fillStyle = c.background;
    ctx.fillRect(0, 0, width, HEADER_HEIGHT);
    ctx.strokeStyle = c.grid;
    ctx.fillStyle = c.secondary;
    ctx.beginPath();
    ctx.moveTo(0, HEADER_HEIGHT - 0.5);
    ctx.lineTo(width, HEADER_HEIGHT - 0.5);
    ctx.stroke();
    const monthFmt = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric", timeZone: "UTC" });
    const dayLabel = pxPerDay >= 18;
    // The current month's label sticks to the left edge until the next month pushes it away.
    const nextMonth = (() => {
      for (let day = firstDay + 1; day <= lastDay; day++) if (new Date(day * DAY_MS).getUTCDate() === 1) return dayX(day);
      return Infinity;
    })();
    const stickyLabel = monthFmt.format(new Date(firstDay * DAY_MS));
    ctx.fillText(stickyLabel, Math.min(4, nextMonth - ctx.measureText(stickyLabel).width - 8), 9);
    for (let day = firstDay; day <= lastDay; day++) {
      const d = new Date(day * DAY_MS);
      const xd = dayX(day);
      if (d.getUTCDate() === 1 && day > firstDay) {
        ctx.fillText(monthFmt.format(d), xd + 4, 9);
        ctx.beginPath();
        ctx.moveTo(Math.round(xd) + 0.5, 0);
        ctx.lineTo(Math.round(xd) + 0.5, HEADER_HEIGHT);
        ctx.stroke();
      }
      const weekday = (d.getUTCDay() + 6) % 7;
      if (dayLabel) {
        ctx.fillText(String(d.getUTCDate()), xd + pxPerDay / 2 - 5, 25);
      } else if (weekday === 0) {
        ctx.fillText(String(d.getUTCDate()), xd + 2, 25);
      }
    }
  }

  function barGeometry(r: Row) {
    if (!r.start || !r.finish) return null;
    const x1 = x(r.start);
    const x2 = Math.max(x(r.finish), x1 + 2);
    return { x1, x2 };
  }

  function drawBar(ctx: CanvasRenderingContext2D, c: Colors, r: Row, cy: number, hatch: CanvasPattern | null, flagged: boolean) {
    const g = barGeometry(r);
    if (!g) return;
    let { x1, x2 } = g;
    if (drag && rows[drag.row] === r && drag.kind !== "link") {
      ctx.globalAlpha = 0.35; // the original stays faint while the ghost shows the result
    }
    if (r.isMilestone) {
      const s = 6;
      ctx.beginPath();
      ctx.moveTo(x1, cy - s);
      ctx.lineTo(x1 + s, cy);
      ctx.lineTo(x1, cy + s);
      ctx.lineTo(x1 - s, cy);
      ctx.closePath();
      ctx.fillStyle = r.critical ? c.critical : c.text;
      ctx.fill();
      x2 = x1 + s;
    } else if (r.isSummary) {
      ctx.fillStyle = c.summary;
      ctx.fillRect(x1, cy - 4, x2 - x1, 5);
      for (const ex of [x1, x2]) {
        ctx.beginPath();
        ctx.moveTo(ex - 4, cy + 1);
        ctx.lineTo(ex + 4, cy + 1);
        ctx.lineTo(ex, cy + 6);
        ctx.closePath();
        ctx.fill();
      }
    } else {
      const top = cy - BAR_HEIGHT / 2;
      ctx.fillStyle = r.critical ? c.critical : c.bar;
      roundRect(ctx, x1, top, x2 - x1, BAR_HEIGHT, 3);
      ctx.fill();
      if (r.critical && hatch) {
        // Pattern as well as colour, so critical tasks are clear without colour vision.
        ctx.fillStyle = hatch;
        roundRect(ctx, x1, top, x2 - x1, BAR_HEIGHT, 3);
        ctx.fill();
      }
      if (r.percentComplete > 0) {
        ctx.fillStyle = "rgba(0,0,0,0.35)";
        ctx.fillRect(x1, cy - 2, ((x2 - x1) * r.percentComplete) / 100, 4);
      }
    }
    if (flagged) {
      ctx.strokeStyle = c.warning;
      ctx.lineWidth = 2;
      roundRect(ctx, x1 - 3, cy - BAR_HEIGHT / 2 - 3, x2 - x1 + 6, BAR_HEIGHT + 6, 4);
      ctx.stroke();
      ctx.lineWidth = 1;
    }
    ctx.globalAlpha = 1;
    if (!r.isSummary) {
      ctx.fillStyle = c.text;
      ctx.fillText(r.name, x2 + 6, cy);
    }
  }

  function drawLinks(ctx: CanvasRenderingContext2D, c: Colors, first: number, last: number) {
    const bad = new Set(snapshot.problem?.linkIds ?? []);
    for (const link of snapshot.links) {
      const a = rowOfId.get(link.predecessorId);
      const b = rowOfId.get(link.successorId);
      if (a === undefined || b === undefined) continue;
      if (Math.max(a, b) < first || Math.min(a, b) > last) continue;
      const ga = barGeometry(rows[a]);
      const gb = barGeometry(rows[b]);
      if (!ga || !gb) continue;
      const fromStart = link.kind === "SS" || link.kind === "SF";
      const toFinish = link.kind === "FF" || link.kind === "SF";
      const ya = HEADER_HEIGHT + a * ROW_HEIGHT - scrollTop + ROW_HEIGHT / 2;
      const yb = HEADER_HEIGHT + b * ROW_HEIGHT - scrollTop + ROW_HEIGHT / 2;
      const milestoneA = rows[a].isMilestone ? 6 : 0;
      const milestoneB = rows[b].isMilestone ? 6 : 0;
      const xs = fromStart ? ga.x1 - milestoneA : ga.x2 + milestoneA;
      const xe = toFinish ? gb.x2 + milestoneB : gb.x1 - milestoneB;
      const out = fromStart ? -1 : 1; // leaves the predecessor going left or right
      const into = toFinish ? -1 : 1; // arrives pointing left (into a finish) or right
      const xs2 = xs + out * 8;
      const xe2 = xe - into * 8;
      const critical = rows[a].critical && rows[b].critical;
      ctx.strokeStyle = bad.has(link.id) ? c.warning : critical ? c.critical : c.link;
      ctx.fillStyle = ctx.strokeStyle;
      ctx.lineWidth = bad.has(link.id) ? 2 : 1;
      ctx.beginPath();
      ctx.moveTo(xs, ya);
      ctx.lineTo(xs2, ya);
      const direct = into === 1 ? xs2 <= xe2 + 8 : xs2 >= xe2 - 8;
      const down = yb > ya ? 1 : -1;
      if (direct && into === out) {
        // Straight down to the successor's row, then in.
        ctx.lineTo(xs2, yb);
      } else {
        const ymid = ya + (down * ROW_HEIGHT) / 2;
        ctx.lineTo(xs2, ymid);
        ctx.lineTo(xe2, ymid);
        ctx.lineTo(xe2, yb);
      }
      ctx.lineTo(xe, yb);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(xe, yb);
      ctx.lineTo(xe - into * 5, yb - 4);
      ctx.lineTo(xe - into * 5, yb + 4);
      ctx.closePath();
      ctx.fill();
      ctx.lineWidth = 1;
    }
  }

  function drawDrag(ctx: CanvasRenderingContext2D, c: Colors, rowY: (i: number) => number) {
    if (!drag) return;
    const r = rows[drag.row];
    const g = r && barGeometry(r);
    if (!g) return;
    const cy = rowY(drag.row) + ROW_HEIGHT / 2;
    if (drag.kind === "link") {
      if (!drag.linking) return;
      ctx.strokeStyle = c.accent;
      ctx.setLineDash([4, 3]);
      ctx.beginPath();
      ctx.moveTo(g.x2, cy);
      ctx.lineTo(drag.x, drag.y);
      ctx.stroke();
      ctx.setLineDash([]);
      return;
    }
    let { x1, x2 } = g;
    if (drag.kind === "move") {
      x1 = snapToDay(x1 + drag.dx);
      x2 = x1 + (g.x2 - g.x1);
    } else {
      x2 = Math.max(x1 + pxPerDay / 2, snapToDay(x2 + drag.dx, true));
    }
    ctx.strokeStyle = c.accent;
    ctx.lineWidth = 2;
    ctx.setLineDash([3, 2]);
    if (r.isMilestone) {
      ctx.beginPath();
      ctx.moveTo(x1, cy - 7);
      ctx.lineTo(x1 + 7, cy);
      ctx.lineTo(x1, cy + 7);
      ctx.lineTo(x1 - 7, cy);
      ctx.closePath();
      ctx.stroke();
    } else {
      roundRect(ctx, x1, cy - BAR_HEIGHT / 2, x2 - x1, BAR_HEIGHT, 3);
      ctx.stroke();
    }
    ctx.setLineDash([]);
    ctx.lineWidth = 1;
  }

  /** Snaps a screen x to the start (or end) of the day under it. */
  function snapToDay(sx: number, endOfDay = false) {
    const day = (sx + scrollLeft) / pxPerDay;
    const snapped = endOfDay ? Math.ceil(day) : Math.round(day);
    return snapped * pxPerDay - scrollLeft;
  }

  const dayAt = (sx: number) => origin + Math.floor((sx + scrollLeft) / pxPerDay);

  function hit(e: React.PointerEvent | React.MouseEvent) {
    const rect = canvasRef.current!.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    const row = py < HEADER_HEIGHT ? -1 : Math.floor((py - HEADER_HEIGHT + scrollTop) / ROW_HEIGHT);
    const r = rows[row];
    const g = r && barGeometry(r);
    let zone: "bar" | "edge" | null = null;
    if (g && !r.isSummary) {
      const right = r.isMilestone ? g.x1 + 7 : g.x2;
      const left = r.isMilestone ? g.x1 - 7 : g.x1;
      if (px >= left - 3 && px <= right + 3) {
        zone = !r.isMilestone && px >= right - EDGE && right - left > 2 * EDGE ? "edge" : "bar";
      }
    }
    return { px, py, row, zone };
  }

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const h = hit(e);
    if (h.row >= 0 && h.row < rows.length) onSelectRow(h.row);
    if (!h.zone) return;
    (e.target as Element).setPointerCapture(e.pointerId);
    setDrag(
      h.zone === "edge"
        ? { kind: "resize", row: h.row, startX: h.px, dx: 0 }
        : { kind: "link", row: h.row, startX: h.px, startY: h.py, x: h.px, y: h.py, linking: false },
    );
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const h = hit(e);
    const drag = dragRef.current;
    if (!drag) {
      setHover(h.zone ? { row: h.row, zone: h.zone } : null);
      return;
    }
    const r = rows[drag.row];
    if (drag.kind === "resize") {
      setDrag({ ...drag, dx: h.px - drag.startX });
      onMessage(`Finish: ${formatDate(dateOfDay(dayAt(h.px)))}`);
    } else if (drag.kind === "move") {
      setDrag({ ...drag, dx: h.px - drag.startX });
      const g = barGeometry(r)!;
      onMessage(`Start no earlier than ${formatDate(dateOfDay(dayAt(snapToDay(g.x1 + h.px - drag.startX) + 1)))}`);
    } else {
      // Mostly vertical: draw a link. Mostly horizontal: move the bar.
      const dx = h.px - drag.startX;
      const dy = h.py - drag.startY;
      if (!drag.linking && Math.abs(dx) > 4 && Math.abs(dx) > Math.abs(dy)) {
        setDrag({ kind: "move", row: drag.row, startX: drag.startX, dx });
      } else if (drag.linking || Math.abs(dy) > ROW_HEIGHT * 0.6) {
        setDrag({ ...drag, x: h.px, y: h.py, linking: true });
        const target = rows[h.row];
        onMessage(target && h.row !== drag.row ? `Link “${r.name}” → “${target.name}” (finish-to-start)` : "Drag onto another task to link them");
      }
    }
  };

  const onPointerUp = async (e: React.PointerEvent) => {
    const d = dragRef.current;
    setDrag(null);
    onMessage(null);
    if (!d) return;
    const r = rows[d.row];
    const h = hit(e);
    let edit: Edit | null = null;
    if (d.kind === "move" && Math.abs(d.dx) > 2) {
      const g = barGeometry(r)!;
      edit = { kind: "MoveTask", id: r.id, startDate: dateOfDay(dayAt(snapToDay(g.x1 + d.dx) + 1)) };
    } else if (d.kind === "resize" && Math.abs(d.dx) > 2) {
      edit = { kind: "SetFinish", id: r.id, finishDate: dateOfDay(dayAt(h.px)) };
    } else if (d.kind === "link" && d.linking) {
      const target = rows[h.row];
      if (target && h.row !== d.row) edit = { kind: "AddLink", predecessorId: r.id, successorId: target.id };
    }
    if (!edit) return;
    try {
      const result = await onEdit(edit);
      // A drag cannot pull a task earlier than its links allow; say so instead of silently
      // leaving the bar where it was.
      if (edit.kind === "MoveTask") {
        const moved = result.snapshot.rows.find((x) => x.id === r.id);
        const start = moved?.start?.slice(0, 10);
        if (start && start > edit.startDate) {
          flash(`“${r.name}” cannot start before ${formatDate(moved!.start)}: its predecessors finish later. Change the links to move it earlier.`);
        }
      }
    } catch (err) {
      flash((err as { message?: string }).message ?? String(err));
    }
  };

  const flashTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  function flash(message: string) {
    onMessage(message);
    if (flashTimer.current) clearTimeout(flashTimer.current);
    flashTimer.current = setTimeout(() => onMessage(null), 8000);
  }

  const onWheel = (e: React.WheelEvent) => {
    if (e.ctrlKey || e.metaKey) {
      // Pinch or ⌘-scroll zooms around the pointer.
      const rect = canvasRef.current!.getBoundingClientRect();
      const px = e.clientX - rect.left;
      const dayUnder = (px + scrollLeft) / pxPerDay;
      const next = Math.max(2, Math.min(120, pxPerDay * (e.deltaY < 0 ? 1.15 : 1 / 1.15)));
      setPxPerDay(next);
      setScrollLeft(Math.max(0, dayUnder * next - px));
      return;
    }
    const horizontal = e.shiftKey ? e.deltaY : e.deltaX;
    if (horizontal) setScrollLeft((s) => Math.max(0, Math.min(contentWidth - size.width, s + horizontal)));
    if (!e.shiftKey && e.deltaY) onScrollBy(e.deltaY);
  };

  const cursor = drag
    ? drag.kind === "resize"
      ? "ew-resize"
      : drag.kind === "move"
        ? "grabbing"
        : "crosshair"
    : hover?.zone === "edge"
      ? "ew-resize"
      : hover?.zone === "bar"
        ? "grab"
        : "default";

  return (
    <div ref={wrapRef} className="gantt" onWheel={onWheel} data-tour="chart">
      <canvas
        ref={canvasRef}
        style={{ width: size.width, height: size.height, cursor }}
        role="img"
        aria-label={`Gantt chart of ${rows.length} tasks from ${formatDate(snapshot.project.start)} to ${formatDate(snapshot.project.finish)}. Critical tasks are red with diagonal stripes. Use the task grid to edit with the keyboard.`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerLeave={() => !drag && setHover(null)}
      />
      <div className="gantt-zoom" role="group" aria-label="Zoom">
        <button aria-label="Zoom out" onClick={() => setPxPerDay((p) => Math.max(2, p / 1.5))}>
          −
        </button>
        <button aria-label="Zoom in" onClick={() => setPxPerDay((p) => Math.min(120, p * 1.5))}>
          +
        </button>
      </div>
    </div>
  );
});

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.roundRect(x, y, Math.max(w, 1), h, Math.min(r, w / 2));
}

let hatchCache: { color: string; pattern: CanvasPattern | null } | null = null;
function hatchPattern(ctx: CanvasRenderingContext2D, color: string): CanvasPattern | null {
  if (hatchCache?.color === color) return hatchCache.pattern;
  const tile = document.createElement("canvas");
  tile.width = tile.height = 6;
  const t = tile.getContext("2d")!;
  t.strokeStyle = "rgba(255,255,255,0.55)";
  t.lineWidth = 1.5;
  t.beginPath();
  t.moveTo(-1, 7);
  t.lineTo(7, -1);
  t.stroke();
  hatchCache = { color, pattern: ctx.createPattern(tile, "repeat") };
  return hatchCache.pattern;
}
