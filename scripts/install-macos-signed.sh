#!/bin/bash
# Explicitly invoked local install: verify, back up, replace, and relaunch.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/src-tauri/target/release/bundle/macos/pi-session-viewer.app"
DEST=/Applications/pi-session-viewer.app
bash "$ROOT/scripts/build-macos-signed.sh" --verify "$APP"
[[ ! -L "$DEST" ]] || { echo "Refusing to replace a symlink: $DEST" >&2; exit 1; }
STAGE_DIR="$(mktemp -d /Applications/.pi-session-viewer-update.XXXXXX)"
trap 'rm -rf "$STAGE_DIR"' EXIT
ditto "$APP" "$STAGE_DIR/pi-session-viewer.app"
bash "$ROOT/scripts/build-macos-signed.sh" --verify "$STAGE_DIR/pi-session-viewer.app"

# Only stop the desktop UI, never pi/rmux sessions or unrelated applications.
# Match an argument boundary too: open --args must not leave the old process
# running against a replaced bundle, or open may reuse it instead of relaunching.
PIDS="$(pgrep -f '^/Applications/pi-session-viewer[.]app/Contents/MacOS/pi-session-viewer([[:space:]]|$)' || true)"
if [[ -n "$PIDS" ]]; then
  echo "Closing pi-session-viewer for the update..."
  for pid in $PIDS; do kill -TERM "$pid"; done
  for attempt in {1..50}; do
    alive=0
    for pid in $PIDS; do kill -0 "$pid" 2>/dev/null && alive=1; done
    [[ "$alive" == 0 ]] && break
    sleep 0.2
  done
  [[ "$alive" == 0 ]] || { echo "App did not exit; installation cancelled (no force kill)." >&2; exit 1; }
fi

BACKUP=""
if [[ -e "$DEST" ]]; then
  BACKUP_DIR="$ROOT/artifacts/builds/stable-signing/backups/$(date +%Y%m%d-%H%M%S)-$$"
  mkdir -p "$BACKUP_DIR"
  BACKUP="$BACKUP_DIR/pi-session-viewer.app"
  mv "$DEST" "$BACKUP"
  echo "Previous app saved at: $BACKUP"
fi
if ! mv "$STAGE_DIR/pi-session-viewer.app" "$DEST"; then
  [[ -z "$BACKUP" ]] || mv "$BACKUP" "$DEST"
  exit 1
fi
if ! bash "$ROOT/scripts/build-macos-signed.sh" --verify "$DEST"; then
  mv "$DEST" "$STAGE_DIR/failed.app"
  [[ -z "$BACKUP" ]] || mv "$BACKUP" "$DEST"
  echo "Installed verification failed; previous app restored." >&2
  exit 1
fi
open "$DEST"
echo "Installed and relaunched $DEST. Session data and TCC settings were not modified."
