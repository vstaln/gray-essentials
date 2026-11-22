#!/bin/sh
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
sys="$(uname -s)"
case "$sys" in
MINGW* | MSYS* | CYGWIN*) rel="x86_64-pc-windows-msvc/gray-extras.exe" ;;
Darwin*)
    case "$(uname -m)" in
    arm64) rel="aarch64-apple-darwin/gray-extras" ;;
    *) rel="x86_64-apple-darwin/gray-extras" ;;
    esac ;;
*)
    case "$(uname -m)" in
    aarch64 | arm64) rel="aarch64-unknown-linux-musl/gray-extras" ;;
    *) rel="x86_64-unknown-linux-musl/gray-extras" ;;
    esac ;;
esac
if [ -x "$here/$rel" ]; then exec "$here/$rel" "$@"; fi
for cand in "$here/target/release/gray-extras.exe" "$here/target/release/gray-extras" "$here/target/debug/gray-extras.exe" "$here/target/debug/gray-extras"; do
    if [ -x "$cand" ]; then exec "$cand" "$@"; fi
done
echo "gray-extras binary not found in $here" >&2
exit 1
