#!/usr/bin/env bash
set -euo pipefail

profile=release
if [[ "${1:-}" == "--debug" ]]; then
  profile=debug
fi

case "$(uname -s)" in
  Darwin) ;;
  *) echo "macOS packaging must run on macOS" >&2; exit 1 ;;
esac

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
target="$root/target/$profile"
app="$target/DivineVoice.app"
contents="$app/Contents"
resources="$contents/Resources"
macos="$contents/MacOS"
iconset="$target/AppIcon.iconset"

if [[ "$profile" == release ]]; then
  cargo build --release --no-default-features
else
  cargo build --no-default-features
fi

rm -rf "$app" "$iconset"
mkdir -p "$resources" "$macos" "$iconset"

# The repository supplies a 256 px ICO. macOS accepts the generated ICNS for
# local testing; replace it with a 1024 px source before a public release.
sips -s format png "$root/icons/app.ico" --out "$iconset/icon_256x256.png" >/dev/null
sips -z 16 16   "$iconset/icon_256x256.png" --out "$iconset/icon_16x16.png" >/dev/null
sips -z 32 32   "$iconset/icon_256x256.png" --out "$iconset/icon_16x16@2x.png" >/dev/null
sips -z 32 32   "$iconset/icon_256x256.png" --out "$iconset/icon_32x32.png" >/dev/null
sips -z 64 64   "$iconset/icon_256x256.png" --out "$iconset/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$iconset/icon_256x256.png" --out "$iconset/icon_128x128.png" >/dev/null
sips -z 256 256 "$iconset/icon_256x256.png" --out "$iconset/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$iconset/icon_256x256.png" --out "$iconset/icon_256x256@2x.png" >/dev/null
iconutil -c icns "$iconset" -o "$resources/AppIcon.icns"

cp "$target/divinevoice" "$macos/divinevoice"
cp "$root/packaging/macos/Info.plist" "$contents/Info.plist"

codesign --force --sign - "$app" >/dev/null
echo "$app"
