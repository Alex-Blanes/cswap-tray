#!/bin/sh
# Builds the Windows tray on the NAS (Linux): tests under Wine, then the release
# .exe into dist/. The repo lives here; Windows machines only fetch the binary:
#
#   scp nas:/volume2/claude/proyectos/cswap-tray/dist/cswap-tray.exe $env:TEMP\
#   scp nas:/volume2/claude/proyectos/cswap-tray/scripts/install-autostart.ps1 $env:TEMP\
#   powershell -ExecutionPolicy Bypass -File $env:TEMP\install-autostart.ps1 -ExePath $env:TEMP\cswap-tray.exe
#
# Debian 13 on purpose: Debian 12's Wine 8 lacks bcryptprimitives.dll, which
# current Rust's std imports, so the tests would not even start.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$repo/dist"
docker run --rm \
    -v "$repo:/src:ro" -v "$repo/dist:/dist" -v cswap-tray-cargo:/cache \
    -e CARGO_TARGET_DIR=/cache/target13 -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine -e WINEDEBUG=-all \
    -e OWNER="$(id -u):$(id -g)" -w /src rust:1-trixie sh -c '
set -e
apt-get update -qq >/dev/null
apt-get install -y -qq gcc-mingw-w64-x86-64 wine wine64 >/dev/null 2>&1
rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1
wine wineboot -i >/dev/null 2>&1; wineserver -w 2>/dev/null || true
cargo test --target x86_64-pc-windows-gnu -- --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked" || true
cargo test --target x86_64-pc-windows-gnu -q -- --test-threads=1 >/dev/null 2>&1
cargo build --release --target x86_64-pc-windows-gnu
cp /cache/target13/x86_64-pc-windows-gnu/release/cswap-tray.exe /dist/
chown "$OWNER" /dist/cswap-tray.exe'
ls -l "$repo/dist/cswap-tray.exe"
