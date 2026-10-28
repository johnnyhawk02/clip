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

**Projects:** File ▸ Save Project (⌘S) / Save Project As (⇧⌘S) saves everything — tracks,
clips, groups, cues, regions, loops, mixer settings and the audio itself — in one `.sndproj`
file. Open it again by double-clicking it in Finder, with File ▸ Open, or by dropping it on the
window. (Audio files can be opened from Finder with Open With ▸ Sound Edit too.) Export Mix (⌥⌘S) writes
the mixed-down WAV/AIFF. The unsaved-changes prompt offers to save the project.

**Arrangement and clips (like Ableton's Arrangement view).** A sound is an arrangement of tracks,
and audio sits on them as clips. Opening a file gives one track with one clip.

- Drag a clip to move it along its track or onto another track (⌥-drag drags a copy instead; dragging below the last track makes a new track for it); drag its left or right edge to
  trim it (the audio underneath is kept, so you can drag the edge back out). Moves and trims snap
  to other clips' edges and the playhead; hold ⌘ to turn snapping off.
- Click empty space to move the playhead. Space plays the whole arrangement from the playhead.
- **Double-click a clip (or press Return) to edit it** in the waveform editor below, with every
  tool and effect. ◀ Arrangement or ⌘Return goes back.
- Clip menu: New Clip, Import File as Clip (at the playhead), Edit Clip, Split Clip at Playhead
  (⌘E), Duplicate Clip (⌘D, placed right after). ⌫ deletes the selected clip; ⌘C / ⌘X / ⌘V copy,
  cut and paste clips (paste goes on the selected track at the playhead). Dropping a file onto
  a track adds it there at the drop point; dropping it on the empty space below the tracks
  makes a new track for it. Dropping several files puts each on its own track (the first on
  the track under the mouse, the rest on the tracks below, making new ones as needed), all at
  the drop point.
- **Tempo and grid:** the bar above the arrangement sets the song tempo (BPM), beats per bar
  and the grid (Auto, 1 Bar, 1/2 … 1/32, or Off). With the grid on, the ruler shows bars and
  beats; moves, trims, clicks on the playhead and drops snap to the grid (Clip ▸ Snap to Grid,
  ⌘4, toggles it; hold ⌘ while dragging to bypass). Clip ▸ Set Tempo from Clip reads a loop's
  length as a whole number of bars and sets the tempo to match (e.g. an 8-second 4-bar loop
  → 120 BPM). Tempo, grid and snap are saved in the project. (Spectrum moved to ⌘6.)
- **Loops:** Clip ▸ Loop Clip (⌥⌘L) makes a clip repeat its loop section — its loop points
  (set in the clip editor with Insert ▸ Set Loop to Selection) or the whole clip — for as long as
  you drag its right edge out. Each clip loops on its own; dotted lines show where it starts over.
  For the arrangement, drag in the ruler to draw a loop range (a brace along the top of the ruler)
  and turn Loop on or off with ⌘L or the loop button; playback then goes round that range. Insert ▸
  Set Loop to Selection in the arrangement sets it to the selected clips. Both are saved in the
  project.
- **Right-click menus:** right-click a clip for Edit/Open, Cut, Copy, Duplicate, Delete, Split,
  Loop, Group/Ungroup, Collapse to Audio and Set Tempo from Clip. Right-click a track (its header
  or empty lane space) for New/Duplicate/Delete Track, Mute, Solo, Colour, Collapse Track to One
  Clip and Import File as Clip. **Collapse** (Clip ▸ Collapse to Audio, ⌘J) renders a group,
  looped or trimmed clip down to one plain audio clip that sounds the same.
- **Clips within clips:** shift-click clips (or ⌘A for all) and choose Clip ▸ Group into Clip (⌘G).
  They become one group clip holding them, in the same places and keeping their tracks' settings.
  Double-click a group to open it as its own arrangement; the path bar (Song ▸ Group 1 ▸ …) or
  ⌘Return goes back up. Groups can hold groups, can be moved, trimmed, split and duplicated like
  any clip (copies are independent), and Clip ▸ Ungroup (⌥⌘G) puts the clips back.
- Export writes the mix of the whole song (even from inside a group). Cues, regions and loops
  belong to each clip.
- Warping: a clip whose file name gives its tempo ("Loop 118bpm 16bar", "Kick - 124 BPM",
  "Bpm130_Drums") is stretched to the song tempo when it's dropped in or imported, keeping its
  pitch. A bar count in the name gives the exact tempo. Change the song tempo and warped clips
  re-fit, staying on their beats. Opening such a file sets the song tempo from it. Clip ▸ Warp to
  Tempo switches warping on or off for the selected clips (clips with no tempo in the name are
  taken as the whole-bar loop closest to the song tempo); Clip ▸ Warp Loops on Import turns the
  automatic part off. Editing a warped clip in the editor makes it plain audio at its current
  speed. Warping is saved in projects, and the Ableton export hands Live the original audio with
  warp markers, so the clips follow Live's tempo too.
- File ▸ Export Stems writes one WAV per track (with its volume and pan), all starting at zero and
  all the same length, so they line up in another DAW. Together they add up to the mix.
- File ▸ Export Ableton Live Set makes a "<name> Project" folder with a Live 12 set (.als) and a
  Samples folder. Each track becomes an audio track with its name, colour, volume, pan, mute and
  solo; clips sit in the arrangement where they are, trimmed the same, with clip loops as Live
  loops, and the tempo and beats per bar carry over. Clips are warped so they play at their own
  speed. Groups come across as their rendered audio. The set is built from Live's own default set
  (`assets/ableton_template.xml`).
- Shortcut changes: Select None is now ⇧⌘A and Zoom to Selection is ⇧⌘E.


A clean black, white and grey layout: tools on the left, the waveform in the middle, the transport
bar (play controls, time counter, level meters) along the bottom, and an Info / Cues side panel on
the right. Window ▸ Tools / Controls / Info / Cues (⌘1 / ⌘2 / ⌘I / ⌘3) shows or hides each part.
Every command is in the menu bar: **File, Edit, Insert, Effects, Control, View, Window**.

**Tools** (palette, or press the key):

| Tool | Key | What it does |
|---|---|---|
| Selection | S | Drag to select, shift-click to extend, double-click selects all, click sets the insertion point |
| Hand | H | Drag to scroll through the sound |
| Zoom | Z | Click to zoom in, ⌥-click to zoom out, drag a range to zoom to it |
| Pencil | P | Redraw the waveform by hand (zoom in close first); each stroke is one undo step |
| Split | B | In the arrangement, click a clip to cut it there (snaps to the grid; hold ⌘ to cut anywhere) |

**Mouse and modifier keys:**

| Do this | How |
|---|---|
| Extend a selection | Shift-click or Shift-drag |
| Fine-tune a selection edge | ⌥-drag near the edge: it moves at 1/10 speed |
| Snap | Hold ⌘ while clicking or dragging: snaps to a cue within 8 pixels, otherwise to the nearest zero crossing (where the waveform crosses the centre line, so cuts don't click). ⌘-dragging a cue snaps it to a zero crossing. |
| Zoom at the mouse | Pinch, ⌘-scroll or ⌥-scroll |
| Scroll sideways | Scroll, two-finger swipe or Shift-scroll |
| Temporary Hand tool | Hold Space and drag (a quick tap of Space still plays/pauses) |
| Zoom out | ⌥-click with the Zoom tool |
| Play from the mouse | ⌥-click in the waveform |
| Straight pencil line | Shift-click with the Pencil: draws a line from the end of the last stroke |
| Duplicate a cue | ⌥-drag its flag |
| Select between cues | Double-click (double-click again for the whole sound) |

The transport bar has rewind, play/pause, stop, record, loop and end buttons, a time counter and
level meters. **Info** shows the format, length, selection and peak level.

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
| Save project | ⌘S, Save As ⇧⌘S |
| Export | ⌥⌘S the mix, ⌥⇧⌘S the selection (editor) or selected clip (arrangement). Bit depth is under File ▸ Export Format. |
| Split into sounds | ⇧⌘K — see below |

**Multiple tracks:** the Tracks menu adds tracks: New Track (⇧⌘T, silent, as long as the sound),
Import File as Track… (⇧⌘O), Duplicate Track and Delete Track. Tracks play together, mixed. Each one
has a header with its name, **M** (mute), **S** (solo) and a volume slider. Click a track, or use ⌥⌘↑ / ⌥⌘↓,
to make it the one you're editing: selections, effects, pasting, recording and the pencil all
work on that track. Export writes the mix of all tracks, and Tracks ▸ Mix Down to One Track
combines them into a single track (undoable).

**Quality:** sample-rate changes (Convert Format, Batch Convert, pasting between files) use a
Kaiser-windowed sinc resampler, so nothing aliases when converting down. Writing 8/16/24-bit files
adds TPDF dither (File ▸ Export Format ▸ Dither, on by default). Tracks ▸ Auto Gain (on by default)
turns a mix down if it would clip. Effects ▸ Cut Out Gaps shortens silent stretches.

**Loops and regions:** Insert ▸ Set Loop to Selection (⇧⌘L) marks loop points (a black bar in the
ruler); with Loop on (⌘L) and nothing selected, Play repeats them. Insert ▸ Region from Selection
(⇧⌘R) adds a named region (a grey band in the ruler; click it to select it). Window ▸ Regions lists
them; File ▸ Export Regions saves each as its own file. Cues, regions and loop points are all saved
in WAV files.

**Typed-in times:** the Info panel's Start, End and Length fields take typed times (or drag them).
View ▸ Ruler Units switches the ruler, counter and fields between minutes:seconds, samples and
30 fps timecode.

**Tracks, further:** each track header also has a colour swatch (click to change) and a pan slider
(double-click to centre). Tracks ▸ Offset Track moves a track later or earlier, and Tracks ▸ Mix…
picks which tracks to mix, at what volume and pan, into a new track or in place of them all.

**Spectrum** (Window ▸ Spectrum, ⌘4): 2D shows the frequencies in the selection (or at the cursor,
following playback live); 3D shows a waterfall of spectra across the selection.

**Batch Convert** (File menu): convert many files at once to WAV (8/16/24/32-bit float) or AIFF,
optionally changing the sample rate or channels and normalizing.

**Recording:** Control ▸ Record… (⌘R), or the record button in the transport bar, opens the Record window.
Press Record, then Stop: the take goes in at the insertion point (or replaces the selection), or
becomes a new sound if nothing is open. It records from the Mac's current input (System Settings ▸
Sound ▸ Input); macOS asks for microphone permission the first time.

**Cue points:** Insert ▸ Cue Point (⌘M) marks the insertion point. Cues show as flags in the
ruler: drag a flag to move it, click it to jump there, and use [ and ] to step between cues. The
Cues panel lists them so you can rename, jump to or remove them. Cues move with your edits and are
saved in exported WAV files (and read back when you open one).

**Insert menu:** Silence, Tone (sine/square/triangle/sawtooth, frequency, level, length),
Noise, FM Synthesis (two-operator: carrier, ratio, brightness, attack, decay) and Touch Tones (phone-dial DTMF beeps for any keys you type, e.g. `555-0123`; a comma adds a
pause), at the insertion point or replacing the selection.

**Effects menu** (selection, or the whole sound if nothing is selected): Amplify, Normalize,
Fade In/Out, Fade… (from/to level with Linear/Smooth/Fast/Slow curves), Backwards, Delay (one
repeat, optional ping-pong), Echo, Reverb (with Small room/Studio/Hall/Cathedral/Plate presets),
Flanger, Smooth (low-pass), Emphasize (treble lift), Compressor, Noise Gate, Change Pitch (keeps
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
- `src/project.rs` — the .sndproj project file
- `src/ableton.rs` — Ableton Live Set export
- `src/recorder.rs` — recording from the default input
- `src/player.rs` — playback through the default output device (resamples to the device rate)
- `src/actions.rs` — every command with its label, icon and shortcut; the menu and palette layouts
- `src/menu.rs` — the native macOS menu bar (muda)
- `src/app.rs` — the window: tool palette, waveform view, selection, dialogs, undo/redo

For headless checks, `SOUND_EDIT_ACTIONS="sel:1-2,fadeout"` and `SOUND_EDIT_SNAPSHOT=out.png`
run scripted actions on the opened file, save a screenshot and quit (`run:<Action>` runs any
command, `dumpmenu` prints the menu bar).
