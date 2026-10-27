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
, **Info** and **Cues** windows (Window menu, ⌘1 / ⌘2 / ⌘I / ⌘3). Every command is in the menu bar:
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

**Recording:** Control ▸ Record… (⌘R), or the red button in Controls, opens the Record window.
Press Record, then Stop: the take goes in at the insertion point (or replaces the selection), or
becomes a new sound if nothing is open. It records from the Mac's current input (System Settings ▸
Sound ▸ Input); macOS asks for microphone permission the first time.

**Cue points:** Insert ▸ Cue Point (⌘M) marks the insertion point. Cues show as blue flags in the
ruler: drag a flag to move it, click it to jump there, and use [ and ] to step between cues. The
Cues window lists them so you can rename, jump to or remove them. Cues move with your edits and are
saved in exported WAV files (and read back when you open one).

**Insert menu:** Silence, Tone (sine/square/triangle/sawtooth, frequency, level, length),
Noise and Touch Tones (phone-dial DTMF beeps for any keys you type, e.g. `555-0123`; a comma adds a
pause), at the insertion point or replacing the selection.

**Effects menu** (selection, or the whole sound if nothing is selected): Amplify, Normalize,
Fade In/Out, Backwards, Echo, Reverb, Flanger, Smooth (low-pass), Noise Gate, Change Pitch (keeps
the length), Change Tempo (keeps the pitch), Change Speed (tape-style) and Convert to Mono. Each
opens a small dialog for its settings. Also:

- **Envelope…** — draw a volume curve across the selection (0–200%), with Swell and Duck presets.
- **Bender…** — draw a pitch curve (±12 semitones); like a tape, bending up also speeds it up.
- **Equalizer…** — five-band graphic EQ (100 Hz, 400 Hz, 1.2 kHz, 3.5 kHz, 9 kHz, ±12 dB).
- **Convert Format…** — change the whole sound's sample rate (8–96 kHz), mono/stereo, or reduce
  it to 8/12/16-bit; 8 or 16 bits also sets the export format (8-bit WAV export is available).

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

To install, run `scripts/install.sh`: it bundles the app, adds the microphone permission text
macOS needs for recording, signs it ad hoc and copies it to /Applications.

## Code

- `src/audio.rs` — decoding, WAV export, the edit operations, waveform peak cache
- `src/recorder.rs` — recording from the default input
- `src/player.rs` — playback through the default output device (resamples to the device rate)
- `src/actions.rs` — every command with its label, icon and shortcut; the menu and palette layouts
- `src/menu.rs` — the native macOS menu bar (muda)
- `src/app.rs` — the window: tool palette, waveform view, selection, dialogs, undo/redo

For headless checks, `SOUND_EDIT_ACTIONS="sel:1-2,fadeout"` and `SOUND_EDIT_SNAPSHOT=out.png`
run scripted actions on the opened file, save a screenshot and quit (`run:<Action>` runs any
command, `dumpmenu` prints the menu bar).
