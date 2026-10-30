//! The project file (.sndproj): the arrangement as JSON, followed by the audio it uses.
//!
//! Layout: `SNDPROJ1`, the JSON length (u64, little-endian), the JSON, then each audio buffer
//! as planar little-endian f32 samples, in the order listed in `audio`.

use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::audio::Audio;

const MAGIC: &[u8; 8] = b"SNDPROJ1";

#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    pub version: u32,
    #[serde(default = "default_tempo")]
    pub tempo: f64,
    #[serde(default = "default_bpb")]
    pub beats_per_bar: u32,
    #[serde(default = "default_grid")]
    pub grid: usize,
    #[serde(default = "default_snap")]
    pub snap: bool,
    #[serde(default)]
    pub arr_loop: Option<(usize, usize)>,
    #[serde(default)]
    pub looping: bool,
    #[serde(default = "default_master")]
    pub master_gain: f32,
    /// Named markers in the song (frame, name), as Live's locators.
    #[serde(default)]
    pub locators: Vec<(usize, String)>,
    /// The Session view's scene names.
    #[serde(default)]
    pub scenes: Vec<String>,
    /// Return tracks (none in projects from before them: they get the default two).
    #[serde(default)]
    pub returns: Option<Vec<TrackData>>,
    /// How the window and views were when it was saved (as a Live Set keeps them).
    #[serde(default)]
    pub view: Option<ViewState>,
    pub rate: u32,
    pub tracks: Vec<TrackData>,
    pub audio: Vec<AudioData>,
}

/// The window and view settings a project reopens with.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct ViewState {
    /// The Session view was showing (else the Arrangement view).
    pub session: bool,
    /// Window position and size, in points: x, y, width, height.
    pub window: Option<[f32; 4]>,
    pub fullscreen: bool,
    pub maximized: bool,
    /// The Arrangement view's visible span and insert marker, in frames.
    pub view_start: f64,
    pub view_len: f64,
    pub cursor: usize,
    pub active: usize,
    pub sel_clip: Option<(usize, usize)>,
    pub sel_slot: Option<(usize, usize)>,
    pub launch_q: f64,
    pub metronome: bool,
    pub arrangement_record: bool,
    pub follow: bool,
    pub fit_height: bool,
    pub show_pool: bool,
    pub show_devices: bool,
    pub device_target: usize,
    pub show_tools: bool,
    pub palette_docked: bool,
    pub show_controls: bool,
    pub show_info: bool,
    pub show_cues: bool,
    pub show_regions: bool,
}

fn default_tempo() -> f64 {
    120.0
}
fn default_bpb() -> u32 {
    4
}
fn default_grid() -> usize {
    1
}
fn default_snap() -> bool {
    true
}
fn default_true() -> bool {
    true
}

fn default_master() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize)]
pub struct AudioData {
    pub rate: u32,
    pub channels: usize,
    pub frames: usize,
}

#[derive(Serialize, Deserialize)]
pub struct TrackData {
    pub name: String,
    pub gain: f32,
    pub pan: f32,
    /// The track's colour: an index into Live's palette when `palette` is set, else one of
    /// the six colours older versions had.
    pub color: usize,
    #[serde(default)]
    pub palette: bool,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<ClipData>,
    /// The track's Session view slots (one per scene).
    #[serde(default)]
    pub slots: Vec<Option<ClipData>>,
    /// Its Spacer (projects from before the device chain; read only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacer: Option<crate::spacer::Settings>,
    /// Its built-in effects, in order.
    #[serde(default)]
    pub devices: Vec<crate::devices::Device>,
    /// How much it sends to each return track.
    #[serde(default)]
    pub sends: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
pub struct ClipData {
    pub name: String,
    /// Index into `ProjectFile::audio`; group clips have none (their mix is rebuilt on load).
    pub audio: Option<usize>,
    pub start: usize,
    pub trim: usize,
    pub len: usize,
    pub cues: Vec<(usize, String)>,
    pub regions: Vec<(usize, usize, String)>,
    pub loop_points: Option<(usize, usize)>,
    #[serde(default)]
    pub looping: bool,
    pub group: Option<Vec<TrackData>>,
    /// Warped clips: (clip tempo, song tempo it's stretched to, index of the original audio).
    #[serde(default)]
    pub warp: Option<(f64, f64, usize)>,
    /// Where bar 1.1.1 is in the clip's audio (frames), for the clip editor's ruler.
    #[serde(default)]
    pub origin: usize,
    /// The clip's own volume (linear).
    #[serde(default = "default_master")]
    pub gain: f32,
    /// Fades from the clip's corner handles (frames).
    #[serde(default)]
    pub fades: (usize, usize),
    /// False for a clip switched off (silent, kept in place).
    #[serde(default = "default_true")]
    pub active: bool,
    /// Its own colour in Live's palette (none follows the track).
    #[serde(default)]
    pub color: Option<usize>,
}

pub fn write(path: &Path, project: &ProjectFile, audio: &[&Audio]) -> Result<(), String> {
    let json = serde_json::to_vec(project).map_err(|e| e.to_string())?;
    // Write to a temporary file first so a failed save never damages the existing project.
    let tmp = path.with_extension("sndproj.tmp");
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut w = std::io::BufWriter::new(file);
    let mut put = |b: &[u8]| w.write_all(b).map_err(|e| e.to_string());
    put(MAGIC)?;
    put(&(json.len() as u64).to_le_bytes())?;
    put(&json)?;
    for a in audio {
        for c in &a.channels {
            let bytes: Vec<u8> = c.iter().flat_map(|s| s.to_le_bytes()).collect();
            put(&bytes)?;
        }
    }
    w.flush().map_err(|e| e.to_string())?;
    drop(w);
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn read(path: &Path) -> Result<(ProjectFile, Vec<Audio>), String> {
    let mut r = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| format!("Can't open project: {e}"))?);
    let mut head = [0u8; 16];
    r.read_exact(&mut head).map_err(|_| "Not a Clip project")?;
    if &head[0..8] != MAGIC {
        return Err("Not a Clip project".into());
    }
    let len = u64::from_le_bytes(head[8..16].try_into().unwrap()) as usize;
    let mut json = vec![0u8; len];
    r.read_exact(&mut json).map_err(|e| e.to_string())?;
    let project: ProjectFile = serde_json::from_slice(&json).map_err(|e| format!("Damaged project: {e}"))?;
    let mut audio = Vec::with_capacity(project.audio.len());
    for a in &project.audio {
        let mut channels = Vec::with_capacity(a.channels);
        for _ in 0..a.channels {
            let mut bytes = vec![0u8; a.frames * 4];
            r.read_exact(&mut bytes).map_err(|_| "The project's audio is incomplete")?;
            channels.push(bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect());
        }
        audio.push(Audio { rate: a.rate, channels });
    }
    Ok((project, audio))
}

pub fn is_project(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("sndproj"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let a = Audio { rate: 1000, channels: vec![vec![0.5, -0.25, 1.0], vec![0.0, 0.1, -1.0]] };
        let p = ProjectFile {
            version: 1,
            tempo: 128.0,
            beats_per_bar: 4,
            grid: 4,
            snap: true,
            arr_loop: Some((0, 3)),
            looping: true,
            master_gain: 1.0,
            locators: Vec::new(),
            scenes: Vec::new(),
            returns: None,
            view: Some(ViewState { session: true, window: Some([10.0, 20.0, 1100.0, 560.0]), ..Default::default() }),
            rate: 1000,
            tracks: vec![TrackData {
                name: "T".into(),
                gain: 0.8,
                pan: -0.5,
                color: 2,
                palette: true,
                mute: false,
                solo: true,
                clips: vec![ClipData {
                    name: "C".into(),
                    audio: Some(0),
                    start: 10,
                    trim: 1,
                    len: 2,
                    cues: vec![(1, "x".into())],
                    regions: vec![],
                    loop_points: Some((0, 2)),
                    looping: true,
                    group: None,
                    warp: None,
                    origin: 0,
                    gain: 1.0,
                    fades: (0, 0),
                    active: true,
                    color: None,
                }],
                slots: Vec::new(),
                spacer: None,
                devices: Vec::new(),
                sends: Vec::new(),
            }],
            audio: vec![AudioData { rate: 1000, channels: 2, frames: 3 }],
        };
        let path = std::env::temp_dir().join("sound_edit_test.sndproj");
        write(&path, &p, &[&a]).unwrap();
        let (q, audio) = read(&path).unwrap();
        assert_eq!(audio[0].channels, a.channels);
        assert_eq!(q.tracks[0].clips[0].cues[0].1, "x");
        assert_eq!(q.tracks[0].pan, -0.5);
        let v = q.view.unwrap();
        assert!(v.session);
        assert_eq!(v.window, Some([10.0, 20.0, 1100.0, 560.0]));
        std::fs::remove_file(path).ok();
    }
}
