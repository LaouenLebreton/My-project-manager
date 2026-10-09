import { useEffect, useRef, useState } from "react";

/** Edit > Find (⌘F): type part of a task name; Return jumps to the next match. */
export function FindBar({ onFind, onClose }: { onFind: (text: string, fromCurrent?: boolean) => boolean; onClose: () => void }) {
  const [text, setText] = useState("");
  const [missing, setMissing] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.focus(), []);
  return (
    <div className="find-bar" role="search">
      <input
        ref={ref}
        type="search"
        aria-label="Find a task by name"
        placeholder="Find task"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setMissing(e.target.value.trim() !== "" && !onFind(e.target.value, true));
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") setMissing(!onFind(text));
          if (e.key === "Escape") onClose();
        }}
      />
      <span className="secondary small" role="status">
        {missing ? "No task has that in its name" : "Return for the next match"}
      </span>
      <button onClick={onClose}>Done</button>
    </div>
  );
}
