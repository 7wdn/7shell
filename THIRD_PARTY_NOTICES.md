# Third-party notices

7Shell itself is licensed under the Apache License 2.0 (see `LICENSE`). The build also includes
third-party components licensed under the MIT License, the Apache License 2.0, or a compatible
permissive license. Every such component is recorded here when it is introduced — with its name, its
license and, where the license asks for it, its copyright notice.

## Source code derived from MIT-licensed work

Two files in this repository contain code derived from an MIT-licensed terminal-snapshot module by
jinpy.he: a set of pure functions that truncate and redact terminal output before it is handed to a
model, and that module's test suite. The derivation is a port rather than a copy: the functions were
rewritten in this project's TypeScript style, renamed, re-documented for the constraints this build
has, and extended — but the redaction rules, their order, and the shape of the pipeline come from the
original.

| File here | What it derives from | How it was changed |
|---|---|---|
| `apps/desktop/src/lib/assistant/terminalContext.ts` | The original module's tail-truncation, redaction, snapshot and escape-stripping functions, and its redaction rule table | Renamed to the names this project uses; comments rewritten for this codebase; added `prepareTerminalContext` (the whole pipeline plus the request's character budget) and `countLines`; no product-, plan- or issue-specific text was carried over |
| `apps/desktop/src/lib/assistant/terminalContext.spec.ts` | The original module's test suite | Test names rewritten as statements; added cases for the request budget, the line bound, the idempotence of redaction, and a short option that must not be redacted; the assertions on the original behaviour are kept |

Copyright (c) 2026 jinpy.he

### MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial
portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

This file is the inclusion the license asks for: the copyright notice above and this permission
notice travel with every copy of the derived files, and both derived files name it in their header.

## Everything else

No other third-party source has been copied into this repository: everything else the build depends
on is pulled in as a package and is listed in `Cargo.lock` and `pnpm-lock.yaml`, where each package
names its own license.
