#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WWWROOT="${SCRIPT_DIR}/../my-telemetry-ui/wwwroot"
DX_OUT="${SCRIPT_DIR}/target/dx/my-telemetry-ui-client/release/web/public"

# The UI - and only the UI - is built with Rust 1.97. Since 1.98 the rust-objcopy that
# dx strips the release wasm with can't load libLLVM.dylib on macOS
# (rust-lang/rust#164063), and dx 0.7 doesn't hand it the library path in a way macOS
# reads (fixed in dx 0.8, not in 0.7). 1.98.0 is affected too. Move this forward once
# a stable release carries the Rust fix.
UI_TOOLCHAIN="1.97"

# Installs the toolchain on a Mac that doesn't have it yet; nothing to do otherwise.
if ! rustup target list --installed --toolchain "${UI_TOOLCHAIN}" 2>/dev/null | grep -x wasm32-unknown-unknown >/dev/null; then
    echo ">> rustup toolchain install ${UI_TOOLCHAIN}"
    rustup toolchain install "${UI_TOOLCHAIN}" --profile minimal --target wasm32-unknown-unknown
fi

export RUSTUP_TOOLCHAIN="${UI_TOOLCHAIN}"

LOG="$(mktemp)"
trap 'rm -f "${LOG}"' EXIT

cd "${SCRIPT_DIR}"

# dx does not clean its output directory, it copies over it: a file renamed or deleted
# in public/ would stay there and ride into wwwroot on every later build.
rm -rf "${DX_OUT}"

echo ">> dx build --release --web (Rust ${UI_TOOLCHAIN})"
dx build --release --web 2>&1 | tee "${LOG}"

if [ ! -d "${DX_OUT}" ]; then
    echo "ERROR: build output not found at ${DX_OUT}"
    exit 1
fi

# A failed wasm-opt does not fail dx: it ships the unoptimised module and logs a single
# ERROR line. Say it out loud, before the bundle gets committed.
if grep -q "ERROR wasm-opt failed" "${LOG}"; then
    echo "!! wasm-opt FAILED - the wasm going into wwwroot is UNOPTIMISED"
fi

echo ">> cleaning ${WWWROOT}"
rm -rf "${WWWROOT}"
mkdir -p "${WWWROOT}"

echo ">> copying ${DX_OUT}/. -> ${WWWROOT}/"
cp -R "${DX_OUT}/." "${WWWROOT}/"

# The bundle is out, so target/ goes: a release wasm build leaves gigabytes there and
# nothing reads them again. A failed dx build stops the script above, so a target worth
# looking into is kept.
rm -rf "${SCRIPT_DIR}/target"

echo ">> done: $(find "${WWWROOT}" -type f | wc -l | tr -d ' ') files, $(du -sh "${WWWROOT}" | cut -f1)"
