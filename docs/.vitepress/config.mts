import { defineConfig } from "vitepress";

const REPOSITORY = "https://github.com/7wdn/7shell";

/**
 * The pages that live outside the site's source directory.
 *
 * `docs/` is the site root, so a page in it cannot link to `CONTRIBUTING.md` or `SECURITY.md` at the
 * repository root: VitePress refuses to build a link that leaves the root. They are linked by URL
 * instead, against the branch they are edited on.
 */
const contributing = `${REPOSITORY}/blob/main/CONTRIBUTING.md`;
const securityPolicy = `${REPOSITORY}/blob/main/SECURITY.md`;

export default defineConfig({
  title: "7Shell",
  description:
    "An open-source SSH and operations console for Windows, macOS and Linux: an encrypted vault, host keys verified before anything is sent, terminals, a file workbench and a host monitor.",
  // The site is served from a project path on GitHub Pages, so it is not at the domain root. Every
  // link in the markdown is written without this prefix and VitePress adds it — the one place the
  // path appears, which is what keeps a move off GitHub Pages a one-line change.
  base: "/7shell/",
  // No carve-outs from the dead-link check. Two pages used to point at repository files with a
  // relative link (`../SECURITY.md`), which resolves when the file is read on GitHub and never when
  // the site is built; those two links now name the file by URL, so the check has nothing to pardon
  // and the next link that leaves the root fails the build, where somebody has to decide about it.
  // `docs/pips/README.md` is the file the repository reads as that folder's front page, and the URL
  // the navigation offers for proposals is `/pips/`. A folder index is the name a static host serves
  // for that URL, so the build renames the page on its way out — the file itself stays where it is,
  // and its own relative links still resolve, because it does not change folders.
  rewrites: { "pips/README.md": "pips/index.md" },
  themeConfig: {
    nav: [
      {
        text: "Guide",
        items: [
          { text: "Install", link: "/guide/install" },
          { text: "Getting started", link: "/guide/getting-started" },
          { text: "Keyboard shortcuts", link: "/guide/shortcuts" },
          { text: "Troubleshooting", link: "/guide/troubleshooting" },
          { text: "FAQ", link: "/guide/faq" },
        ],
      },
      {
        text: "Reference",
        items: [
          { text: "Architecture", link: "/architecture" },
          { text: "Security model", link: "/security-model" },
          { text: "Descriptors", link: "/descriptors" },
          { text: "Interface conventions", link: "/ui-conventions" },
          { text: "Translation glossary", link: "/i18n-glossary" },
        ],
      },
      {
        text: "Project",
        items: [
          { text: "Release process", link: "/release-process" },
          { text: "Proposals (PIPs)", link: "/pips/" },
          { text: "Contributing", link: contributing },
          { text: "Security policy", link: securityPolicy },
        ],
      },
    ],
    sidebar: [
      {
        text: "Guide",
        items: [
          { text: "Install", link: "/guide/install" },
          { text: "Getting started", link: "/guide/getting-started" },
          { text: "Keyboard shortcuts", link: "/guide/shortcuts" },
          { text: "Troubleshooting", link: "/guide/troubleshooting" },
          { text: "FAQ", link: "/guide/faq" },
        ],
      },
      {
        text: "Reference",
        items: [
          { text: "Architecture", link: "/architecture" },
          { text: "Security model", link: "/security-model" },
          { text: "Descriptors", link: "/descriptors" },
          { text: "Interface conventions", link: "/ui-conventions" },
          { text: "Translation glossary", link: "/i18n-glossary" },
        ],
      },
      {
        text: "Project",
        items: [
          { text: "Release process", link: "/release-process" },
          { text: "Proposals (PIPs)", link: "/pips/" },
          { text: "Contributing", link: contributing },
          { text: "Security policy", link: securityPolicy },
        ],
      },
    ],
    // Local search: the site is static and has no server that could answer a query, so the index is
    // built into the deployment and searched in the browser.
    search: { provider: "local" },
    socialLinks: [{ icon: "github", link: REPOSITORY }],
    // The footer link and the "edit this page" link both name the repository, and both are wrong if
    // the branch they are built against is not the one the site is published from.
    footer: {
      message: "Released under the Apache-2.0 licence.",
      copyright: `Copyright 2026 SevenShell contributors · <a href="${REPOSITORY}">github.com/7wdn/7shell</a>`,
    },
    editLink: {
      pattern: `${REPOSITORY}/edit/main/docs/:path`,
      text: "Edit this page on GitHub",
    },
  },
});
