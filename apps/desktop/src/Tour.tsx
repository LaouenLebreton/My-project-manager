import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { keys } from "./platform";

interface Step {
  /** CSS selector of the thing being explained. */
  target: string;
  title: string;
  text: string;
}

const STEPS: Step[] = [
  {
    target: '[data-tour="name"]',
    title: "Type your tasks",
    text: "Click the blank row at the bottom and type a name, then Return for the next one. Tab indents a task under the one above it.",
  },
  {
    target: '[data-tour="predecessors"]',
    title: "Link tasks",
    text: "Type the row numbers of the tasks that must happen first, like 2 or 3, 5. Dates update straight away.",
  },
  {
    target: '[data-tour="chart"]',
    title: "Find the critical path",
    text: "Red striped bars are critical: if one of them slips, the project finishes later. Blue bars have slack and can slip a little.",
  },
  {
    target: '[data-tour="chart"]',
    title: "Drag to plan",
    text: "Drag a bar to move a task, its right end to change the duration, or drag down onto another bar to link the two.",
  },
  {
    target: '[data-tour="status"]',
    title: "Nothing to save",
    text: keys("Every change is saved as you make it, and ⌘Z undoes anything. Help explains every term whenever you need it."),
  },
];

/** A short guided tour on the sample project (spec 5.1.7). Escape or Skip ends it. */
export function Tour({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const s = STEPS[step];

  useLayoutEffect(() => {
    const el = document.querySelector(s.target);
    setRect(el ? el.getBoundingClientRect() : null);
  }, [s.target]);

  useEffect(() => {
    boxRef.current?.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDone();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [step, onDone]);

  // Below a small target; inside the bottom of a large one (such as the chart), so the box
  // does not cover what it is describing.
  const width = 320;
  const boxHeight = 170;
  const left = rect ? Math.min(window.innerWidth - width - 16, Math.max(16, rect.left + 16)) : 16;
  const top = !rect
    ? 80
    : rect.bottom + boxHeight + 16 < window.innerHeight
      ? rect.bottom + 8
      : Math.max(16, Math.min(rect.bottom, window.innerHeight) - boxHeight - 24);

  return (
    <>
      {rect && <div className="tour-highlight" style={{ left: rect.left - 4, top: rect.top - 4, width: rect.width + 8, height: rect.height + 8 }} />}
      <div
        ref={boxRef}
        className="tour"
        role="dialog"
        aria-labelledby="tour-title"
        aria-describedby="tour-text"
        tabIndex={-1}
        style={{ left, top, width }}
      >
        <p className="secondary small">
          {step + 1} of {STEPS.length}
        </p>
        <h2 id="tour-title">{s.title}</h2>
        <p id="tour-text">{s.text}</p>
        <div className="tour-actions">
          <button onClick={onDone}>Skip</button>
          {step > 0 && <button onClick={() => setStep(step - 1)}>Back</button>}
          <button className="primary" onClick={() => (step + 1 < STEPS.length ? setStep(step + 1) : onDone())}>
            {step + 1 < STEPS.length ? "Next" : "Done"}
          </button>
        </div>
      </div>
    </>
  );
}
