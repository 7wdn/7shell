#!/bin/bash
# Runs inside the Debian 10 build image: assembles the portable browser package.
#
#   SRC        the checkout, mounted at /src
#   WEB_BUILD  the work directory the caller prepared: target/ (release build) and legacy-dist/
#   VERSION    the version being packaged, without the leading v
#
# The package targets systems whose glibc stops at 2.28: the binaries are built by this image's
# toolchain, every library they need beyond libc is bundled, and the two things that make the
# package unusable on those systems — a newer GLIBC symbol, a dynamic OpenSSL — fail the build here
# rather than on somebody's machine.
set -euo pipefail

SRC="${SRC:-/src}"
WORK="${WEB_BUILD:?the work directory is required}"
VERSION="${VERSION:?the version being packaged is required}"
HERE="$(cd "$(dirname "$0")" && pwd)"

OUT="$WORK/pkg/7Shell-web-linux-amd64"
TAR="$WORK/7Shell_${VERSION}_amd64-web.tar.gz"

test -x "$WORK/target/release/sevenshell-web" || { echo "error: sevenshell-web was not built" >&2; exit 1; }
test -x "$WORK/target/release/sshell-mcp" || { echo "error: sshell-mcp was not built" >&2; exit 1; }
test -f "$WORK/legacy-dist/index.html" || { echo "error: legacy-dist was not prepared" >&2; exit 1; }

rm -rf "$OUT" "$TAR"
mkdir -p "$OUT/bin" "$OUT/lib" "$OUT/dist" "$OUT/data"

cp "$WORK/target/release/sevenshell-web" "$OUT/bin/"
cp "$WORK/target/release/sshell-mcp" "$OUT/bin/sshell-mcp.bin"
strip "$OUT/bin/sevenshell-web"
strip "$OUT/bin/sshell-mcp.bin"
cp "$HERE/sshell-mcp" "$OUT/bin/sshell-mcp"
cp -a "$WORK/legacy-dist/." "$OUT/dist/"
cp "$HERE/7shell" "$OUT/7shell"
cp "$HERE/7shell.desktop" "$OUT/7shell.desktop"
cp "$HERE/README.txt" "$OUT/README.txt"
chmod 755 "$OUT/7shell" "$OUT/7shell.desktop" "$OUT/bin/sevenshell-web" "$OUT/bin/sshell-mcp" "$OUT/bin/sshell-mcp.bin"
chmod 644 "$OUT/README.txt"

# The CSS was rewritten after Vite named it, so its filename no longer matches its contents.
# Give it a content hash again; a browser that cached the old name must ask for a new URL.
CSS_NAME="$(basename "$(ls "$OUT/dist/assets"/index-*.css | head -n 1)")"
CSS_HASH="$(sha256sum "$OUT/dist/assets/$CSS_NAME" | cut -c1-8)"
CSS_NEW="index-legacy-${CSS_HASH}.css"
mv "$OUT/dist/assets/$CSS_NAME" "$OUT/dist/assets/$CSS_NEW"
sed -i "s/$CSS_NAME/$CSS_NEW/g" "$OUT/dist/index.html"

{
  echo "7Shell web package"
  echo "version: $VERSION"
  echo "built: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "css: assets/$CSS_NEW"
  echo "bridge: bin/sshell-mcp (bin/sshell-mcp.bin)"
  echo "glibc ceiling: 2.28"
} > "$OUT/BUILD.txt"
chmod 644 "$OUT/BUILD.txt"
grep -o "$CSS_NEW" "$OUT/dist/index.html" | wc -l

# Everything the binaries need beyond the glibc/loader that every supported system has.
for binary in "$OUT/bin/sevenshell-web" "$OUT/bin/sshell-mcp.bin"; do
  ldd "$binary" | awk '/=> \// {print $3}' | while read -r lib; do
    base="$(basename "$lib")"
    case "$base" in
      libc.so.6|libm.so.6|libdl.so.2|libpthread.so.0|librt.so.1|ld-linux-x86-64.so.2|libresolv.so.2) continue ;;
    esac
    cp -L "$lib" "$OUT/lib/$base"
  done
done

echo "== bundled libraries =="
ls "$OUT/lib"

# The two ways this package becomes unusable on the systems it exists for, checked rather than
# assumed: a symbol newer than the oldest glibc, and an OpenSSL the host may not have.
echo "== glibc ceiling of the binaries =="
for binary in "$OUT/bin/sevenshell-web" "$OUT/bin/sshell-mcp.bin"; do
  ceiling="$(objdump -T "$binary" | grep -o 'GLIBC_[0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -n 1)"
  newest="$(printf '%s\n2.28\n' "$ceiling" | sort -V | tail -n 1)"
  if [ "$newest" != "2.28" ]; then
    echo "error: $(basename "$binary") needs GLIBC $ceiling, above the 2.28 baseline" >&2
    exit 1
  fi
  if objdump -p "$binary" | grep -qE 'NEEDED +lib(ssl|crypto)'; then
    echo "error: $(basename "$binary") links OpenSSL dynamically; it has to be vendored" >&2
    exit 1
  fi
  echo "$(basename "$binary"): GLIBC $ceiling"
done

echo "== package size =="
du -sh "$OUT"
echo "== tar =="
cd "$WORK/pkg"
tar --owner=0 --group=0 -czf "$TAR" 7Shell-web-linux-amd64
sha256sum "$TAR"