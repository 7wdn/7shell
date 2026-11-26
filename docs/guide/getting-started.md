# Getting started

This page follows a first run in the order the application asks its questions: the vault, a saved
session, the host key, the terminal, the files, and where the settings live.

## 1. Create the vault

The first start opens on **Create the vault**. There is no vault on the device yet, and the
application will not save a session until there is one.

- The vault password protects every credential saved on this device.
- It is **not stored anywhere and cannot be recovered**. If it is lost, the saved sessions and keys
  are lost with it. Minimum length is eight characters, and the confirmation field has to match.
- **Create vault** writes the file and unlocks it.

Later starts open on **Unlock the vault**, which asks for the same password. Under the form are two
quiet ways out for a password that is gone, and neither of them deletes anything:

- **Forgot the password? Archive and reset** asks once, then renames the file beside itself
  (`vault.json.old`, then `.2.old`, and so on) and leaves the create form on screen. Renaming the file
  back restores it.
- **Switch or manage archived vaults** lists those renamed files with when they were written and how
  big they are. One can be tried with a password, made the live vault again, or read into the vault
  that is open — and the folder they live in opens from the same dialog.

![The create-the-vault screen: a password field, a confirmation field and the Create vault button](/screenshots/create-vault.png)

_The vault files it will write live in one configuration directory — the paths are in the [FAQ](faq.md)._

## 2. Save a session

The session library is the column on the left. **New session** opens the editor:

| Field          | Notes                                                         |
| -------------- | ------------------------------------------------------------- |
| Name           | What the row and the tab will read                            |
| Host           | Name or address of the machine                                |
| Port           | 22 unless the server says otherwise                           |
| User           | The account to log in as                                      |
| Group          | Optional; groups are folders in the library and can be pinned |
| Authentication | Password, Public key, Keyboard interactive or SSH agent       |

Pageant is listed by the editor on Windows and is not selectable, because this build does not
implement it yet.

For **Public key**, **Choose a key file…** opens a native file dialog. The path never reaches the
interface: what is kept is the file's name, its algorithm and its fingerprint, and the key material
goes into the vault. A key with a passphrase is read together with it.

**Save** writes the session into the vault. Nothing is written to disk in the clear, and no wire
type has a field a secret could sit in.

A session in the library can be searched, filtered by authentication method, renamed, moved between
groups, and exported or imported as a file (`Import / Export` in the library's header). An export
with a passphrase carries the stored credentials sealed; an export without one carries none of them.

## 3. Answer the host key question

Opening a session connects in two stages. The first is a probe that sends nothing at all — no user
name, no password, no key — and compares what it received with what this device trusts for that host
and port.

**The first connection** stops on **A host key that has not been seen before**. It shows the host,
the key type and the fingerprint presented now, and nothing has been sent to that server yet.
Compare the fingerprint with what the server's own administrator can tell you, then **Trust this key
and connect**. Trusting records the key in the vault and connects. **Cancel** leaves nothing
half-open.

**A key that has changed** stops harder, on purpose: **The server is presenting a different host key**
shows both the fingerprint presented now and the one trusted before, and the only way forward is
**Forget the old key and trust this one** — which removes the key you trusted before, cannot be
undone, and is, as the screen itself says, the choice an attacker would like you to make. Take it
only when you know why the key changed: the machine was rebuilt or reinstalled, or the server was
given a new key on purpose. Otherwise cancel and find out first.

![The host key prompt: the key type, the fingerprint presented now, and the buttons to trust or cancel](/screenshots/host-key-prompt.png)

_An unknown key is a decision for a person. A changed key is not a question._

## 4. Work in the terminal

Trusting the key opens the terminal in a tab of its own. What is worth knowing:

- Every terminal is one session, and there can be several at once. Each has its own scrollback,
  font and colour palette (Settings → SSH terminal), and scrollback survives switching tabs.
- Opening a session from its own menu when it is already open asks whether to open another tab, and
  the second tab of a session is numbered (`lab`, `lab (1)`, `lab (2)`). **Remember my choice** in
  that question turns the answer into a setting (Settings → Workspace). Clicking the session row
  itself never asks and never opens a second tab: it goes to the tab that is open, or opens the
  session when there is none.
- The window can be resized and the terminal follows; the shell is told the new size.
- **Find in terminal** (`Mod+F`, which is Ctrl on Windows and Linux and Command on macOS) opens the
  search box above the terminal.
- A terminal that ends stays on screen with what it printed and the server's own reason, and offers
  **Reconnect** rather than disappearing.
- If the session has local port forwards, they come up with the terminal and are listed in the
  **Tunnels** strip. A forward that could not listen — a port already taken, most often — is
  reported beside the ones that did come up, and each forward counts the connections it carried.
- The host monitor panel, when it is switched on, samples load, memory, network and file systems
  over this same connection, about every two seconds, and stops when it is hidden.

### Terminal panes

**Split right** and **Split down**, on a terminal's strip or its tab menu, open another terminal
for the same session in the current tab. Split any pane again to combine rows and columns, and
drag the dividers to change their proportions. Dividers also accept the arrow keys, Home, End and
a double click to restore equal sizes.

**Choose a split terminal…** offers two sources:

- **New terminal** opens another terminal for the selected saved session. SSH still verifies the
  host key and authenticates before a pane appears.
- **Move an open terminal** moves an existing terminal into the split. Its running commands,
  scrollback and file panel stay with it; moving does not reconnect it.

Click a pane to make it active. The command input, command library, assistant context and host
monitor use that pane's terminal. Typing in one pane does not broadcast to the others.

The pane header offers **Maximize pane** / **Restore pane layout**, **Move pane to its own tab**
and **Close pane**. A small pane temporarily hides a file or command panel that cannot fit;
maximizing restores it without closing its browser or editor. Closing a pane leaves the others
running; closing the workspace tab closes its panes, keeping any with unsaved files open until their
editor is saved or closed. A terminal that ends keeps its output and
its own reconnect action.

The tab strip's **New tab** button still opens a separate tab. Layouts belong to the current
application window. Reconnecting a terminal starts a new shell; keeping remote programs alive
across disconnections or sharing one shell between people requires a remote persistent session.

**Broadcast input** on a split tab lets you send the same keyboard input, paste and command input
to selected panes. Choose at least two connected panes, then press **Start broadcasting**. The
bar names the targets, and a broadcast icon marks each participating pane. Input in an unselected
pane stays in that pane. Existing text at the remote prompts is not synchronized by enabling it.

Press **Stop broadcasting** to return to independent input. Editing the selection, leaving the tab
or losing a selected connection stops broadcasting; new and reconnected panes are not automatically
enrolled. A maximized pane still broadcasts to the named targets. Output remains separate, and a
command sent from the command input bar keeps the existing approval for risky multi-terminal sends.
File-panel directory actions and the command library keep their explicit targets.

## 5. Open the files

**Open the file panel under this terminal** on the strip beneath the terminal puts the file browser
in the same tab, following the shell's directory — a `cd` in the terminal takes the panel with it.
The session's own row menu has **File manager**, which opens the browser in a tab of its own instead:
a place to browse from rather than the panel that follows this shell, and it stands beside **Open
terminal** whatever shape a session opens in (Settings → Workspace).

- The layout is a directory tree beside the listing, or the listing alone (Settings → Files).
- **New folder** and **New file** create, **Rename** and **Delete** act on the row under the
  pointer, and **Edit** opens a text file in place — a plain editor with a save action and a question
  before closing unsaved work.
- **Upload** opens a native file picker, a drag-and-drop onto the listing works, and the clipboard
  shortcut (`Mod+V`) uploads the files on the clipboard. Local paths stay in the shell: the
  interface is handed a token, never a path.
- **Download** opens a save dialog, or writes into the download folder once one has been chosen
  (the **Download to** setting in the transfers list). While no folder has been chosen, every
  download asks.
- A whole folder goes over as one packed archive, made on the far side by the host's own `tar` and
  opened into the destination folder on arrival.

### Editing text

The file editor and the Kubernetes manifest editors support keyboard indentation and undo. While
the editor has focus, **Tab** inserts one indentation step and **Shift+Tab** removes one. Selecting
text applies the operation to the selected lines. **Ctrl+Z** undoes it; **Ctrl+Shift+Z** or **Ctrl+Y**
redoes it on Windows and Linux. macOS uses Command for save and undo.

Settings → Files offers the indentation width (1–8 columns, default 4), spaces or Tab characters,
and an optional **Keep indentation on Enter** switch. Kubernetes YAML always uses spaces. Changing
these settings preserves an open draft and its undo history; it does not reformat existing text.

| Key          | File editor                                         | Kubernetes manifest editor  |
| ------------ | --------------------------------------------------- | --------------------------- |
| Ctrl+S       | Save and keep editing                               | Save or create the resource |
| Ctrl+Enter   | Save and close                                      | Save or create the resource |
| Esc          | Cancel; asks before discarding unsaved changes      | Cancel editing              |
| Ctrl+Shift+M | Switch Tab between indentation and focus navigation | Same                        |

Use Command instead of Ctrl for these actions on macOS. The **Tab: indent / Tab: move focus** button
at the bottom of the editor offers the same navigation switch. Read-only or busy editors cannot
write, and a failed save keeps the draft on screen. Text pasted into an editor stays text rather
than triggering a file upload. File saves keep the original line-ending style.

The **Transfers** panel is where every transfer reports itself: its direction, its state (Running,
Finished, Failed, Cancelled, Paused), what it has moved, and — when something went wrong — the
reason, with the error's code beneath it. A row's own button is whichever action it needs next
(hold, continue, or try again); stopping and removing a row live in its menu, along with **Open**
(finished transfers only) and **Show in folder**.

## 6. Where the settings are

Settings is a tab of its own. Its categories, in the order the navigation lists them: Appearance,
SSH terminal, Workspace, Files, Security, Shortcuts, Command library, Suggested words, Sync, About.
The search field looks through every category rather than the one on screen.

- **Appearance** — interface theme (system, dark, light), colour theme, interface scale, the two
  fonts, the layout of the panels and tabs, the corner style, the background image and the tray icon.
- **SSH terminal** — the terminal's colours (follow the interface, always dark, always light),
  font and text size, with a live preview of all of it.
- **Workspace** — whether a session opens as a terminal alone or with the files under it, and the
  host monitor panel.
- **Files** — the file browser's layout (tree and list, or list only), and whether a folder travels
  as one packed archive.
- **Security** — the keys this device holds, and the host keys this device trusts, with the action
  that stops trusting one.
- **Shortcuts** — the keys the application reserves, and how to change them; see
  [Keyboard shortcuts](shortcuts.md).
- **Command library** and **Suggested words** — what the command line offers while a command is
  being typed.
- **Sync** — where a backup of everything this installation knows is sent, and how it is sealed.
- **About** — the build's own facts, and the update check, the one request the application makes on
  its own.

**Apply and close** writes what has changed; leaving the page with unwritten changes asks whether to
write them, drop them, or stay. Most choices take effect as they are made.

Everything the application keeps lives in one configuration directory: `settings.json` holds what is
not secret (theme, language, layout, the font choices), `vault.json` holds credentials, keys and
trusted host keys, and the download folder's record sits beside them. The [FAQ](faq.md) has the
directory for each platform.
