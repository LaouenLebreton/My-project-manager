import { useEffect, useRef } from "react";
import { GLOSSARY, SHORTCUTS } from "./help";

/** Help > Super PM Help: what the terms mean and every shortcut, no manual needed. */
export function HelpSheet({ onClose, onTour }: { onClose: () => void; onTour: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => ref.current?.showModal(), []);
  return (
    <dialog ref={ref} className="report" onClose={onClose} aria-labelledby="help-title">
      <h2 id="help-title">Super PM Help</h2>
      <section>
        <h3>What these words mean</h3>
        <dl className="glossary">
          {Object.values(GLOSSARY).map((g) => (
            <div key={g.term}>
              <dt>{g.term}</dt>
              <dd>{g.text}</dd>
            </div>
          ))}
        </dl>
      </section>
      <section>
        <h3>Keyboard</h3>
        <table className="shortcuts">
          <tbody>
            {SHORTCUTS.map(([keys, what]) => (
              <tr key={keys}>
                <th scope="row">{keys}</th>
                <td>{what}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <form method="dialog" className="report-actions">
        <button type="button" onClick={onTour}>
          Show Tour
        </button>
        <button className="primary" autoFocus>
          Done
        </button>
      </form>
    </dialog>
  );
}
