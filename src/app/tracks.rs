//! Tracks, as in Live: they all play together, mixed with their volume and pan (solo wins over
//! mute). Building the song for the live engine, and adding, moving and removing tracks.

use super::*;
use crate::engine;

/// The song as the live engine plays it: the heard tracks' clips, read straight from their audio
/// (a re-pitched clip from its original, at the speed the tempo needs), mixed as in Live.
pub(super) fn song(tracks: &[Track], rate: u32, end: usize, master: f32, click: Option<(f64, u32)>, session: &[TrackPlay]) -> engine::Song {
    let any_solo = tracks.iter().any(|t| t.solo);
    let mut open = false;
    let layers: Vec<engine::LayerSrc> = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| if any_solo { t.solo } else { !t.mute })
        .map(|(i, t)| {
            let play = session.get(i);
            // A track playing from the Session view leaves its song-view clips silent, as in Live.
            let mut clips: Vec<engine::ClipSrc> = if play.is_some_and(|p| p.owned) {
                Vec::new()
            } else {
                t.clips.iter().zip(crossfades(&t.clips)).filter(|(c, _)| c.len > 0 && c.active).map(|(c, cross)| clip_src(c, cross, rate)).collect()
            };
            // Launched Session clips: each loops from the moment it was launched until stopped.
            for &(slot, from, to) in play.map_or(&[][..], |p| &p.segments[..]) {
                if let Some(Some(c)) = t.slots.get(slot) {
                    let mut c = c.clone();
                    c.start = from;
                    c.len = to.unwrap_or(from + FOREVER).saturating_sub(from).max(1);
                    open |= to.is_none();
                    clips.push(clip_src(&c, (0, 0), rate));
                }
            }
            engine::LayerSrc::new(i, t.gain, if t.pan == 0.0 { (1.0, 1.0) } else { effects::pan_gains(t.pan) }, clips)
        })
        .collect();
    // Spacers: point each at its source's place among the heard tracks.
    let mut layers = layers;
    let order: Vec<usize> = layers.iter().map(|l| l.track).collect();
    for l in &mut layers {
        if let Some(set) = tracks[l.track].spacer {
            l.spacer = order.iter().position(|&t| t == set.source).map(|src| (src, set));
        }
    }
    // While a Session clip plays on, so does the transport.
    let end = if open { end.max(1) + FOREVER } else { end };
    engine::Song { layers, end, rate, master, click }
}

/// How long a launched clip can play on for (about a week of frames).
const FOREVER: usize = 1 << 35;

fn clip_src(c: &Clip, cross: (usize, usize), rate: u32) -> engine::ClipSrc {
    let (audio, speed, frames) = match c.warp.as_ref().filter(|_| repitch_mode()) {
        // Re-pitched live from the original, so a new tempo needs no re-render.
        Some(w) => (w.source.clone(), w.tempo / w.bpm, (w.source.frames() as f64 * w.bpm / w.tempo).round() as usize),
        None => (c.audio.clone(), 1.0, c.audio.frames()),
    };
    let looping = c.looping.then(|| c.loop_points.map(|(s, e)| (s.min(frames), e.min(frames))).filter(|(s, e)| e > s).unwrap_or((0, frames)));
    let mut src = engine::ClipSrc { audio, speed, start: c.start, len: c.len, trim: c.trim, looping, frames, fades: (0, 0), gain: c.gain };
    let level = |t: usize| (0..2).map(|ch| src.sample_at(t, ch).abs()).fold(0.0f32, f32::max);
    let (first, last) = (level(c.start), level(c.end() - 1));
    src.fades = with_clip_fades(c, edge_fades_for(first, last, c.len, rate, cross));
    src
}

impl App {
    pub(super) fn set_active(&mut self, i: usize) {
        if i < self.tracks.len() {
            self.active = i;
        }
    }

    /// Keep each Spacer pointing at the same track after tracks move (none if it's gone).
    fn remap_spacers(&mut self, f: impl Fn(usize) -> Option<usize>) {
        for t in &mut self.tracks {
            t.spacer = t.spacer.and_then(|mut s| {
                s.source = f(s.source)?;
                Some(s)
            });
        }
    }

    pub(super) fn add_track(&mut self, track: Track, label: &'static str) {
        self.push_undo(label);
        let at = (self.active + 1).min(self.tracks.len());
        self.remap_spacers(|t| Some(if t >= at { t + 1 } else { t }));
        self.tracks.insert(at, track);
        self.active = at;
        self.dirty = true;
        self.refresh_playback();
    }

    pub(super) fn new_track(&mut self) {
        let n = (1..).find(|i| !self.tracks.iter().any(|t| t.name == format!("Track {i}"))).unwrap();
        self.add_track(Track::new(format!("Track {n}")), "New Track");
        self.status = "New empty track — paste, record or import clips into it".into();
    }

    pub(super) fn duplicate_track(&mut self) {
        let Some(t) = self.tracks.get(self.active) else { return };
        let mut t = t.clone();
        t.name = format!("{} copy", t.name);
        self.add_track(t, "Duplicate Track");
    }

    pub(super) fn delete_track(&mut self) {
        if self.tracks.len() < 2 {
            return;
        }
        self.push_undo("Delete Track");
        self.tracks.remove(self.active);
        let gone = self.active;
        self.remap_spacers(|t| match t {
            t if t == gone => None,
            t if t > gone => Some(t - 1),
            t => Some(t),
        });
        self.active = self.active.min(self.tracks.len() - 1);
        self.sel_clip = None;
        self.dirty = true;
        self.refresh_playback();
    }

    /// Move the active layer above or below its neighbour, changing which one is heard.
    pub(super) fn move_layer(&mut self, up: bool) {
        let a = self.active;
        let Some(b) = (if up { a.checked_sub(1) } else { Some(a + 1) }).filter(|&b| b < self.tracks.len()) else { return };
        self.push_undo(if up { "Move Track Up" } else { "Move Track Down" });
        self.tracks.swap(a, b);
        let swap = |t: usize| if t == a { b } else if t == b { a } else { t };
        self.remap_spacers(|t| Some(swap(t)));
        self.sel_clip = self.sel_clip.map(|(t, c)| (swap(t), c));
        self.sel_extra = self.sel_extra.iter().map(|&(t, c)| (swap(t), c)).collect();
        self.active = b;
        self.dirty = true;
        self.refresh_playback();
    }

    pub(super) fn mix_down(&mut self) {
        if self.tracks.len() < 2 {
            return;
        }
        let mix = Arc::new(self.mixdown());
        self.push_undo("Mix Down");
        let mut t = Track::new("Mix");
        t.clips.push(Clip::new("Mix", mix, 0));
        self.tracks = vec![t];
        self.active = 0;
        self.sel_clip = Some((0, 0));
        self.dirty = true;
        self.refresh_playback();
        self.status = "Mixed all tracks into one — ⌘Z to undo".into();
    }
}

