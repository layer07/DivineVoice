#!/usr/bin/env bash
set -euo pipefail

profile=release
if [[ "${1:-}" == "--debug" ]]; then
  profile=debug
elif [[ $# -ne 0 ]]; then
  echo "usage: $0 [--debug]" >&2
  exit 2
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
version=$(awk -F '"' '/^version[[:space:]]*=/{print $2; exit}' "$root/Cargo.toml")

if [[ "$profile" == release ]]; then
  # ONNX Runtime's static CoreML provider is not compatible with thin LTO on
  # the Apple Silicon toolchain. This remains an optimized release build.
  CARGO_PROFILE_RELEASE_LTO=false cargo build --release --no-default-features
else
  cargo build --no-default-features
fi

rm -rf "$app" "$iconset"
mkdir -p "$resources" "$macos"

# sips can convert the repository's ICO directly. A larger source asset will
# improve the result but is not required for an installable app bundle.
sips -s format icns "$root/icons/app.ico" --out "$resources/AppIcon.icns" >/dev/null

cp "$target/divinevoice" "$macos/divinevoice"
cp "$root/packaging/macos/Info.plist" "$contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$contents/Info.plist"

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
  codesign --force --options runtime --timestamp --sign "$MACOS_SIGN_IDENTITY" "$app" >/dev/null
else
  codesign --force --sign - "$app" >/dev/null
fi
codesign --verify --deep --strict "$app"
echo "$app"
