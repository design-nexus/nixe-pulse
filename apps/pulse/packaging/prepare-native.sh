#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
native="$root/target/native/ntgcalls"
sha=a6430d968db2b5c273784ede0b8c32f12a55ba89151d3d217c9396397ea1beb3
mkdir -p "$native"
libsha=c65af3126092a6a496620e6335731ec5a862b4c4e39cfea1248ee7cfa1138534
if [[ ! -f "$native/lib/libntgcalls.so" ]] || ! printf '%s  %s\n' "$libsha" "$native/lib/libntgcalls.so" | sha256sum -c --status; then
  archive="$native/ntgcalls-v3.0.5.zip"
  curl -fL --retry 2 https://github.com/pytgcalls/ntgcalls/releases/download/v3.0.5/ntgcalls.linux-x86_64-shared_libs.zip -o "$archive"
  printf '%s  %s\n' "$sha" "$archive" | sha256sum -c -
  unzip -qo "$archive" -d "$native"
fi

printf '%s  %s\n' "$libsha" "$native/lib/libntgcalls.so" | sha256sum -c --status
