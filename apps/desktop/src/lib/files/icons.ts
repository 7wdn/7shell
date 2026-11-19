/**
 * The drawing beside a name in the file listing.
 *
 * A row is read by shape and colour before its name is read, which is what a file manager's icons
 * are for: the same picture for every archive, a different one for a document, a picture for the
 * files whose whole name means something. The drawings are Material Icon Theme's, MIT licensed;
 * `assets/file-icons/material/README.md` records where they came from.
 *
 * The mapping is by extension first and then by whole name, because `README` and `Dockerfile` are
 * file types of their own without an extension. A name this table does not know keeps the generic
 * stroke glyph the rest of the interface uses, which is an honest answer rather than a guess.
 */
import type { SftpEntry } from "@/types/session";

import { extensionOf } from "./format";

const FILES = import.meta.glob("../../assets/file-icons/material/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Readonly<Record<string, string>>;

/** The drawings by the name each was loaded under, with the parts a row cannot use taken off. */
const MARKUP: Readonly<Record<string, string>> = Object.fromEntries(
  Object.entries(FILES).flatMap(([path, raw]) => {
    const markup = rowMarkup(raw);
    return markup ? [[path.slice(path.lastIndexOf("/") + 1, -".svg".length), markup]] : [];
  }),
);

/** Extensions that name a kind of file. Keys are lower case and carry no dot. */
const EXTENSION_ICONS: Readonly<Record<string, string>> = {
  png: "image",
  jpg: "image",
  jpeg: "image",
  gif: "image",
  webp: "image",
  svg: "image",
  bmp: "image",
  ico: "image",
  heic: "image",
  avif: "image",
  tif: "image",
  tiff: "image",

  pdf: "pdf",
  txt: "document",
  rtf: "document",
  doc: "document",
  docx: "document",
  odt: "document",
  rst: "document",
  tex: "document",
  md: "markdown",
  markdown: "markdown",
  log: "log",

  sh: "console",
  bash: "console",
  zsh: "console",
  fish: "console",
  bat: "console",
  cmd: "console",
  ps1: "powershell",

  json: "json",
  jsonc: "json",
  json5: "json",
  ndjson: "json",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  xml: "xml",
  plist: "xml",
  ini: "settings",
  conf: "settings",
  cfg: "settings",
  env: "settings",
  properties: "settings",
  lock: "lock",

  csv: "table",
  tsv: "table",
  xls: "table",
  xlsx: "table",
  ods: "table",

  html: "html",
  htm: "html",
  xhtml: "html",
  css: "css",
  scss: "sass",
  sass: "sass",
  less: "css",
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  ts: "typescript",
  jsx: "react",
  tsx: "react",
  vue: "vue",
  py: "python",
  go: "go",
  rs: "rust",
  java: "java",
  c: "c",
  h: "c",
  cc: "cpp",
  cpp: "cpp",
  hpp: "cpp",
  cs: "csharp",
  php: "php",
  rb: "ruby",
  swift: "swift",
  lua: "lua",
  gradle: "gradle",
  sql: "database",
  db: "database",
  sqlite: "database",

  zip: "zip",
  tar: "zip",
  gz: "zip",
  tgz: "zip",
  bz2: "zip",
  xz: "zip",
  zst: "zip",
  "7z": "zip",
  rar: "zip",
  jar: "zip",
  war: "zip",
  deb: "zip",
  rpm: "zip",
  dmg: "zip",
  iso: "zip",

  mp3: "audio",
  wav: "audio",
  flac: "audio",
  aac: "audio",
  ogg: "audio",
  m4a: "audio",
  opus: "audio",
  mp4: "video",
  mov: "video",
  mkv: "video",
  avi: "video",
  webm: "video",
  m4v: "video",
  flv: "video",

  pem: "certificate",
  crt: "certificate",
  cer: "certificate",
  csr: "certificate",
  p12: "certificate",
  pfx: "certificate",
  key: "key",
  pub: "key",
  gpg: "key",
  asc: "key",
  p8: "key",
  exe: "exe",
  msi: "exe",
  app: "exe",
};

/** Names that say what they are without an extension, or say it better than their extension does. */
const NAME_ICONS: Readonly<Record<string, string>> = {
  dockerfile: "docker",
  "docker-compose.yml": "docker",
  "docker-compose.yaml": "docker",
  license: "license",
  licence: "license",
  copying: "license",
  readme: "readme",
  "readme.md": "readme",
  "readme.txt": "readme",
  "package.json": "nodejs",
  "package-lock.json": "nodejs",
  "nginx.conf": "nginx",
  makefile: "settings",
  ".gitignore": "git",
  ".gitattributes": "git",
  ".gitmodules": "git",
  ".gitconfig": "git",
};

/** The drawing for one directory entry, or nothing when the generic glyph is the right answer. */
export function fileIconMarkup(name: string, kind: SftpEntry["kind"]): string | null {
  if (kind === "symlink" || kind === "other") {
    return null;
  }

  const lower = name.toLowerCase();
  if (kind === "directory") {
    // A bundle is a folder on disk but one thing to the person: a program.
    return lower.endsWith(".app") ? (MARKUP.exe ?? null) : null;
  }

  const byName = NAME_ICONS[lower];
  if (byName) {
    return MARKUP[byName] ?? null;
  }

  const extension = extensionOf(name);
  if (extension !== null) {
    const byExtension = EXTENSION_ICONS[extension];
    if (byExtension) {
      return MARKUP[byExtension] ?? null;
    }
  }

  // A dotfile is configuration, which is what the drawing for a settings file says.
  return lower.startsWith(".") ? (MARKUP.settings ?? null) : null;
}

/** Every drawing that is actually present, for the test that keeps the table honest. */
export const AVAILABLE_FILE_ICONS: readonly string[] = Object.keys(MARKUP).sort();

/** Every drawing the two tables name, so a missing asset is a failed test rather than a blank row. */
export const USED_FILE_ICONS: readonly string[] = [
  ...new Set([...Object.values(EXTENSION_ICONS), ...Object.values(NAME_ICONS)]),
].sort();

/** The `<svg>` element out of a copied file, with the parts a row cannot use taken off it. */
function rowMarkup(raw: string): string | null {
  const start = raw.indexOf("<svg");
  const end = raw.lastIndexOf("</svg>");
  if (start < 0 || end < start) {
    return null;
  }

  const openEnd = raw.indexOf(">", start);
  const opening = raw
    .slice(start, openEnd + 1)
    .replace(/\s(?:width|height|class|style|t|p-id)="[^"]*"/g, "")
    .replace(/^<svg/, '<svg aria-hidden="true" focusable="false"');

  return `${opening}${raw.slice(openEnd + 1, end)}</svg>`;
}
