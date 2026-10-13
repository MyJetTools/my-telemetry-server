#!/usr/bin/env bash
# Builds the wasm client (../my-telemetry-ui-client) and copies it into wwwroot/, which
# is committed and baked into the image by the Dockerfile. Run it after changing anything
# under my-telemetry-ui-client/ or my-telemetry-ui-shared/, then commit wwwroot/. Just a
# convenience wrapper so you don't have to cd into the client.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec "${SCRIPT_DIR}/../my-telemetry-ui-client/build.sh"
