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
# Let Finder open Sound Edit projects (and audio, via Open With) in the app.
plutil -replace UTExportedTypeDeclarations -json '[{"UTTypeIdentifier":"com.johnnyhawk.soundedit.project","UTTypeDescription":"Sound Edit Project","UTTypeConformsTo":["public.data"],"UTTypeTagSpecification":{"public.filename-extension":["sndproj"]}}]' "$PLIST"
plutil -replace CFBundleDocumentTypes -json '[{"CFBundleTypeName":"Sound Edit Project","CFBundleTypeRole":"Editor","LSHandlerRank":"Owner","LSItemContentTypes":["com.johnnyhawk.soundedit.project"]},{"CFBundleTypeName":"Audio","CFBundleTypeRole":"Editor","LSHandlerRank":"Alternate","LSItemContentTypes":["public.audio"]}]' "$PLIST"
codesign --force --deep -s - "$APP"
rm -rf "/Applications/Sound Edit.app"
cp -R "$APP" /Applications/
# Tell Launch Services about the document types.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "/Applications/Sound Edit.app" 2>/dev/null || true
echo "Installed /Applications/Sound Edit.app"
