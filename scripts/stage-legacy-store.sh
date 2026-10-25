#!/usr/bin/env bash
# Stage the legacy ZimaOS/CasaOS v1 store source.
#
# The v1 builder expects a top-level `Apps/` directory, while the v2 store and
# the rest of the monorepo use `apps/`. Keeping both in Git makes the repository
# impossible to check out on case-insensitive filesystems (macOS by default).
# Generate the v1 layout in a throwaway directory instead.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
stage="${1:-$root/out/legacy-store}"
if [[ "$stage" != /* ]]; then
  stage="$root/$stage"
fi

rm -rf "$stage"
mkdir -p "$stage/Apps"

cp -R "$root/build" "$stage/build"
cp -R "$root/apps/MIX-agent" "$stage/Apps/MIX-agent"
for file in category-list.json featured-apps.json recommend-list.json README.md; do
  cp "$root/$file" "$stage/$file"
done

printf 'Staged legacy v1 store source at %s\n' "$stage"
