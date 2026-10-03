#!/usr/bin/env bash
set -euo pipefail

# Build a deliberately unsigned macOS app bundle. This script does not invoke
# codesign, notarytool, or any credential-bearing command.

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PROFILE=release
OUTPUT="$ROOT_DIR/target/macos/MeetNote.app"
LICENSE_NOTICES=""
DRY_RUN=0

usage() {
  cat <<'EOF'
Usage: package-macos-app.sh --license-notices PATH [options]

Create or validate an unsigned target/macos/MeetNote.app bundle.

Options:
  --license-notices PATH  Required license-notice input copied into the bundle
  --profile NAME          Cargo profile directory (default: release)
  --output PATH           Bundle output (default: target/macos/MeetNote.app)
  --dry-run               Validate inputs and print the planned layout only
  -h, --help              Show this help
EOF
}

fail() {
  printf 'package-macos-app: %s\n' "$1" >&2
  exit 2
}

while (($# > 0)); do
  case "$1" in
    --license-notices)
      (($# >= 2)) || fail "--license-notices requires a path"
      LICENSE_NOTICES=$2
      shift 2
      ;;
    --profile)
      (($# >= 2)) || fail "--profile requires a name"
      PROFILE=$2
      shift 2
      ;;
    --output)
      (($# >= 2)) || fail "--output requires a path"
      OUTPUT=$2
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

[[ -n "$LICENSE_NOTICES" ]] || fail "--license-notices is required"

BINARY="$ROOT_DIR/target/$PROFILE/meetnote"
LICENSE_NOTICES=$(cd "$(dirname "$LICENSE_NOTICES")" && pwd)/$(basename "$LICENSE_NOTICES")
[[ -x "$BINARY" ]] || fail "missing executable $BINARY; run cargo build --$PROFILE first"
[[ -s "$LICENSE_NOTICES" ]] || fail "license-notices input is missing or empty: $LICENSE_NOTICES"

if [[ "$OUTPUT" != /* ]]; then
  OUTPUT="$ROOT_DIR/$OUTPUT"
fi
OUTPUT=$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$OUTPUT")
TARGET_ROOT=$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$ROOT_DIR/target")
case "$OUTPUT" in
  "$TARGET_ROOT"/*) ;;
  *) fail "output must remain under $ROOT_DIR/target" ;;
esac

APP_NAME=$(awk -F'"' '$1 ~ /^[[:space:]]*name[[:space:]]*=/ { print $2; exit }' "$ROOT_DIR/apps/meetnote/Cargo.toml")
APP_VERSION=$(awk -F'"' '$1 ~ /^[[:space:]]*version[[:space:]]*=/ { print $2; exit }' "$ROOT_DIR/apps/meetnote/Cargo.toml")
[[ "$APP_NAME" == "meetnote" && -n "$APP_VERSION" ]] || fail "apps/meetnote/Cargo.toml lacks stable name/version metadata"

printf 'bundle=%s\n' "$OUTPUT"
printf 'binary=%s\n' "$BINARY"
printf 'license_notices=%s\n' "$LICENSE_NOTICES"
printf 'signed=false\nnotarized=false\n'

if ((DRY_RUN)); then
  printf 'layout=Contents/{Info.plist,MacOS/%s,Resources/LicenseNotices/%s}\n' "$APP_NAME" "$(basename "$LICENSE_NOTICES")"
  exit 0
fi

mkdir -p "$ROOT_DIR/target/macos"
STAGING=$(mktemp -d "$ROOT_DIR/target/macos/.MeetNote.app.XXXXXX")
cleanup() { rm -rf "$STAGING"; }
trap cleanup EXIT

STAGED_APP="$STAGING/MeetNote.app"
mkdir -p "$STAGED_APP/Contents/MacOS" "$STAGED_APP/Contents/Resources/LicenseNotices"
install -m 0755 "$BINARY" "$STAGED_APP/Contents/MacOS/$APP_NAME"
install -m 0644 "$LICENSE_NOTICES" \
  "$STAGED_APP/Contents/Resources/LicenseNotices/$(basename "$LICENSE_NOTICES")"
mkdir -p "$STAGED_APP/Contents/Resources/resources/models"
install -m 0644 "$ROOT_DIR/resources/models/catalog.json" \
  "$STAGED_APP/Contents/Resources/resources/models/catalog.json"

cat > "$STAGED_APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>MeetNote</string>
  <key>CFBundleExecutable</key>
  <string>$APP_NAME</string>
  <key>CFBundleIdentifier</key>
  <string>com.meetnote.app</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>MeetNote</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$APP_VERSION</string>
  <key>CFBundleVersion</key>
  <string>$APP_VERSION</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
</dict>
</plist>
EOF

if command -v plutil >/dev/null 2>&1; then
  plutil -lint "$STAGED_APP/Contents/Info.plist" >/dev/null || fail "generated Info.plist is invalid"
fi
test -x "$STAGED_APP/Contents/MacOS/$APP_NAME"
test -s "$STAGED_APP/Contents/Info.plist"
test -s "$STAGED_APP/Contents/Resources/LicenseNotices/$(basename "$LICENSE_NOTICES")"
test -s "$STAGED_APP/Contents/Resources/resources/models/catalog.json"

rm -rf "$OUTPUT"
mv "$STAGED_APP" "$OUTPUT"
trap - EXIT
printf 'created_unsigned_bundle=%s\n' "$OUTPUT"
