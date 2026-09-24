#!/bin/sh

set -eu

store_root=/var/lib/casaos/appstore/default
new_root=${store_root}.new
old_root=${store_root}.old

rm -rf "$old_root"
if [ -d "$store_root" ]; then
  mv "$store_root" "$old_root"
fi

if [ ! -d "$new_root" ]; then
  echo "New app store payload not found: $new_root" >&2
  if [ -d "$old_root" ]; then
    mv "$old_root" "$store_root"
  fi
  exit 1
fi

mv "$new_root" "$store_root"
rm -rf "$old_root"
