import { useEffect, useRef } from "react";
import type { ReportLineView } from "./bindings/ReportLineView";
import type { ReportView } from "./bindings/ReportView";

/** What an import did with the file: nothing is dropped without being listed (spec 4.8). */
export function ImportReport({ report, onClose }: { report: ReportView; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  const clean = report.approximated.length === 0 && report.ignored.length === 0;
  return (
    <dialog ref={ref} className="report" onClose={onClose} aria-labelledby="report-title">
      <h2 id="report-title">Imported “{report.source}”</h2>
      {clean && <p>Everything in the file was imported.</p>}
      <Section title="Imported" lines={report.imported} />
      <Section
        title="Changed to fit Super PM"
        hint="These came in, but not exactly as they were in Microsoft Project."
        lines={report.approximated}
      />
      <Section title="Not imported" hint="Super PM cannot hold these yet. The original file is unchanged." lines={report.ignored} />
      {report.recalculated.length > 0 && (
        <p className="secondary small">
          Dates, slack and the critical path were recalculated by Super PM rather than copied from the file (
          {report.recalculated.join(", ")}).
        </p>
      )}
      <form method="dialog" className="report-actions">
        <button className="primary" autoFocus>
          OK
        </button>
      </form>
    </dialog>
  );
}

function Section({ title, hint, lines }: { title: string; hint?: string; lines: ReportLineView[] }) {
  if (lines.length === 0) return null;
  return (
    <section>
      <h3>{title}</h3>
      {hint && <p className="secondary small">{hint}</p>}
      <ul>
        {lines.map((l) => (
          <li key={l.what}>
            {l.what}
            {l.count > 1 && <span className="secondary"> · {l.count}</span>}
            {l.examples.length > 0 && <span className="secondary small"> (for example: {l.examples.join(", ")})</span>}
          </li>
        ))}
      </ul>
    </section>
  );
}
