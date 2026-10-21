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

# TDLib is required at runtime; fresh binary installs must not rely on another app.
tdlib="$root/target/native/tdlib"
mkdir -p "$tdlib"
td_sha=564c69f81a7537f2857e94a53cdde363d9e51a2fca48d26b365b8d9bb9ad637d
lib_sha=47ad24192a3c84d0be783c7c07adb14af0d2f3f53b5830d4660f58c51f298197
if [[ ! -f "$tdlib/libtdjson.so" ]] || ! printf '%s  %s\n' "$lib_sha" "$tdlib/libtdjson.so" | sha256sum -c --status; then
  curl -fL --retry 2 https://registry.npmjs.org/@prebuilt-tdlib/linux-x64-glibc/-/linux-x64-glibc-0.1008067.0.tgz -o "$tdlib/source.tgz"
  printf '%s  %s\n' "$td_sha" "$tdlib/source.tgz" | sha256sum -c -
  tar -xzf "$tdlib/source.tgz" -C "$tdlib" --strip-components=1 package/libtdjson.so package/LICENSE package/package.json
fi
printf '%s  %s\n' "$lib_sha" "$tdlib/libtdjson.so" | sha256sum -c --status
