# Interface conventions

Two things about this interface are decided once and worn everywhere: what a table looks like, and
what text can be selected. Both had drifted before they were written down — ten tables in three
treatments of their headings, and a shell where nothing stopped a drag across a toolbar from
selecting its labels. Each rule below says *why*, because a rule that only says what is the one that
gets dropped in a rewrite.

## What can be selected

This is a desktop application, not a document. Text is selected in it to take something away — an
address, a path, a version, a line of output — so the test for any surface is one question: would
somebody paste this somewhere?

1. **The shell is not text.** The window's bar, the session library, the tabs, the strips, the file
   listing, the directory tree, the menus and the drag handles. A drag in those places means moving,
   resizing or scrolling, and a selection smeared over a toolbar is noise the person then has to
   clear. Every one of them offers what it holds another way: a row's menu copies its name, its path
   or its address.
2. **A control is not text, and neither is a name.** A button's label, a heading, a column heading
   and a field's name are what the interface *says*; the reading beside them is what somebody copies.
   A drag that starts on a button otherwise takes its label — and the icon inside it, as a highlighted
   box — which is what this rule was written after seeing. They are unselectable by their own markup
   (`button`, `th`, `dt`, `h1`–`h3`, in `styles/globals.css`) rather than by a class on each of the
   several hundred buttons, so a new button is covered the moment it is written.
3. **Readings are text.** Everything a page shows about a host — numbers, paths, names, command
   lines, environment variables — is what people read *in order to copy from*, and the host pages
   are nothing but that. A table's *cells* are in this rule and its *headings* are in the one above:
   a table copied whole comes out as the values, which is what a person pastes.
4. **A field is always text.** Search boxes, rename fields, the command bar, the file editor. A
   surface that takes typing is a document while the keyboard is in it.
5. **An error is text.** Banners and toasts are quoted into bug reports, so they are selectable, and
   the ones without a copy button are still selectable by hand. The monitor panel refuses selection
   everywhere *except* its failure block, which is the same rule seen from the other side.
6. **The terminal is its own world.** xterm draws to a canvas and keeps its own selection: dragging
   selects, the right-click menu copies, and the DOM takes no part. That is the terminal behaving
   like a terminal, and the rules above do not apply inside its screen.

| Surface | Selectable | Why |
|---|---|---|
| Window bar, window controls | no | a title is not a reading |
| Session library, groups, directory tree | no | rows are pressed; their menus copy the name and the address |
| File listing | no | rows are chosen, often by dragging a rectangle over them; names and paths come from the menu |
| Tabs, the terminal's strip | no | a tab name is something to press |
| Menus | no | a list of verbs |
| Dividers, sidebar handle | no | dragging means resizing; the whole body refuses selection while one is dragged |
| Buttons, headings, column headings, field names | no | a control says what it does; the reading beside it is the text |
| The strip naming the connections of a port | no | it is the heading of the table under it; the row above holds the same values |
| Monitor panel | no | it is an instrument; the address has its own button and the rows have menus. The failure block is the one exception |
| Sidebar search, rename fields, command bar, file editor | yes | text being typed |
| Terminal screen | yes, its own way | xterm's selection and menu, not the DOM's |
| Host pages: system, processes, network | yes | readings, paths and command lines — the surfaces people copy from |
| Settings, dialogs, the home page, the batch panel, the command library | yes | values, and the labels beside them where the row is not a form control |
| Error banners and toasts | yes | an error is quoted somewhere |

Measured in a real window rather than asserted from the markup: a selection over the monitor panel,
the tab strip, the terminal's strip, a table's headings or the connections strip contains
**nothing**; the same selection over the body of any of those tables contains the readings and only
the readings.

`scripts/tests/selection.test.mjs` keeps both halves honest — every shell component still refuses
selection, and the surfaces that hold readings never do — and the elements a component draws for
itself are asserted in that component's own spec, where they can be named (`WorkspaceTabs.spec.ts`,
`ContextMenu.spec.ts`).

## Tables

Every table is drawn from `apps/desktop/src/lib/table.ts` — two surfaces, `page` and `panel`, and one
function that says what a row is — and no table writes its own headings, cells or hover bands.
`scripts/tests/tables.test.mjs` is what makes that true rather than aspirational, and the file itself
states the four rules it enforces.
