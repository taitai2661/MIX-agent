#!/usr/bin/env bash
# Stage the ZimaOS store source layout expected by the upstream builders.
#
# Both the v1 and v2 builders expect a top-level `Apps/` directory. The
# monorepo keeps the source at `apps/MIX-agent` and cannot commit both spellings
# because that breaks checkout on case-insensitive filesystems (macOS by
# default). Generate the builder layout in a throwaway directory instead.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
stage="${1:-$root/out/store-source}"
if [[ "$stage" != /* ]]; then
  stage="$root/$stage"
fi

rm -rf "$stage"
mkdir -p "$stage/Apps"

cp -R "$root/build" "$stage/build"
cp -R "$root/apps/MIX-agent" "$stage/Apps/MIX-agent"
for file in \
  README.md \
  category-list.json \
  featured-apps.json \
  recommend-list.json \
  store-config.json \
  supported-languages.json
do
  cp "$root/$file" "$stage/$file"
done

printf 'Staged store source at %s\n' "$stage"
