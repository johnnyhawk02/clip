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

| Do this | How |
|---|---|
| Select | Drag on the waveform. Shift-click/shift-drag extends. Double-click selects all (⌘A). |
| Place cursor | Click. Esc clears the selection. |
| Play / pause | Space. Plays the selection if there is one, otherwise from the cursor. |
| Stop | Esc while playing (returns to where playback started). L toggles looping the selection. |
| Zoom | Pinch or ⌘-scroll over the waveform, `=` / `-`, **F** fit, **Z** zoom to selection |
| Scroll | Scroll / two-finger swipe, or drag the bar under the waveform |
| Trim to selection | ⌘T |
| Delete selection | ⌫ |
| Cut / copy / paste | ⌘X / ⌘C / ⌘V (paste inserts at the cursor or replaces the selection) |
| Undo / redo | ⌘Z / ⇧⌘Z |
| Export WAV | ⌘S (tick "selection only" to export just the selection) |

**Split into sounds…** cuts a recording of separate clicks/hits into one file per sound and saves
them as `sound01.wav`, `sound02.wav`… in a folder you pick. A sound starts at the first non-zero
sample and ends once there have been at least *gap* zero samples in a row (10 by default, set next
to the button). It works on the selection, or the whole file if nothing is selected, and uses the
WAV format chosen in the toolbar. It looks for exact zeros, so it suits WAV/AIFF/FLAC recordings
with true digital silence; MP3/AAC rarely contain exact zeros between sounds.

Fade in/out, Silence, Normalize (to the dB target next to it), Gain, Reverse and To mono are in the
toolbar. Normalize, Gain and Reverse act on the whole file when nothing is selected.

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
- `src/app.rs` — the window: toolbar, waveform view, selection, shortcuts, undo/redo

For headless checks, `SOUND_EDIT_ACTIONS="sel:1-2,fadeout"` and `SOUND_EDIT_SNAPSHOT=out.png`
run scripted actions on the opened file, save a screenshot and quit.
