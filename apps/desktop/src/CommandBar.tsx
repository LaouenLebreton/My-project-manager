import { useEffect, useMemo, useRef, useState } from "react";
import { keys } from "./platform";

export interface CommandItem {
  name: string;
  shortcut?: string;
  run: () => void;
}

/** ⌘K: every command by name (spec 5.1.2). Type to filter, ↑↓ to choose, Return to run. */
export function CommandBar({ commands, onClose }: { commands: CommandItem[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const matches = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return commands.filter((c) => words.every((w) => c.name.toLowerCase().includes(w)));
  }, [commands, query]);

  useEffect(() => inputRef.current?.focus(), []);
  useEffect(() => setIndex(0), [query]);

  const run = (c: CommandItem | undefined) => {
    if (!c) return;
    onClose();
    c.run();
  };

  return (
    <div className="command-backdrop" onMouseDown={onClose}>
      <div className="command-bar" role="dialog" aria-label="Command bar" onMouseDown={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          className="command-input"
          placeholder="Type a command"
          aria-label="Command"
          aria-controls="command-list"
          aria-activedescendant={matches[index] ? `command-${index}` : undefined}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") setIndex((i) => Math.min(matches.length - 1, i + 1));
            else if (e.key === "ArrowUp") setIndex((i) => Math.max(0, i - 1));
            else if (e.key === "Enter") run(matches[index]);
            else if (e.key === "Escape") onClose();
            else return;
            e.preventDefault();
          }}
        />
        <ul id="command-list" role="listbox" aria-label="Commands">
          {matches.map((c, i) => (
            <li
              key={c.name}
              id={`command-${i}`}
              role="option"
              aria-selected={i === index}
              className={i === index ? "selected" : undefined}
              onMouseEnter={() => setIndex(i)}
              onClick={() => run(c)}
            >
              <span>{c.name}</span>
              {c.shortcut && <kbd>{keys(c.shortcut)}</kbd>}
            </li>
          ))}
          {matches.length === 0 && <li className="secondary">No command matches “{query}”</li>}
        </ul>
      </div>
    </div>
  );
}
