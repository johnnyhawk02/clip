#!/bin/sh
# Build the .app, add the microphone permission text macOS needs for recording,
# sign it ad hoc and copy it to /Applications.
set -e
cd "$(dirname "$0")/.."
cargo bundle --release
APP="target/release/bundle/osx/Sound Edit.app"
PLIST="$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Delete :NSMicrophoneUsageDescription" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :NSMicrophoneUsageDescription string 'Sound Edit records from your microphone or line input when you press Record.'" "$PLIST"
codesign --force --deep -s - "$APP"
rm -rf "/Applications/Sound Edit.app"
cp -R "$APP" /Applications/
echo "Installed /Applications/Sound Edit.app"
