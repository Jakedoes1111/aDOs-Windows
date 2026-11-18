# Owner decisions, 15 Nov 2026 (after v1 sign-off)

1. Hot-reloaded UI: adopt gpui-shell for Home; all use stays in `ados-script`.
2. Notes: a new note asks for its name. Renaming is manual. No rename from the first line, no fixed file names.
3. Assistant toasts: off by default (setting). Undo for assistant changes needs another home, e.g. a palette action.
4. Focus and keys, shared in `ados-core`:
   - While an overlay shows, it takes every key.
   - One filter-with-list component for every filter box.
   - The caret follows the focused window; this can be toggled off.
5. Memory: Ekaj's model rebuilt natively on PostgreSQL, named "Memory", with 2D and 3D graph views. Feeding rules are designed with the owner first.
6. Wanted: comment on a web page element or selected text so the agent gets exactly that element, text or region.
7. Memory stack (from comments on the design page): PostgreSQL downloaded and run by aDOs (`postgresql_embedded`); `petgraph` + `fdg` for layout; GPUI canvas for 2D and 3D. Graph storage (plain tables vs pgGraph now) still open. Feeding rules to be discussed 16 Nov.
