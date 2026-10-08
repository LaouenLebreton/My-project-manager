import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

/** Menu item ids sent by the Rust side (see src-tauri/src/menu.rs). */
type MenuId = string;

export function App() {
  useEffect(() => {
    const unlisten = listen<MenuId>("menu", (e) => {
      console.debug("menu", e.payload);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  return (
    <main className="empty">
      <h1>No project open</h1>
      <p>Creating and opening projects arrives in a later step of this prototype.</p>
    </main>
  );
}
