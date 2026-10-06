/**
 * The names the menus give themselves.
 *
 * A menu's title is only ever heard — it is the `aria-label` on the layer, and nothing on screen
 * prints it — which is exactly why the titles are gathered here rather than written at each call
 * site: a region passes one of these and nothing else, so every one of them can be asked for in every
 * language from a single test (`labels.spec.ts`).
 */
export const MENU_TITLES = {
  terminal: "menu.terminal",
  file: "menu.file",
  /** For a menu over several selected rows, which has no single name to be about. */
  files: "menu.files",
  directory: "menu.directory",
  transfer: "menu.transfer",
  tab: "menu.tab",
  session: "menu.session",
  group: "menu.group",
  /** The whole session library — exporting it, importing into it — rather than any one row. */
  library: "menu.library",
  tunnel: "menu.tunnel",
  process: "menu.process",
  disk: "menu.disk",
  socket: "menu.socket",
  connection: "menu.connection",
  detail: "menu.detail",
  environment: "menu.environment",
} as const;
