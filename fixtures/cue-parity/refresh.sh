#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

mode=${1:-refresh}
case "$mode" in
  refresh | --check) ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2
    ;;
esac

for tool in modot jq; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "refresh.sh: $tool not found on PATH" >&2
    exit 1
  fi
done

fixtures=(configdash base16-gtk webapp schema)

# Keep only the roots that the fixture contributes.
# The dump also carries host-specific prelude data and absolute paths.
filter_for() {
  case "$1" in
    configdash) echo '{modules, options, file, dashboard_demo}' ;;
    base16-gtk | webapp) echo '{modules, options, file}' ;;
    schema) echo '{modules, options, file, backup, lint}' ;;
  esac
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for fixture in "${fixtures[@]}"; do
  HOME="$PWD/$fixture/home" modot home config dump |
    jq --sort-keys "$(filter_for "$fixture")" >"$tmp/$fixture.json"
done

if [[ $mode == --check ]]; then
  status=0
  for fixture in "${fixtures[@]}"; do
    if ! diff -u "$fixture/expected.json" "$tmp/$fixture.json"; then
      status=1
    fi
  done
  exit "$status"
fi

for fixture in "${fixtures[@]}"; do
  mv "$tmp/$fixture.json" "$fixture/expected.json"
  echo "wrote $fixture/expected.json"
done
