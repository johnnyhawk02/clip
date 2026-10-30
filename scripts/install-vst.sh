#!/bin/sh
# Builds each of Clip's devices as a VST3 and installs it to the Johnny folder.
# With names (tape, autofilter, ...) builds just those.
set -e
cd "$(dirname "$0")/../vst"
dest="$HOME/Library/Audio/Plug-Ins/VST3/Johnny"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

name_of() {
    case $1 in
        autofilter) echo "Clip Auto Filter" ;;
        *) echo "Clip $(echo "$1" | awk '{print toupper(substr($0,1,1)) substr($0,2)}')" ;;
    esac
}

for fx in ${@:-reverb spring delay tape phaser chorus pultec riser tame opto autofilter spacer}; do
    name=$(name_of "$fx")
    cargo build --release --features "$fx" 2>&1 | grep -E '^error|Finished' || true
    bundle="target/bundles/$name.vst3"
    rm -rf "$bundle"
    mkdir -p "$bundle/Contents/MacOS"
    cp target/release/libclip_vst.dylib "$bundle/Contents/MacOS/$name"
    printf 'BNDL????' > "$bundle/Contents/PkgInfo"
    cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>$name</string>
    <key>CFBundleIdentifier</key><string>com.johnnyhawk.clip.$fx</string>
    <key>CFBundleName</key><string>$name</string>
    <key>CFBundlePackageType</key><string>BNDL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
</dict>
</plist>
PLIST
    codesign --force --sign - "$bundle"
    mkdir -p "$dest"
    rm -rf "$dest/$name.vst3"
    cp -R "$bundle" "$dest/"
    echo "installed $name"
done
