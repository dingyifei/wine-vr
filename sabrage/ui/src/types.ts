// Shared frontend types for the Sabrage app shell.

/** The state graph the main area switches on: one screen per sidebar nav item,
 * plus "edit" — the Library's add/edit-game form, reached from Library, never the sidebar. */
export type Screen = "about" | "library" | "session" | "doctor" | "logs" | "settings" | "edit";
