//! Ableton Live Set export: a gzipped .als file built from Live 12's own default set, with
//! one audio track per arrangement track and the clips placed in its arrangement.

use std::io::Write;
use std::path::Path;

/// Live's DefaultLiveSet.als (decompressed). Its first audio track is the template for ours.
const TEMPLATE: &str = include_str!("../assets/ableton_template.xml");

pub struct LiveTrack {
    pub name: String,
    /// Index into Live's clip colour palette.
    pub color: u32,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub clips: Vec<LiveClip>,
}

/// A clip on the arrangement. Arrangement times are in beats; `loop_*`, `start_rel` and
/// `sample_beats` are in beats of the sample itself, played at its original speed.
pub struct LiveClip {
    pub name: String,
    pub color: u32,
    /// Path of the sample relative to the set's folder, and its absolute path.
    pub rel_path: String,
    pub abs_path: String,
    pub file_size: u64,
    pub frames: usize,
    pub rate: u32,
    pub start: f64,
    pub end: f64,
    pub looping: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    pub start_rel: f64,
    pub sample_beats: f64,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Replace the first `old` that comes after `anchor` (searching from `from`), returning where
/// the anchor was found so later edits can keep searching from there.
fn replace_after(s: &mut String, from: usize, anchor: &str, old: &str, new: &str) -> Result<usize, String> {
    let a = s[from..].find(anchor).map(|i| from + i).ok_or_else(|| format!("template has no {anchor}"))?;
    let o = s[a..].find(old).map(|i| a + i).ok_or_else(|| format!("template has no {old} after {anchor}"))?;
    s.replace_range(o..o + old.len(), new);
    Ok(a)
}

/// The text between `open` and the end of its closing tag `close`, starting at the first `open`.
fn block<'a>(s: &'a str, open: &str, close: &str) -> Result<(usize, usize), String> {
    let a = s.find(open).ok_or_else(|| format!("template has no {open}"))?;
    let b = s[a..].find(close).map(|i| a + i + close.len()).ok_or_else(|| format!("template has no {close}"))?;
    Ok((a, b))
}

/// Give every pointee id in a copied track (automation and modulation targets and the like,
/// which are unique across the set) a fresh number. Small ids are list indices and stay.
fn renumber(track: &str, next: &mut u64) -> String {
    let mut out = String::with_capacity(track.len());
    let mut rest = track;
    while let Some(i) = rest.find(" Id=\"") {
        let v = i + 5;
        let Some(len) = rest[v..].find('"') else { break };
        out.push_str(&rest[..v]);
        match rest[v..v + len].parse::<u64>() {
            Ok(n) if n >= 100 => {
                out.push_str(&next.to_string());
                *next += 1;
            }
            _ => out.push_str(&rest[v..v + len]),
        }
        rest = &rest[v + len..];
    }
    out.push_str(rest);
    out
}

fn clip_xml(id: usize, c: &LiveClip, beats_per_bar: u32) -> String {
    // Two warp markers a second apart at the song tempo keep the sample at its own speed.
    let n = |v: f64| format!("{v}");
    format!(
        r#"<AudioClip Id="{id}" Time="{start}">
<LomId Value="0" />
<LomIdView Value="0" />
<CurrentStart Value="{start}" />
<CurrentEnd Value="{end}" />
<Loop>
<LoopStart Value="{ls}" />
<LoopEnd Value="{le}" />
<StartRelative Value="{sr}" />
<LoopOn Value="{on}" />
<OutMarker Value="{out}" />
<HiddenLoopStart Value="{ls}" />
<HiddenLoopEnd Value="{le}" />
</Loop>
<Name Value="{name}" />
<Annotation Value="" />
<Color Value="{color}" />
<LaunchMode Value="0" />
<LaunchQuantisation Value="0" />
<TimeSignature>
<TimeSignatures>
<RemoteableTimeSignature Id="0">
<Numerator Value="{bpb}" />
<Denominator Value="4" />
<Time Value="0" />
</RemoteableTimeSignature>
</TimeSignatures>
</TimeSignature>
<Envelopes>
<Envelopes />
</Envelopes>
<ScrollerTimePreserver>
<LeftTime Value="0" />
<RightTime Value="{out}" />
</ScrollerTimePreserver>
<TimeSelection>
<AnchorTime Value="0" />
<OtherTime Value="0" />
</TimeSelection>
<Legato Value="false" />
<Ram Value="false" />
<GrooveSettings>
<GrooveId Value="-1" />
</GrooveSettings>
<Disabled Value="false" />
<VelocityAmount Value="0" />
<FollowAction>
<FollowTime Value="4" />
<IsLinked Value="true" />
<LoopIterations Value="1" />
<FollowActionA Value="4" />
<FollowActionB Value="0" />
<FollowChanceA Value="100" />
<FollowChanceB Value="0" />
<JumpIndexA Value="1" />
<JumpIndexB Value="1" />
<FollowActionEnabled Value="false" />
</FollowAction>
<Grid>
<FixedNumerator Value="1" />
<FixedDenominator Value="16" />
<GridIntervalPixel Value="20" />
<Ntoles Value="2" />
<SnapToGrid Value="true" />
<Fixed Value="false" />
</Grid>
<FreezeStart Value="0" />
<FreezeEnd Value="0" />
<IsWarped Value="true" />
<TakeId Value="0" />
<IsInKey Value="false" />
<ScaleInformation>
<Root Value="0" />
<Name Value="0" />
</ScaleInformation>
<SampleRef>
<FileRef>
<RelativePathType Value="3" />
<RelativePath Value="{rel}" />
<Path Value="{abs}" />
<Type Value="2" />
<LivePackName Value="" />
<LivePackId Value="" />
<OriginalFileSize Value="{size}" />
<OriginalCrc Value="0" />
<SourceHint Value="" />
</FileRef>
<LastModDate Value="0" />
<SourceContext />
<SampleUsageHint Value="0" />
<DefaultDuration Value="{frames}" />
<DefaultSampleRate Value="{rate}" />
<SamplesToAutoWarp Value="0" />
</SampleRef>
<Onsets>
<UserOnsets />
<HasUserOnsets Value="false" />
</Onsets>
<WarpMode Value="0" />
<GranularityTones Value="30" />
<GranularityTexture Value="65" />
<FluctuationTexture Value="25" />
<TransientResolution Value="6" />
<TransientLoopMode Value="2" />
<TransientEnvelope Value="100" />
<ComplexProFormants Value="100" />
<ComplexProEnvelope Value="128" />
<Sync Value="true" />
<HiQ Value="true" />
<Fade Value="false" />
<Fades>
<FadeInLength Value="0" />
<FadeOutLength Value="0" />
<ClipFadesAreInitialized Value="true" />
<CrossfadeInState Value="0" />
<FadeInCurveSkew Value="0" />
<FadeInCurveSlope Value="0" />
<FadeOutCurveSkew Value="0" />
<FadeOutCurveSlope Value="0" />
<IsDefaultFadeIn Value="false" />
<IsDefaultFadeOut Value="false" />
</Fades>
<PitchCoarse Value="0" />
<PitchFine Value="0" />
<SampleVolume Value="1" />
<WarpMarkers>
<WarpMarker Id="0" SecTime="0" BeatTime="0" />
<WarpMarker Id="1" SecTime="1" BeatTime="{bps}" />
</WarpMarkers>
<SavedWarpMarkersForStretched />
<MarkersGenerated Value="false" />
<IsSongTempoLeader Value="false" />
</AudioClip>
"#,
        start = n(c.start),
        end = n(c.end),
        ls = n(c.loop_start),
        le = n(c.loop_end),
        sr = n(c.start_rel),
        on = c.looping,
        out = n(c.sample_beats),
        name = esc(&c.name),
        color = c.color,
        bpb = beats_per_bar,
        rel = esc(&c.rel_path),
        abs = esc(&c.abs_path),
        size = c.file_size,
        frames = c.frames,
        rate = c.rate,
        bps = n(c.sample_beats / (c.frames as f64 / c.rate.max(1) as f64)),
    )
}

/// Build the Live Set's XML.
pub fn build(tempo: f64, beats_per_bar: u32, tracks: &[LiveTrack]) -> Result<String, String> {
    let mut s = TEMPLATE.to_string();
    let (_, tpl_end) = block(&s, "<AudioTrack Id=\"8\"", "</AudioTrack>")?;
    let tpl_start = s.find("<AudioTrack Id=\"8\"").unwrap();
    let template = s[tpl_start..tpl_end].to_string();

    let next_tag = "<NextPointeeId Value=\"";
    let np = s.find(next_tag).ok_or("template has no NextPointeeId")? + next_tag.len();
    let np_len = s[np..].find('"').ok_or("bad NextPointeeId")?;
    let mut next: u64 = s[np..np + np_len].parse().map_err(|_| "bad NextPointeeId")?;

    let mut out = String::new();
    for (i, t) in tracks.iter().enumerate() {
        let mut x = renumber(&template, &mut next);
        x = x.replacen("<AudioTrack Id=\"8\"", &format!("<AudioTrack Id=\"{}\"", 100 + i), 1);
        let name = esc(&t.name);
        replace_after(&mut x, 0, "<Name>", "<EffectiveName Value=\"3-Audio\" />", &format!("<EffectiveName Value=\"{name}\" />"))?;
        replace_after(&mut x, 0, "<Name>", "<UserName Value=\"\" />", &format!("<UserName Value=\"{name}\" />"))?;
        replace_after(&mut x, 0, "</Name>", "<Color Value=\"20\" />", &format!("<Color Value=\"{}\" />", t.color))?;
        replace_after(&mut x, 0, "<Speaker>", "<Manual Value=\"true\" />", &format!("<Manual Value=\"{}\" />", !t.mute))?;
        replace_after(&mut x, 0, "<SoloSink", "<SoloSink Value=\"false\" />", &format!("<SoloSink Value=\"{}\" />", t.solo))?;
        replace_after(&mut x, 0, "<Pan>", "<Manual Value=\"0\" />", &format!("<Manual Value=\"{}\" />", t.pan.clamp(-1.0, 1.0)))?;
        replace_after(&mut x, 0, "<Volume>", "<Manual Value=\"1\" />", &format!("<Manual Value=\"{}\" />", t.gain.clamp(0.0, 1.99526)))?;
        let clips: String = t.clips.iter().enumerate().map(|(k, c)| clip_xml(k, c, beats_per_bar)).collect();
        let seq = x.find("<MainSequencer>").ok_or("template has no MainSequencer")?;
        replace_after(&mut x, seq, "<ArrangerAutomation>", "<Events />", &format!("<Events>\n{clips}</Events>"))?;
        out.push_str(&x);
        out.push_str("\n\t\t\t");
    }

    // Swap the default MIDI and audio tracks for ours (the return tracks stay).
    let first = s.find("<MidiTrack Id=").ok_or("template has no tracks")?;
    let returns = s.find("<ReturnTrack Id=").ok_or("template has no return tracks")?;
    s.replace_range(first..returns, &out);

    let np = s.find(next_tag).unwrap() + next_tag.len();
    let np_len = s[np..].find('"').unwrap();
    s.replace_range(np..np + np_len, &next.to_string());

    // Tempo and time signature live on the main track, and in its automation's first event.
    let bpm = format!("{}", tempo.clamp(20.0, 999.0));
    let sig = (beats_per_bar.clamp(1, 99) - 1 + 2 * 99).to_string();
    let main = s.find("<MainTrack").ok_or("template has no main track")?;
    replace_after(&mut s, main, "<Tempo>", "<Manual Value=\"120\" />", &format!("<Manual Value=\"{bpm}\" />"))?;
    replace_after(&mut s, main, "<TimeSignature>", "<Manual Value=\"201\" />", &format!("<Manual Value=\"{sig}\" />"))?;
    replace_after(&mut s, main, "<AutomationEnvelopes>", "<FloatEvent Id=\"0\" Time=\"-63072000\" Value=\"120\" />", &format!("<FloatEvent Id=\"0\" Time=\"-63072000\" Value=\"{bpm}\" />"))?;
    replace_after(&mut s, main, "<AutomationEnvelopes>", "<EnumEvent Id=\"0\" Time=\"-63072000\" Value=\"201\" />", &format!("<EnumEvent Id=\"0\" Time=\"-63072000\" Value=\"{sig}\" />"))?;
    Ok(s)
}

/// Write the set, gzipped as Live expects.
pub fn write(path: &Path, tempo: f64, beats_per_bar: u32, tracks: &[LiveTrack]) -> Result<(), String> {
    let xml = build(tempo, beats_per_bar, tracks)?;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    gz.write_all(xml.as_bytes()).map_err(|e| e.to_string())?;
    gz.finish().map_err(|e| e.to_string())?;
    Ok(())
}
