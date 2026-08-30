// Shared frontend types for the Sabrage app shell.

/** The small state graph the main area switches on — one screen per sidebar nav item,
 * plus "edit" (Phase 4): the Library's add/edit-game form, reached from Library, never
 * from the sidebar directly. */
export type Screen = "about" | "library" | "session" | "doctor" | "logs" | "settings" | "edit";
