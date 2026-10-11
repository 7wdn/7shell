# System icons

One mark per system the session list can recognise. All of them share a frame so a row reads as a
list rather than a pile: `viewBox="0 0 1024 1024"`, the glyph scaled to 92% of the box (a wide mark
and a square one then look the same size beside the same text), and every colour replaced by
`currentColor` so an icon takes the colour of the row it sits in.

## Where they came from

Fetched from iconfont.cn (a public icon library) on 2026-10-10. Two libraries, because one covers the
Linux/BSD marks and the other has the two the first one lacks:

| file | iconfont id | name | library |
|---|---|---|---|
| `linux.svg` | 43280137 | linux-tux | 641108 |
| `ubuntu.svg` | 43280131 | linux-ubuntu | 641108 |
| `debian.svg` | 43280046 | linux-debian | 641108 |
| `centos.svg` | 43280025 | linux-centos | 641108 |
| `redhat.svg` | 43280121 | linux-redhat | 641108 |
| `fedora.svg` | 43280042 | linux-fedora | 641108 |
| `alpine.svg` | 43280013 | linux-alpine | 641108 |
| `arch.svg` | 43280018 | linux-archlinux | 641108 |
| `opensuse.svg` | 43280105 | linux-opensuse | 641108 |
| `mint.svg` | 43280068 | linux-linuxmint | 641108 |
| `deepin.svg` | 43280035 | linux-deepin | 641108 |
| `rocky.svg` | 43280143 | linux-rocky_linux | 641108 |
| `alma.svg` | 43280016 | linux-almalinux | 641108 |
| `freebsd.svg` | 43280047 | linux-freebsd | 641108 |
| `macos.svg` | 6807414 | Apple | 140078 |
| `windows.svg` | 6805535 | Windows | 140078 |

The fourteen Linux/BSD marks are one library (641108, a `linux-*` set by one author, drawn in one
batch), which is what makes them one style. That set has no macOS and no Windows mark, so those two
come from library 140078 — the same solid-geometry treatment the rest of the set uses, and the reason
`windows.svg` is the four-pane mark rather than the thin outlined variant in the same library.

`linux.svg` is the generic mark: a host that answers `uname -s` with Linux but whose distribution is
not in this table draws as plain Linux, not as a distribution it might not be.

## Before shipping

iconfont carries a per-icon licence on its own pages and this build does not read it. Bundling these
in a release means checking those terms first; the ids above are what to check. This is the
attribution record CONTRIBUTING asks for, kept beside the files it is about rather than in a list
somewhere else.
