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
    pub rate: u32,
    pub tracks: Vec<TrackData>,
    pub audio: Vec<AudioData>,
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
    pub color: usize,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<ClipData>,
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
    pub group: Option<Vec<TrackData>>,
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
    r.read_exact(&mut head).map_err(|_| "Not a Sound Edit project")?;
    if &head[0..8] != MAGIC {
        return Err("Not a Sound Edit project".into());
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
            rate: 1000,
            tracks: vec![TrackData {
                name: "T".into(),
                gain: 0.8,
                pan: -0.5,
                color: 2,
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
                    group: None,
                }],
            }],
            audio: vec![AudioData { rate: 1000, channels: 2, frames: 3 }],
        };
        let path = std::env::temp_dir().join("sound_edit_test.sndproj");
        write(&path, &p, &[&a]).unwrap();
        let (q, audio) = read(&path).unwrap();
        assert_eq!(audio[0].channels, a.channels);
        assert_eq!(q.tracks[0].clips[0].cues[0].1, "x");
        assert_eq!(q.tracks[0].pan, -0.5);
        std::fs::remove_file(path).ok();
    }
}
