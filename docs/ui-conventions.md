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
   listing, the directory tree, the transfer list, the menus and the drag handles. A drag in those
   places means moving, resizing or scrolling, and a selection smeared over a toolbar is noise the
   person then has to clear. Every one of them offers what it holds another way: a row's menu copies
   its name, its path or its address.
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
   everywhere *except* its failure block, which is the same rule seen from the other side. The
   settings page is the same shape: it is chrome, so it refuses selection as a whole, and the few
   readings it holds — the build's own facts, the keys and hosts this device holds, an error, and the
   two fields somebody types in — carry an exemption of their own.
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
| Dividers, sidebar handle, the edge between two columns | no | dragging means resizing; the whole body refuses selection while one is dragged |
| Buttons, headings, column headings, field names | no | a control says what it does; the reading beside it is the text |
| The strip naming the connections of a port | no | it is the heading of the table under it; the row above holds the same values |
| Monitor panel | no | it is an instrument; the address has its own button and the rows have menus. The failure block is the one exception |
| Transfer list panel | no | rows are pressed and their menus copy what they hold; the panel floats over the listing, so a drag that lands on it was meant for the listing. The failure lines — the banner over the list and a row's own reason — are the exception |
| Sidebar search, rename fields, command bar, file editor | yes | text being typed |
| Terminal screen | yes, its own way | xterm's selection and menu, not the DOM's |
| Host pages: system, processes, network | yes | readings, paths and command lines — the surfaces people copy from |
| The settings page | no | a page of controls: the readings inside it — the build's facts, the keys and hosts, an error, the search field — are exempted one by one |
| Dialogs, the home page, the batch panel, the command library | yes | values, and the labels beside them where the row is not a form control |
| Error banners and toasts | yes | an error is quoted somewhere |

Measured in a real window rather than asserted from the markup: a selection over the monitor panel,
the tab strip, the terminal's strip, a table's headings or the connections strip contains
**nothing**; the same selection over the body of any of those tables contains the readings and only
the readings.

`scripts/tests/selection.test.mjs` keeps both halves honest — every shell component still refuses
selection, and the surfaces that hold readings never do — and the elements a component draws for
itself are asserted in that component's own spec, where they can be named (`WorkspaceTabs.spec.ts`,
`ContextMenu.spec.ts`, `NetworkReport.spec.ts`). Where one file holds both halves, that spec is the
only place the difference is visible: `SettingsPage.spec.ts` names the exemption each reading
carries, because "the settings page refuses selection" and "the search field does not" are one file.

## Type

Four sizes and one weight. Each has a job, and a size that has no job is the one that makes a page
look accidental — a 10px line here, a bold word there, neither of which anybody can give a reason
for. The scale was worked out against the reference implementation's own distribution, whose
settings page is overwhelmingly `text-xs` with `text-sm` for names and one `text-base` title.

| What it is | Size | Weight |
|---|---|---|
| The page's own title — the tab says "Settings", so the page says nothing | `text-base` | medium |
| A name: a row's title, a field's label, a button's label, a key's name | `text-sm` | medium |
| A sentence at reading size: a value, an empty state, an address | `text-sm` | normal |
| A supporting sentence: a summary, a hint, an explanation, a timestamp | `text-xs` | normal |
| Machine text: an error code, a fingerprint, a version, a path | `text-[11px]` `font-mono` | normal |

1. **Nothing is smaller than 11px.** That is the floor for text somebody has to read, and everything
   below it in this interface is a machine's own text — which is monospace, and 11px is where
   monospace stops looking like a smudge.
2. **Nothing is bolder than medium.** A name is medium; a sentence is not bold at all. A page that
   reaches for `font-semibold` to make a point has run out of hierarchy, and the next one reaches
   for it twice.
3. **A category is named once.** The navigation says which one is on screen; the body of the page
   does not repeat it. The exception is a search, where a hit is grouped under the category it came
   from — because there the question being answered is "where is this setting".

The monitor panel is the one place that goes below the floor, on purpose and as a whole: it is an
instrument drawn at 10px with 19px rows, so that a dozen readings fit beside the workspace. It is
exempt because the exception is the panel's whole design rather than a line somebody shrank.

`scripts/tests/typography.test.mjs` holds the first two rules over the settings page, which is where
the drift was.

## Tables

Every table is drawn from `apps/desktop/src/lib/table.ts` — two surfaces, `page` and `panel`, and one
function that says what a row is — and no table writes its own headings, cells or hover bands.
`scripts/tests/tables.test.mjs` is what makes that true rather than aspirational, and the file itself
states the four rules it enforces.

A column's *width* used to be left to the table as well, written as a class on the heading and again
on every cell under it. It is now its own module — `apps/desktop/src/lib/columns.ts` — for the reason
that shape was written after: the two copies drifted sixteen pixels apart in the file listing, and
nobody noticed until the columns were counted. Four rules hold there, and they are worth knowing
before adding a column anywhere:

1. **Every column carries a width and an edge, and the edge is drawn.** The file listing's name
   column and the process table's command line are not exceptions to that: an edge that answered to
   nothing would read as a broken column rather than as a column with a rule of its own. What says
   where an edge is is a hairline down the heading strip at the boundary — always there, thickening
   and taking the accent colour while the pointer is on it or while it is dragged — and it is drawn in
   the heading strip rather than through the rows, because that is where a boundary is acted on: a
   rule down every row is six lines over a list whose rows already carry a rule of their own and a
   band under the pointer. An edge can be dragged, nudged with the arrow keys, taken to the column's
   floor with Home, or double-clicked to put the column back to the width its table states.
2. **A heading is centred over its column; a value keeps its own alignment.** A heading is a label
   rather than a value — it names the column — so it sits in the middle, and the figures under it stay
   right-aligned and the names stay left, which is how each of them is read. Centring the label also
   keeps it clear of the edge at the column's right.
3. **The room a panel has to spare goes to the columns nobody has moved**, divided equally between
   them and handed out to the pixel, so that the columns add up to the line they are drawn on exactly.
   A column somebody has moved takes no part in that division: it is the width it was put at, and the
   others share what is left. A drag therefore reads in both directions — narrowing a column hands its
   room back to the columns that still share, widening one takes from them.
4. **There is no ceiling, and there is a floor.** A column may be dragged as wide as somebody likes;
   what happens when the columns no longer fit is the strip's own scrollbar, which is how a long file
   name or a long command line gets read. The floor is where a column stops saying what it is, and it
   holds whichever way the window is resized — so the line is drawn at least as wide as its columns
   ask for, and a panel too narrow for them scrolls rather than squeezing any of them to nothing.

A width belongs to the run rather than to the installation: nothing is written down, and closing the
window puts every column back to the width its table states, which is what the file managers this
interface is measured against do with theirs.
