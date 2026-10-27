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

The look is modelled on classic Mac sound editors: a platinum desktop, the sound in a document
window (black waveform on white, selection shown inverted), and floating **Tools**, **Controls**
and **Info** windows (Window menu, ⌘1 / ⌘2 / ⌘I). Every command is in the menu bar:
**File, Edit, Insert, Effects, Control, View, Window**.

**Tools** (palette, or press the key):

| Tool | Key | What it does |
|---|---|---|
| Selection | S | Drag to select, shift-click to extend, double-click selects all, click sets the insertion point |
| Hand | H | Drag to scroll through the sound |
| Zoom | Z | Click to zoom in, ⌥-click to zoom out, drag a range to zoom to it |
| Pencil | P | Redraw the waveform by hand (zoom in close first); each stroke is one undo step |

**Controls** has rewind, play/pause, stop, loop and end buttons, a time counter and L/R level
meters. **Info** shows the format, length, selection and peak level.

| Do this | How |
|---|---|
| Play / pause | Space (plays the selection if there is one). Esc stops. ⌘L loops the selection. |
| Zoom | Pinch or ⌘-scroll, ⌘= / ⌘-, ⌘0 shows the entire sound, ⌘E zooms to the selection |
| Scroll | Scroll / two-finger swipe, the Hand tool, or the bar under the waveform |
| Cut / Copy / Paste / Clear | ⌘X / ⌘C / ⌘V / ⌫ |
| Mix Paste | ⌥⌘V — mixes the clipboard in at the cursor (with a level) instead of inserting it |
| Trim | ⌘T keeps only the selection |
| Undo / Redo | ⌘Z / ⇧⌘Z |
| New | ⌘N gives an empty 44.1 kHz stereo sound to build up with Insert, Paste or Mix Paste |
| Export | ⌘S whole sound, ⇧⌘S selection. Bit depth is under File ▸ Export Format. |
| Split into sounds | ⇧⌘K — see below |

**Insert menu:** Silence, Tone (sine/square/triangle/sawtooth, frequency, level, length) and
Noise, at the insertion point or replacing the selection.

**Effects menu** (selection, or the whole sound if nothing is selected): Amplify, Normalize,
Fade In/Out, Backwards, Echo, Reverb, Flanger, Smooth (low-pass), Noise Gate, Change Pitch (keeps
the length), Change Tempo (keeps the pitch), Change Speed (tape-style) and Convert to Mono. Each
opens a small dialog for its settings.

**Split into Sounds…** cuts a recording of separate clicks/hits into one file per sound and saves
them as `sound01.wav`, `sound02.wav`… in a folder you pick. A sound starts at the first non-zero
sample and ends once there have been at least that many zero samples in a row (10 by default; the
dialog shows how many sounds it will find). It uses the selection, or the whole sound, and the WAV
format chosen under File ▸ Export Format. It looks for exact zeros, so it suits recordings with
true digital silence. For noisy recordings, run **Noise Gate** first: it turns quiet parts into
exact zeros.

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
