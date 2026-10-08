# Decisions

Short notes, newest last. Owner-approved product decisions (D1–D9) are in the spec; these are implementation decisions.

## 2026-10-08

- **Tauri 2.12.1, not 3.0.** Tauri 3 is in alpha. D1 says Tauri 2; stay on 2.x stable.
- **Pinned front-end versions:** React 19.3.0, Vite 8.3.1, @vitejs/plugin-react 6.1.1, TypeScript 5.9.3. Releases from the last two weeks were skipped (Vite 8.3.4 came out on 2026-10-08). TypeScript 7 exists, but 5.9 is the conservative choice for tooling compatibility.
- **Time in the core is `Moment`: whole minutes since 1970-01-01T00:00, naive local.** All calendar arithmetic is integer math. `jiff` civil types appear only at API edges. Date conversion uses Howard Hinnant's algorithms, checked against `jiff` by a property test.
- **Calendar arithmetic walks day by day.** Simple and correct first. If the 5,000-task benchmark shows it matters, add week skipping.
- **Ids are UUID v7 created outside `pm-core`,** so the core needs no clock or random source.
- **Ordered maps (`BTreeMap`) in the model,** for deterministic output (rule R29).
- **No network in the app:** the capability file grants only `core:event:default`, and the CSP allows only `'self'` and Tauri IPC.
- **Placeholder identity:** product name `Super PM`, bundle id `app.superpm.desktop`, placeholder icon in `apps/desktop/app-icon.svg`. See `open-questions.md`.
