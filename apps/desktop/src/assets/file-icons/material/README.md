# File-type icons

One drawing per file family and per common name, shown in the file browser's name column.

The drawings are from [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme),
MIT licensed, through Iconify. They are copied as published; the browser strips the pixel size the
files carry and sizes them by the row instead. The mapping from a name or extension to one of these
files lives in `apps/desktop/src/lib/files/icons.ts`.

The generic file and symbolic-link rows deliberately keep the stroke glyphs this interface uses
elsewhere: those two do not stand for a file type, and a coloured picture of "some file" would say
more than the row knows. Directories are drawn by `FileTypeIcon` itself as a solid amber folder, so
they are not part of this folder.

See `THIRD_PARTY_NOTICES.md` for the licence text and copyright notice.
