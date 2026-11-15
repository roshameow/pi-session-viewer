#!/bin/bash
# Maintainer's local update workflow; never creates certificates or changes TCC.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[[ "$(uname -s)" == Darwin ]] || { echo "This workflow requires macOS." >&2; exit 1; }

# Public certificate fingerprint from the existing, previously signed builds.
# Keep the certificate/private key: recreating a same-named certificate changes identity.
CERT=F8E06F1769EDBB5583BE2C874E40256CA6D07617
REQ='identifier "com.pidesktop.viewer" and certificate leaf = H"f8e06f1769edbb5583be2c874e40256ca6d07617"'
HELPER_SHA=194189dc5650b584e7d3742bdf8a4e09bb7ca8172dc9d00b26ed36b6b7769198
APP="$ROOT/src-tauri/target/release/bundle/macos/pi-session-viewer.app"

if [[ "${1:-}" == --verify && "$#" == 2 ]]; then
  APP="$2"
elif [[ "$#" == 0 ]]; then
  [[ -z "${CARGO_TARGET_DIR:-}" ]] || { echo "Unset CARGO_TARGET_DIR for this workflow." >&2; exit 1; }
  # Do not use find-identity -v: existing self-signed local identities may be
  # omitted from that list even though codesign can use their private keys.
  security find-identity -p codesigning | grep -F "$CERT" >/dev/null || {
    echo "Existing signing identity missing. Restore it; do not replace it with an unsigned build." >&2
    exit 1
  }
  export PATH="$HOME/.cargo/bin:$PATH"
  "$ROOT/node_modules/.bin/tauri" build --bundles app --ci \
    --config "{\"bundle\":{\"macOS\":{\"signingIdentity\":\"$CERT\"}}}"
else
  echo "Usage: $0 [--verify /path/to/pi-session-viewer.app]" >&2
  exit 1
fi

HELPER="$APP/Contents/Resources/resources/tab-open-helper"
[[ "$(shasum -a 256 "$HELPER" | awk '{print $1}')" == "$HELPER_SHA" ]] || {
  echo "Bundled tab helper changed; refusing to claim permission continuity." >&2
  exit 1
}
# Verify the helper directly; it lives under Resources rather than nested-code directories.
codesign --verify --strict "$HELPER"
# Check both integrity and continuity with the approved certificate, not merely
# whether a bundle satisfies its own (possibly ad-hoc/CDHash) requirement.
codesign --verify --deep --strict -R="$REQ" "$APP"
DETAILS="$(codesign -d -r- "$APP" 2>&1)"
echo "$DETAILS"
[[ "$DETAILS" == *"designated => $REQ"* ]] || {
  echo "Designated requirement changed; review before replacing the installed app." >&2
  exit 1
}
echo "Verified stable signed app: $APP"
echo "Build/verification only: no app installed, launched, or permissions changed."
