# Sound Edit

A simple desktop audio editor written in Rust (egui + symphonia + cpal + hound).

## Run it

- **App:** open `/Applications/Sound Edit.app` (or Spotlight "Sound Edit").
  A fresh copy is built at `target/release/bundle/osx/Sound Edit.app`.
- **Terminal:** `cargo run --release -- path/to/file.wav`

Open a file by dragging it onto the window, with **Open…** (⌘O), or as a command-line argument.
Reads WAV, MP3, FLAC, OGG/Vorbis, M4A/AAC, AIFF, CAF. Exports WAV (16-bit, 24-bit or 32-bit float).
Try the files in `samples/`.

## Using it

All commands are in the macOS menu bar (**File, Edit, Effects, Play, View**). The common ones are
also in the tool palette across the top of the window; hover over a tool to see its name and shortcut.

| Do this | How |
|---|---|
| Select | Drag on the waveform. Shift-click/shift-drag extends. Double-click or ⌘A selects all, ⌘D/Esc none. |
| Place cursor | Click |
| Play / pause | Space. Plays the selection if there is one, otherwise from the cursor. |
| Stop | Esc while playing (returns to where playback started). ⌘L (or L) loops the selection. |
| Zoom | Pinch or ⌘-scroll over the waveform, ⌘= / ⌘- (or = / -), ⌘0 or F to fit, ⌘E or Z to zoom to the selection |
| Scroll | Scroll / two-finger swipe, or drag the bar under the waveform |
| Trim to selection | ⌘T |
| Delete selection | ⌫ |
| Cut / copy / paste | ⌘X / ⌘C / ⌘V (paste inserts at the cursor or replaces the selection) |
| Undo / redo | ⌘Z / ⇧⌘Z |
| Export WAV | ⌘S for the whole file, ⇧⌘S for just the selection. Bit depth is under File ▸ Export Format. |
| Normalize / Gain | ⇧⌘N / ⇧⌘G — opens a small dialog for the dB value |
| Split into sounds | ⇧⌘K — see below |

Edits are never written to your original file. You'll be asked before quitting or opening another
file with unexported changes.

## Build

```sh
cargo build --release          # binary at target/release/sound-edit
cargo bundle --release         # .app (and .dmg) in target/release/bundle/
cargo test --release
```

After bundling, `cp -R "target/release/bundle/osx/Sound Edit.app" /Applications/` and
`codesign --force --deep -s - "/Applications/Sound Edit.app"` to install.

## Code

- `src/audio.rs` — decoding, WAV export, the edit operations, waveform peak cache
- `src/player.rs` — playback through the default output device (resamples to the device rate)
- `src/actions.rs` — every command with its label, icon and shortcut; the menu and palette layouts
- `src/menu.rs` — the native macOS menu bar (muda)
- `src/app.rs` — the window: tool palette, waveform view, selection, dialogs, undo/redo

For headless checks, `SOUND_EDIT_ACTIONS="sel:1-2,fadeout"` and `SOUND_EDIT_SNAPSHOT=out.png`
run scripted actions on the opened file, save a screenshot and quit (`run:<Action>` runs any
command, `dumpmenu` prints the menu bar).
