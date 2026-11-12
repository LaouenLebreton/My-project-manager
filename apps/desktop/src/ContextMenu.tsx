import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuItem =
  | {
      label: string;
      run: () => void;
      shortcut?: string;
      /** A tick, for options that are on or off. */
      checked?: boolean;
      disabled?: boolean;
      /** Shown in red: deletes something. */
      danger?: boolean;
    }
  | { separator: true }
  | { heading: string };

type Open = (e: { clientX: number; clientY: number; preventDefault: () => void; stopPropagation: () => void }, items: MenuItem[]) => void;

const MenuContext = createContext<Open>(() => {});

/** Opens a context menu at the pointer: `onContextMenu={(e) => openMenu(e, items)}`. */
export const useContextMenu = () => useContext(MenuContext);

/**
 * Right-click menus for the whole window. Elsewhere, right-clicking does nothing (instead of the
 * web view's own menu with Reload); text fields keep their Cut / Copy / Paste menu.
 */
export function ContextMenuProvider({ children }: { children: React.ReactNode }) {
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const open = useCallback<Open>((e, items) => {
    e.preventDefault();
    e.stopPropagation();
    if (items.length) setMenu({ x: e.clientX, y: e.clientY, items });
  }, []);

  useEffect(() => {
    const onContext = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (t?.closest("input, textarea, [contenteditable=true]")) return;
      e.preventDefault();
    };
    window.addEventListener("contextmenu", onContext);
    return () => window.removeEventListener("contextmenu", onContext);
  }, []);

  return (
    <MenuContext.Provider value={open}>
      {children}
      {menu && <Menu {...menu} onClose={() => setMenu(null)} />}
    </MenuContext.Provider>
  );
}

function Menu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  const runnable = items.flatMap((it, i) => ("run" in it && !it.disabled ? [i] : []));
  const [active, setActive] = useState<number | null>(null);

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({ left: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)), top: Math.max(4, Math.min(y, window.innerHeight - r.height - 4)) });
    el.focus();
  }, [x, y]);

  useEffect(() => {
    const away = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const blur = () => onClose();
    window.addEventListener("mousedown", away, true);
    window.addEventListener("blur", blur);
    window.addEventListener("resize", blur);
    return () => {
      window.removeEventListener("mousedown", away, true);
      window.removeEventListener("blur", blur);
      window.removeEventListener("resize", blur);
    };
  }, [onClose]);

  const choose = (i: number) => {
    const it = items[i];
    if (!("run" in it) || it.disabled) return;
    onClose();
    it.run();
  };

  const onKey = (e: React.KeyboardEvent) => {
    const step = (d: number) => {
      if (!runnable.length) return;
      const at = active === null ? -1 : runnable.indexOf(active);
      setActive(runnable[(at + d + runnable.length) % runnable.length]);
    };
    if (e.key === "Escape") onClose();
    else if (e.key === "ArrowDown") step(1);
    else if (e.key === "ArrowUp") step(-1);
    else if ((e.key === "Enter" || e.key === " ") && active !== null) choose(active);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div ref={ref} className="context-menu" role="menu" tabIndex={-1} style={pos} onKeyDown={onKey} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it, i) =>
        "separator" in it ? (
          <div key={i} className="context-separator" role="separator" />
        ) : "heading" in it ? (
          <div key={i} className="context-heading">
            {it.heading}
          </div>
        ) : (
          <div
            key={i}
            role={it.checked === undefined ? "menuitem" : "menuitemcheckbox"}
            aria-checked={it.checked}
            aria-disabled={it.disabled}
            className={["context-item", i === active ? "active" : "", it.disabled ? "disabled" : "", it.danger ? "danger" : ""].filter(Boolean).join(" ")}
            onMouseEnter={() => setActive(it.disabled ? null : i)}
            onClick={() => choose(i)}
          >
            <span className="context-check" aria-hidden>
              {it.checked ? "✓" : ""}
            </span>
            <span className="context-label">{it.label}</span>
            {it.shortcut && <span className="context-shortcut">{it.shortcut}</span>}
          </div>
        ),
      )}
    </div>
  );
}
