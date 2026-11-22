#!/usr/bin/env bash
# ./build.sh       builds ./seat from the working tree (release profile).
# ./build.sh tag   builds the tag on HEAD into dist/, and refuses a working tree with changes.
set -euo pipefail
cd "$(dirname "$0")"

exe=seat
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) exe=seat.exe ;; esac

case "${1:-}" in
"")
  cargo build --release --locked
  cp "target/release/$exe" "./$exe"
  rev=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
  [ -n "$(git status --porcelain 2>/dev/null)" ] && rev="$rev-dirty"
  echo "built ./$exe ($(./$exe --version), dev-$rev)"
  ;;
tag)
  tag=$(git describe --tags --exact-match HEAD 2>/dev/null) || { echo "HEAD has no tag" >&2; exit 1; }
  [ -z "$(git status --porcelain)" ] || { echo "the working tree has changes; commit or stash them first" >&2; exit 1; }
  version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
  [ "$tag" = "v$version" ] || { echo "tag $tag does not match Cargo.toml version $version" >&2; exit 1; }
  cargo build --release --locked
  mkdir -p dist
  cp "target/release/$exe" "dist/$exe"
  echo "built dist/$exe ($tag)"
  ;;
*)
  echo "usage: ./build.sh [tag]" >&2
  exit 2
  ;;
esac
