# PhotoSound

A desktop audio editor written in Rust (egui + symphonia + cpal + hound), built for one job:
chopping up loops, keeping the good bits and editing them into a track. (It was called Sound Edit
until October 2026.)

## Tracks, as in Live

Tracks play together, mixed. Each has Live's track activator (its number, lit while it plays),
Solo, a volume slider, a colour and pan; solo wins over mute.

- Fades: drag the small handles at a clip's top corners to fade it in or out.
- Locators: Control ▸ Add Locator (or right-click the ruler) adds a named marker; [ and ] jump
  between them, a click on one jumps there, and right-click renames it, deletes it or loops to the
  next one.
- Playback starts from where you last clicked (the insert marker) and stopping leaves it there;
  ⇧Space carries on from where you stopped. Control ▸ Follow (⌘F) turns view scrolling on or off,
  and the Metronome button in the top bar (or Control ▸ Metronome) clicks along on every beat, higher
  on the first of the bar. It's only heard, never exported.
  Clip ▸ Quantize (⌘U) snaps clip starts to the grid (Slice at Grid moved to ⌥⌘U).
- More Live editing: ⇧⌥-drag a clip's waveform to slide its audio inside it; Edit ▸ Cut / Copy /
  Paste Time (⇧⌘X ⇧⌘C ⇧⌘V) work on a stretch of every track; ⇧⌘J crops clips to a time selection
  and ⌘J consolidates it into one clip per track; 0 switches clips off and on (an off clip stays in
  place, silent). Arrow keys nudge the selected clips or move the insert
  marker by the grid, ⇧← ⇧→ grow the selection, ↑ ↓ move the loop by its length (Loop on, nothing
  selected) or change track, ⌘↑ ⌘↓ double or halve the loop. Double-click the bar numbers to zoom
  to the selection (or the whole song); H fits the tracks to the height. Drag a locator to move it,
  click it to select it, Delete removes it and ⌘R renames it. The Clip View has Pitch (semitones,
  keeping the length) and ×2 / ÷2 for the clip's BPM.
- Select one clip and it opens in the detail view under the song, as in Live: its waveform with the
  bar ruler, start flag and loop strip, and the Clip View down the left (Warp and the clip's BPM,
  Loop, Reverse, Start and Length in bars.beats.sixteenths, Gain). Click in the waveform to work on
  the clip there; click the song above to go back. The transport sits along the top.
- Playback mixes the tracks live as it plays, as Live does, so edits and tempo changes are heard
  at once with nothing to render first. Dragging the tempo re-pitches what you hear straight away.
- Recording puts the take on the selected track at the insert marker. Tracks ▸ Mix Down to One
  Track mixes everything into one clip.

**Guide:** https://johnnyhawk02.github.io/photosound/

## Chopping loops

- Drop loops in; ones with a tempo in the name ("118bpm 16bar", "124 BPM") warp to the song tempo.
- Split Clip (⌘E) cuts the clip under the pointer, at the pointer (snapped to the grid; hold ⌘
  for a free cut), or at the playhead when the pointer isn't over a clip. The Split tool (B) cuts
  with a click.
- Slice at Grid (⌥⌘U) cuts the selected clips at every grid line (the grid size you pick, or
  every beat when the grid is Off or Auto), ready to drag around, duplicate (⌘D), loop and group.
- Clip ▸ Mangle rearranges the selected clips in the slots they sit in (a single clip is sliced
  at the grid first): Shuffle, Reverse Order, Swap Pairs, Stutter (some slices repeat the one
  before), Random Reverse, Drop Out, and BBCut (breakbeat cutting in the style of Nick Collins' BBCut: the selection is re-cut in blocks, some jump to other parts of the loop, some become stutter rolls). BBCut comes in styles: Jungle (3+3+2 eighths, accelerating rolls), House (straight quarters, a fill roll at the end of a bar), Hip-Hop (beat chops shuffled), Dembow (3+3+2 sixteenths), Footwork (sixteenths in threes, busy rolls), Trap (triplet rolls), Breakcore (everything, plus reversed bits) and Halftime (long blocks, sparse fills). Mangle Again (⌘Y) re-rolls the last one. Also on the
  clip's right-click menu, along with Reverse.
- To mangle a stretch of time instead of whole clips, drag across the ruler to set the loop and turn Loop on. Mangle then cuts the clips at the loop's edges, slices what's inside at the grid, and mangles only that, on the track you're working on (or the tracks of the selected clips). You hear it looping, and ⌘Y gives a new variation.
- Auto Arrange (in the top bar, or Clip ▸ Auto Arrange) builds a 4-minute track from the loops in the song. Each loop is warped to the song tempo and cut into 8 or 16 bar sections, and the sections are laid end to end on one track, following an intro, a build, a breakdown and an outro: calm loops for the quiet parts, busy ones for the peaks. ◀ ▶ step through variations; each number always gives the same layout, so you can go back to one you liked. Loops shorter than a section repeat to fill it.
- Clip edges get a short fade, as in Live: none where the audio crosses zero at the edge (so a
  transient stays sharp), up to 4 ms where the edge cuts through a loud part of the wave and would
  click.
- Clips on a track never overlap, as in Live: drop, paste or drag a clip onto another on the same
  track and it cuts a gap in the one underneath the length of the dropped clip. Dragging a clip's edge over its neighbour instead overlaps the two, and they
  crossfade across the overlap (equal power), shown by an X.
- Swap Section (⇧R, or Clip ▸ Auto Arrange ▸ Swap Section): click a clip and press ⇧R to replace it with a random section of the same length from the loop pool, in the same place. Press ⇧R again for another; ⌘Z undoes it.
- The Loop Pool (View ▸ Panels ▸ Loop Pool, ⌘7) lists the loops in the song, warped to its tempo. Drag one onto a track to add it there at the grid line you drop it on, or below the last track for a new one. Auto Arrange and Swap Section draw from the same pool; Grouped clips count as loops too, and Clip ▸ Add to Pool (also on the right-click menu) puts any selected clip in as it is, trimmed or grouped. × takes a loop out of the pool and Add Loops from Song brings in new ones.
- The Master strip under the tracks holds the output volume (saved in the project, double-click to reset), its meters, and a headphone button for Headphone Crossfeed. Control ▸ Headphone Crossfeed makes headphones sound like speakers: each ear also hears a delayed, filtered share of the other side. The filters are measured from Johnny's CanOpener setting (a click through each side) and match it to within -100 dB. It only changes what you hear, never what you export, and it stays on between runs.
- The tool palette floats: a strip of the tools, then a row of Mangle buttons and a row of
  BBCut styles. Drag it by its title, collapse it with the triangle, close it with the cross
  (Window ▸ Tools brings it back). Window ▸ Dock Tools on the Left puts it back in the left column.
- The menus hold just that. The older sound-editor features (effects, insert, markers, recording,
  extra tools and panels, batch convert) are all still there, under the **More** menu.

## Run it

- **App:** open `/Applications/PhotoSound.app` (or Spotlight "PhotoSound").
  A fresh copy is built at `target/release/bundle/osx/PhotoSound.app` (`scripts/install.sh` installs it).
- **Terminal:** `cargo run --release -- path/to/file.wav`

Open a file by dragging it onto the window, with **Open…** (⌘O), or as a command-line argument.
Reads WAV, MP3, FLAC, OGG/Vorbis, M4A/AAC, AIFF, CAF. Exports WAV (16-bit, 24-bit or 32-bit float).
Try the files in `samples/`.

## Using it

**Projects:** File ▸ Save Project (⌘S) / Save Project As (⇧⌘S) saves everything — tracks,
clips, groups, cues, regions, loops, mixer settings and the audio itself — in one `.sndproj`
file. Open it again by double-clicking it in Finder, with File ▸ Open, or by dropping it on the
window. (Audio files can be opened from Finder with Open With ▸ PhotoSound too.) Export (⇧⌘R) writes
what you hear as a WAV/AIFF. The unsaved-changes prompt offers to save the project. When you open the app on its own, it reopens the last project you opened or saved.

**Arrangement and clips (like Ableton's Arrangement view).** A sound is a stack of tracks,
and audio sits on them as clips. Opening a file gives one track with one clip.

- Drag a clip to move it along its track or onto another track (⌥-drag drags a copy instead; dragging below the last track makes a new track for it); drag its left or right edge to
  trim it (the audio underneath is kept, so you can drag the edge back out). Moves and trims snap
  to other clips' edges and the playhead; hold ⌘ to turn snapping off.
- Selecting works like Live's: drag across empty space, or across a clip's waveform, to select a
  stretch of time on one or more tracks (a light blue wash, snapped to the grid; ⌘ for free). ⌫
  deletes that stretch, ⌘E splits the clips at its ends, ⌘L / Set Loop loops it, ⌘D duplicates
  it right after itself (⇧⌘D Duplicate Time pushes the rest later), Edit ▸ Delete Time (⇧⌘⌫) takes it
  out and closes the gap, Insert Time (⌘I) opens up empty time, and Mangle, BBCut
  and the clip processes (Reverse, Swap, Slice, Collapse, Warp, Loop, Duplicate, Copy, Cut, Group)
  work only inside it. Drag a clip by
  its name strip to move it; click clips (shift-click for more) to select whole clips.
- Click empty space to move the playhead. Space plays the whole arrangement from the playhead.
- **Double-click a clip (or press Return) to edit it** in the waveform editor below, with every
  tool and effect. Its ruler works like the song view's: bars
  and beats counting 1.1.1 from the clip's start, a loop strip on top (whole bars), and drag the bar
  numbers to zoom. The black flag is the start marker, as in Live: drag it and the clip plays from
  there, looping round so the audio before it comes at the end; the waveform stays put (snaps to
  the ruler's ticks, ⌘ for anywhere). Right-click the flag ▸ Set 1.1.1 Here to count bars from it,
  with -1, -2… before it. With Snap on, selections snap to the song's grid (⌘ snaps to zero
  crossings instead). ◀ Arrangement or ⌘Return goes back.
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
  For the arrangement, drag in the strip along the top of the ruler to draw a loop range (a black
  brace; drag it to move the whole loop, or either end to resize). Loops are always whole bars.
  Drag in the bar numbers below the strip to zoom (down zooms in, up zooms out, sideways scrolls)
  and turn Loop on or off with ⌘L or the loop button; playback then goes round that range. Insert ▸
  Set Loop to Selection in the arrangement sets it to the selected clips. Both are saved in the
  project.
- **Right-click menus:** right-click a clip for Edit/Open, Cut, Copy, Duplicate, Delete, Split,
  Loop, Group/Ungroup, Collapse to Audio and Set Tempo from Clip. Right-click a track (its header
  or empty lane space) for New/Duplicate/Delete Track, Move Track Up/Down, Mute, Colour, Collapse
  Track to One Clip and Import File as Clip. **Collapse** (Clip ▸ Collapse to Audio, ⌘J) renders a group,
  looped or trimmed clip down to one plain audio clip that sounds the same.
- **Clips within clips:** shift-click clips (or ⌘A for all) and choose Clip ▸ Group into Clip (⌘G).
  They become one group clip holding them, in the same places and keeping their tracks' settings.
  Double-click a group to open it as its own arrangement; the path bar (Song ▸ Group 1 ▸ …) or
  ⌘Return goes back up. Groups can hold groups, can be moved, trimmed, split and duplicated like
  any clip (copies are independent), and Clip ▸ Ungroup (⇧⌘G) puts the clips back.
- Export writes what the whole song sounds like (even from inside a group). Cues, regions and loops
  belong to each clip.
- Warping: a clip whose file name gives its tempo ("Loop 118bpm 16bar", "Kick - 124 BPM",
  "Bpm130_Drums") is stretched to the song tempo when it's dropped in or imported, keeping its
  pitch. A bar count in the name gives the exact tempo. Change the song tempo and warped clips
  re-fit, staying on their beats: by default they re-pitch like a record (speed and pitch move
  together, as Live's Re-Pitch), or PhotoSound ▸ Preferences ▸ Keep pitch stretches them instead; unwarped clips move to stay on their bar too (playing at their own
  speed), and the loop and playhead follow. Opening such a file sets the song tempo from it. Clip ▸ Warp to
  Tempo switches warping on or off for the selected clips (clips with no tempo in the name are
  taken as the whole-bar loop closest to the song tempo); Clip ▸ Warp Loops on Import turns the
  automatic part off. Editing or collapsing a warped clip keeps it warped, so it keeps
  following the tempo. Clips that don't follow the tempo show "no warp" in their name strip. Warping is saved in projects, and the Ableton export hands Live the original audio with
  warp markers, so the clips follow Live's tempo too.
- File ▸ Export Ableton Live Set makes a "<name> Project" folder with a Live 12 set (.als) and a
  Samples folder. Each track becomes an audio track with its name, colour, volume, pan, mute and
  solo; clips sit where they are, with clip loops as Live
  loops, and the tempo and beats per bar carry over. Clips are warped so they play at their own
  speed. Groups come across as their rendered audio. The set is built from Live's own default set
  (`assets/ableton_template.xml`).
- A fine line runs across the arrangement every 4 bars, counting from bar 1, so phrases are easy to
  see. PhotoSound ▸ Preferences (⌘,) sets how many bars apart (0 turns them off).
- Shortcuts follow Ableton Live wherever the two apps share a feature: ⌘E split, ⌘D duplicate, ⌘J consolidate (Collapse to Audio), ⌘G / ⇧⌘G group and ungroup, ⌘L loop on/off, R reverse clip, Z zoom to selection, X zoom back, W fit the song in the window, ⌘1 / ⌘2 narrow and widen the grid, ⌘4 snap, B draw (Pencil), ⌘T new track, ⇧⌘J crop (Trim), ⇧⌘R export, F9 record, ⇧Tab switch between the arrangement and the clip editor. Select None is ⇧⌘A.


A clean black, white and grey layout: tools on the left, the waveform in the middle, the transport
bar (play controls, time counter, level meters) along the bottom, and an Info / Cues side panel on
the right. Window ▸ Tools / Controls / Info / Cues (⌥⌘1 / ⌥⌘2 / ⌘I / ⌘3) shows or hides each part.
Every command is in the menu bar: **File, Edit, Insert, Effects, Control, View, Window**.

**Tools** (palette, or press the key):

| Tool | Key | What it does |
|---|---|---|
| Selection | S | Drag to select, shift-click to extend, double-click selects all, click sets the insertion point |
| Hand | H | Drag to scroll through the sound |
| Zoom | — | Click to zoom in, ⌥-click to zoom out, drag a range to zoom to it |
| Pencil | B | Redraw the waveform by hand (zoom in close first); each stroke is one undo step |
| Split | C | In the arrangement, click a clip to cut it there (snaps to the grid; hold ⌘ to cut anywhere) |

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
| Zoom | Pinch or ⌘-scroll, ⌘= / ⌘-, W or ⌘0 shows the entire sound, Z zooms to the selection and X zooms back |
| Scroll | Scroll / two-finger swipe, the Hand tool, or the bar under the waveform |
| Cut / Copy / Paste / Clear | ⌘X / ⌘C / ⌘V / ⌫ |
| Mix Paste | ⌥⌘V — mixes the clipboard in at the cursor (with a level) instead of inserting it |
| Trim | ⇧⌘J keeps only the selection |
| Undo / Redo | ⌘Z / ⇧⌘Z |
| New | ⌘N gives an empty 44.1 kHz stereo sound to build up with Insert, Paste or Mix Paste |
| Save project | ⌘S, Save As ⇧⌘S |
| Export | ⇧⌘R the mix, ⌥⇧⌘S the selection (editor) or selected clip (arrangement). Bit depth is under File ▸ Export Format. |
| Split into sounds | ⇧⌘K — see below |

**Tracks:** the Tracks menu adds tracks: New Track (⌘T), Import File as Track… (⇧⌘O), Duplicate
Track and Delete Track. Each one has a header with its name, an activator (mute), Solo, a volume slider,
a colour swatch and pan. Click a track, or use ⌥⌘↑ / ⌥⌘↓, to make it the one you're editing. Tracks ▸
Mix Down to One Track turns what you hear into a single clip (undoable).

**Quality:** sample-rate changes (Convert Format, Batch Convert, pasting between files) use a
Kaiser-windowed sinc resampler, so nothing aliases when converting down. Writing 8/16/24-bit files
adds TPDF dither (File ▸ Export Format ▸ Dither, on by default). Auto Gain (on by default) turns the
output down if it would clip. Effects ▸ Cut Out Gaps shortens silent stretches.

**Loops and regions:** Insert ▸ Set Loop to Selection (⇧⌘L) marks loop points (a black bar in the
ruler); with Loop on (⌘L) and nothing selected, Play repeats them. Insert ▸ Region from Selection
(⌥⌘R) adds a named region (a grey band in the ruler; click it to select it). Window ▸ Regions lists
them; File ▸ Export Regions saves each as its own file. Cues, regions and loop points are all saved
in WAV files.

**Typed-in times:** the Info panel's Start, End and Length fields take typed times (or drag them).
View ▸ Ruler Units switches the ruler, counter and fields between minutes:seconds, samples and
30 fps timecode.

**Tracks, further:** More ▸ Tracks ▸ Offset Track moves a track later or earlier.

**Spectrum** (Window ▸ Spectrum, ⌘4): 2D shows the frequencies in the selection (or at the cursor,
following playback live); 3D shows a waterfall of spectra across the selection.

**Batch Convert** (File menu): convert many files at once to WAV (8/16/24/32-bit float) or AIFF,
optionally changing the sample rate or channels and normalizing.

**Recording:** Control ▸ Record… (F9), or the record button in the transport bar, opens the Record window.
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
- `src/ptrack.rs` — playback through the default output device (resamples to the device rate)
- `src/actions.rs` — every command with its label, icon and shortcut; the menu and palette layouts
- `src/menu.rs` — the native macOS menu bar (muda)
- `src/app.rs` — the app's state, undo/redo, playback, the action dispatcher, shortcuts and the frame loop
  - `src/app/clip.rs` — clips and tracks, warping to the song tempo, clip operations and groups
  - `src/app/chop.rs` — Split, Slice at Grid, Reverse, Mangle and BBCut
  - `src/app/arrange.rs` — the arrangement view and its right-click menus
  - `src/app/editor.rs` — the single-clip waveform editor and selection edits
  - `src/app/dialogs.rs` — effect dialogs and the curve, spectrum, mix and record windows
  - `src/app/files.rs` — opening, saving, export (mix, stems, Ableton, regions) and Batch Convert
  - `src/app/panels.rs` — Tools palette, transport bar, side panel and status bar
  - `src/app/look.rs` — colours, style, shared dialog widgets and drawn icons
  - `src/app/autoarrange.rs` — Auto Arrange: a whole track from a pool of loops
  - `src/app/automation.rs` — scripted headless runs for testing

For headless checks, `SOUND_EDIT_ACTIONS="sel:1-2,fadeout"` and `SOUND_EDIT_SNAPSHOT=out.png`
run scripted actions on the opened file, save a screenshot and quit (`run:<Action>` runs any
command, `dumpmenu` prints the menu bar).
