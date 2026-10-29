#!/bin/sh
# Build the .app, add the microphone permission text macOS needs for recording,
# sign it ad hoc and copy it to /Applications.
set -e
cd "$(dirname "$0")/.."
cargo bundle --release
APP="target/release/bundle/osx/PhotoSound.app"
PLIST="$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Delete :NSMicrophoneUsageDescription" "$PLIST" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :NSMicrophoneUsageDescription string 'PhotoSound records from your microphone or line input when you press Record.'" "$PLIST"
# Let Finder open PhotoSound projects (and audio, via Open With) in the app.
plutil -replace UTExportedTypeDeclarations -json '[{"UTTypeIdentifier":"com.johnnyhawk.soundedit.project","UTTypeDescription":"PhotoSound Project","UTTypeConformsTo":["public.data"],"UTTypeTagSpecification":{"public.filename-extension":["sndproj"]}}]' "$PLIST"
plutil -replace CFBundleDocumentTypes -json '[{"CFBundleTypeName":"PhotoSound Project","CFBundleTypeRole":"Editor","LSHandlerRank":"Owner","LSItemContentTypes":["com.johnnyhawk.soundedit.project"]},{"CFBundleTypeName":"Audio","CFBundleTypeRole":"Editor","LSHandlerRank":"Alternate","LSItemContentTypes":["public.audio"]}]' "$PLIST"
codesign --force --deep -s - "$APP"
# The app was called Sound Edit before it became PhotoSound.
rm -rf "/Applications/Sound Edit.app" "/Applications/PhotoSound.app"
cp -R "$APP" /Applications/
# Leave no copies in the build folder, or Spotlight lists them next to the installed app.
LSREG=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
for old in target/release/bundle/*/*.app; do
  $LSREG -u "$old" 2>/dev/null || true
  rm -rf "$old"
done
touch target/.metadata_never_index
# Tell Launch Services about the document types.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "/Applications/PhotoSound.app" 2>/dev/null || true
echo "Installed /Applications/PhotoSound.app"
