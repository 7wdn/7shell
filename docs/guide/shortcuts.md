# Keyboard shortcuts

The keys the application reserves, from one table in the source
(`apps/desktop/src/lib/shortcuts/registry.ts`). A surface reads its combination from that table
rather than spelling it out, so a menu cannot advertise a key the application does not listen for.

| Action | Default | Where it is live |
|---|---|---|
| Upload the files on the clipboard | <kbd>Mod+V</kbd> | Anywhere |
| Show the transfers | <kbd>Mod+Shift+D</kbd> | Anywhere |
| Find in the terminal | <kbd>Mod+F</kbd> | In the terminal |
| Take the row the arrows are on | <kbd>Tab</kbd> | In the command line |
| Open the completion list | <kbd>Alt+/</kbd> | In the command line |

**`Mod` is Ctrl on Windows and Linux, and Command on macOS.** That is the spelling a binding is
stored in, so the same saved binding means the same key on every platform and a settings file that
travels between two machines does not have to be corrected.

Three of those choices are deliberate rather than arbitrary:

- **The transfers panel is on a shifted key** because every plain control combination belongs to the
  terminal — <kbd>Ctrl+J</kbd> is a newline and <kbd>Ctrl+D</kbd> ends its input — and the panel has
  to be openable while somebody is typing in one.
- **<kbd>Alt+/</kbd> triggers completion, not <kbd>Ctrl+Space</kbd>**, because on Windows that
  combination belongs to the input method: a key that switches the IME instead of opening the list
  is a key nobody can use.
- **Matching is exact.** Every modifier a binding does not name has to be up, and every one it names
  has to be down, so <kbd>Ctrl+Shift+F</kbd> is not <kbd>Ctrl+F</kbd>. This is what keeps the
  interface's own combinations from swallowing the ones a shell or an editor means for itself.

## Changing them

Settings → **Shortcuts** lists every action, the key it answers to now, and the surface it belongs
to. A row offers:

- **Change** — the row then says "Press a key…", and the next press that names a key becomes the
  binding. The press belongs to the recording, so a combination the browser would otherwise act on
  is recorded instead of being acted on. <kbd>Escape</kbd> leaves the row as it was.
- **Reset** — puts the default from the table back.
- **Unbind** — the action answers to no key, and the row says "None". It stays unbound rather than
  quietly returning to the default, because "I do not want this key to do that" is a decision.

Two actions in one surface must not answer to the same key: whichever is listened for first would
win, silently. A row that shares its key with another in the same surface says which one it
conflicts with, and compares spellings through the same normalisation the stored binding uses, so a
file edited by hand cannot hide a conflict behind `mod+v`.
