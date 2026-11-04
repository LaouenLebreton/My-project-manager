/** Mac or not: shortcuts are written ⇧⌘Z on a Mac and Ctrl+Shift+Z elsewhere. */
export const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

const NAMES: Record<string, string> = { "⌃": "Ctrl", "⌥": "Alt", "⇧": "Shift", "⌘": "Ctrl" };
const ORDER = ["⌘", "⌃", "⌥", "⇧"];

/**
 * Shortcuts in text, as this computer writes them: "⇧⌘Z" stays on a Mac and becomes
 * "Ctrl+Shift+Z" on Windows; works inside sentences ("Undo (⌘Z)").
 */
export function keys(text: string, mac = isMac): string {
  if (mac) return text;
  return text.replace(/[⌃⌥⇧⌘]+([^\s\-/…]?)/g, (_, key: string) => {
    const mods = ORDER.filter((m) => _.includes(m)).map((m) => NAMES[m]);
    return [...new Set(mods), ...(key ? [key] : [])].join("+");
  });
}
