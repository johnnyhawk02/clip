//! The twelve plugins: each device's name and controls, with the ranges Clip's panels give them.

use nih_plug::prelude::*;

use crate::devices::Device;
use crate::knobs::List;
use crate::{Effect, autofilter, chorus, fx, opto, phaser, pultec, riser, spacer, tame, tape};

/// Clip's insert-style starting mix (as its Reverb Device… dialogs), since a plugin is rarely on
/// a return that wants it all wet.
const WET: f32 = 0.3;

/// The note lengths Delay and Tape's echo times come in.
const TIMES: &[(&str, f32)] = &[("1/16", 0.25), ("1/8", 0.5), ("3/16", 0.75), ("1/4", 1.0), ("3/8", 1.5), ("1/2", 2.0), ("3/4", 3.0), ("1 Bar", 4.0)];

macro_rules! effect {
    ($feature:literal, $ty:ident, $name:literal, $id:literal, [$($sub:ident),*], $settings:ty, $device:path, $start:expr, |$l:ident| $knobs:expr $(, $side:literal)?) => {
        pub struct $ty;

        impl Effect for $ty {
            const NAME: &'static str = $name;
            const CLASS_ID: [u8; 16] = *$id;
            const SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, $(Vst3SubCategory::$sub),*];
            $(const SIDECHAIN: bool = $side;)?
            type Settings = $settings;
            fn settings() -> $settings {
                $start
            }
            fn device(s: $settings) -> Device {
                $device(s)
            }
            fn knobs($l: List<$settings>) -> List<$settings> {
                $knobs
            }
        }

        #[cfg(feature = $feature)]
        nih_plug::nih_export_vst3!(crate::ClipFx<$ty>);
    };
}

effect!("reverb", ClipReverb, "Clip Reverb", b"JohnnyClipReverb", [Reverb], fx::ReverbSettings, Device::Reverb,
    fx::ReverbSettings { mix: WET, ..Default::default() }, |l| l
        .pct("Size", |s| &mut s.size)
        .pct("Damping", |s| &mut s.damping)
        .pct("Width", |s| &mut s.width)
        .float("Pre-Delay", 0.0, 200.0, " ms", 0, |s| &mut s.predelay)
        .pct("Mix", |s| &mut s.mix));

effect!("spring", ClipSpring, "Clip Spring", b"JohnnyClipSpring", [Reverb], fx::SpringSettings, Device::Spring,
    fx::SpringSettings { mix: WET, ..Default::default() }, |l| l
        .float("Length", 20.0, 120.0, " ms", 0, |s| &mut s.length)
        .pct("Decay", |s| &mut s.decay)
        .log("Tone", 1500.0, 7000.0, " Hz", 0, |s| &mut s.tone)
        .pct("Chirp", |s| &mut s.chirp)
        .pct("Mix", |s| &mut s.mix));

effect!("delay", ClipDelay, "Clip Delay", b"JohnnyClipDelay_", [Delay], fx::DelaySettings, Device::Delay,
    fx::DelaySettings { mix: WET, ..Default::default() }, |l| l
        .values("Time", TIMES, |s| &mut s.beats)
        .pct_range("Feedback", 0.0, 0.95, |s| &mut s.feedback)
        .toggle("Ping Pong", |s| &mut s.ping_pong)
        .log("Low Cut", 20.0, 2000.0, " Hz", 0, |s| &mut s.low_cut)
        .log("High Cut", 500.0, 20000.0, " Hz", 0, |s| &mut s.high_cut)
        .pct("Mix", |s| &mut s.mix));

effect!("tape", ClipTape, "Clip Tape", b"JohnnyClipTape__", [Delay, Distortion], tape::Settings, Device::Tape,
    tape::Settings { mix: WET, ..Default::default() }, |l| l
        .values("Time", TIMES, |s| &mut s.beats)
        .pct_range("Feedback", 0.0, 1.05, |s| &mut s.feedback)
        .pct("Wow", |s| &mut s.wow)
        .pct("Mix", |s| &mut s.mix)
        .values("Speed", &[("3.75 ips", 3.75), ("7.5 ips", 7.5), ("15 ips", 15.0), ("30 ips", 30.0)], |s| &mut s.speed)
        .pct("Drive", |s| &mut s.drive)
        .pct("Bias", |s| &mut s.bias)
        .float("Spacing", 0.0, 30.0, " um", 0, |s| &mut s.spacing)
        .choice("LFO Shape", &tape::LFO_SHAPES, |s| &mut s.lfo_shape)
        .log("LFO Rate", 0.01, 20.0, " Hz", 2, |s| &mut s.lfo_rate)
        .toggle("LFO x0.01", |s| &mut s.lfo_slow)
        .float("LFO Depth", 0.0, 50.0, " ms", 1, |s| &mut s.lfo_depth)
        .pct("LFO Slew", |s| &mut s.lfo_slew));

effect!("phaser", ClipPhaser, "Clip Phaser", b"JohnnyClipPhaser", [Modulation], phaser::Settings, Device::Phaser,
    phaser::Settings::default(), |l| l
        .log("Speed", 0.05, 10.0, " Hz", 2, |s| &mut s.speed)
        .pct("Mix", |s| &mut s.mix));

// Choosing a mode in Clip sets its own rate, so here Rate is a multiple of the mode's.
effect!("chorus", ClipChorus, "Clip Chorus", b"JohnnyClipChorus", [Modulation], chorus::Settings, Device::Chorus,
    chorus::Settings::default(), |l| l
        .choice("Mode", &chorus::MODES, |s| &mut s.mode)
        .times("Rate", 0.1, 10.0, |s, v| s.rate = chorus::preset(s.mode).rate * v)
        .pct_range("Amount", 0.0, 2.0, |s| &mut s.amount)
        .pct("Mix", |s| &mut s.mix)
        .pct("Noise", |s| &mut s.noise)
        .pct("Drive", |s| &mut s.drive));

effect!("pultec", ClipPultec, "Clip Pultec", b"JohnnyClipPultec", [Eq], pultec::Settings, Device::Pultec,
    pultec::Settings::default(), |l| l
        .values("Low Freq", &[("20 Hz", 20.0), ("30 Hz", 30.0), ("60 Hz", 60.0), ("100 Hz", 100.0)], |s| &mut s.low_freq)
        .float("Low Boost", 0.0, 10.0, "", 1, |s| &mut s.low_boost)
        .float("Low Atten", 0.0, 10.0, "", 1, |s| &mut s.low_atten)
        .values("High Freq", &[("3 kHz", 3.0), ("4 kHz", 4.0), ("5 kHz", 5.0), ("8 kHz", 8.0), ("10 kHz", 10.0), ("12 kHz", 12.0), ("16 kHz", 16.0)], |s| &mut s.high_freq)
        .float("High Boost", 0.0, 10.0, "", 1, |s| &mut s.high_boost)
        .float("Bandwidth", 0.0, 10.0, "", 1, |s| &mut s.bandwidth)
        .values("Atten Sel", &[("5 kHz", 5.0), ("10 kHz", 10.0), ("20 kHz", 20.0)], |s| &mut s.atten_sel)
        .float("High Atten", 0.0, 10.0, "", 1, |s| &mut s.high_atten));

// Automate Amount for the rise: that's what Clip's Ramp does when printing.
effect!("riser", ClipRiser, "Clip Riser", b"JohnnyClipRiser_", [Modulation], riser::Settings, Device::Riser,
    riser::Settings { ramp: false, ..Default::default() }, |l| l
        .pct("Amount", |s| &mut s.amount));

effect!("tame", ClipTame, "Clip Tame", b"JohnnyClipTame__", [Restoration, Dynamics], tame::Settings, Device::Tame,
    tame::Settings::default(), |l| l
        .pct("Depth", |s| &mut s.depth)
        .pct("Detail", |s| &mut s.detail)
        .pct("Speed", |s| &mut s.speed)
        .log("Low", 20.0, 5000.0, " Hz", 0, |s| &mut s.low)
        .log("High", 200.0, 20000.0, " Hz", 0, |s| &mut s.high)
        .toggle("Hard", |s| &mut s.hard)
        .float("Threshold", -60.0, 0.0, " dB", 1, |s| &mut s.threshold)
        .toggle("Delta", |s| &mut s.delta)
        .pct("Mix", |s| &mut s.mix));

effect!("opto", ClipOpto, "Clip Opto", b"JohnnyClipOpto__", [Dynamics], opto::Settings, Device::Opto,
    opto::Settings::default(), |l| l
        .pct("Peak Reduction", |s| &mut s.peak_reduction)
        .float("Gain", -12.0, 24.0, " dB", 1, |s| &mut s.gain)
        .toggle("Limit", |s| &mut s.limit)
        .pct("Colour", |s| &mut s.colour)
        .pct("Mix", |s| &mut s.mix));

effect!("autofilter", ClipAutoFilter, "Clip Auto Filter", b"JohnnyClipAutoFl", [Filter], autofilter::Settings, Device::AutoFilter,
    autofilter::Settings::default(), |l| {
        let mut l = l
            .choice("Type", &autofilter::KINDS, |s| &mut s.kind)
            .log("Frequency", 20.0, 20000.0, " Hz", 0, |s| &mut s.freq)
            .pct("Resonance", |s| &mut s.res)
            .float("Drive", 0.0, 24.0, " dB", 1, |s| &mut s.drive)
            .float("Output", -24.0, 24.0, " dB", 1, |s| &mut s.output)
            .pct("Mix", |s| &mut s.mix)
            .pct_range("Follower Amount", -1.0, 1.0, |s| &mut s.env_amount)
            .log("Follower Attack", 0.1, 1000.0, " ms", 1, |s| &mut s.env_attack)
            .log("Follower Release", 1.0, 5000.0, " ms", 0, |s| &mut s.env_release)
            .pct_range("Env Amount", -1.0, 1.0, |s| &mut s.gen_amount)
            .choice("Env Trigger", &autofilter::TRIGGERS, |s| &mut s.gen_trigger)
            .values("Env Rate", &autofilter::SYNC_RATES, |s| &mut s.gen_beats)
            .log("Env Attack", 0.1, 5000.0, " ms", 1, |s| &mut s.gen_attack)
            .log("Env Decay", 1.0, 10000.0, " ms", 0, |s| &mut s.gen_decay)
            .pct("Env Sustain", |s| &mut s.gen_sustain)
            .log("Env Release", 1.0, 10000.0, " ms", 0, |s| &mut s.gen_release)
            .pct("LFO Amount", |s| &mut s.lfo_amount)
            .toggle("LFO Sync", |s| &mut s.lfo_sync)
            .log("LFO Rate", 0.01, 40.0, " Hz", 2, |s| &mut s.lfo_rate)
            .values("LFO Sync Rate", &autofilter::SYNC_RATES, |s| &mut s.lfo_beats)
            .choice("LFO Shape", &tape::LFO_SHAPES, |s| &mut s.lfo_shape)
            .pct("LFO Slew", |s| &mut s.lfo_slew)
            .pct("LFO Drift", |s| &mut s.lfo_drift)
            .float("LFO Phase", 0.0, 360.0, " deg", 0, |s| &mut s.lfo_phase)
            .toggle("Seq On", |s| &mut s.seq_on)
            .pct_range("Seq Amount", -1.0, 1.0, |s| &mut s.seq_amount)
            .count("Seq Length", autofilter::STEPS as u8, |s| &mut s.seq_len)
            .values("Seq Rate", &autofilter::SYNC_RATES, |s| &mut s.seq_beats)
            .log("Seq Glide", 1.0, 2000.0, " ms", 0, |s| &mut s.seq_glide)
            .choice("Seq Direction", &autofilter::DIRECTIONS, |s| &mut s.seq_dir)
            .toggle("Seq Drift", |s| &mut s.seq_drift);
        for i in 0..autofilter::STEPS {
            l = l.pct_range(&format!("Step {} Value", i + 1), -1.0, 1.0, move |s| &mut s.seq_steps[i]);
        }
        for i in 0..autofilter::STEPS {
            l = l.bit(&format!("Step {} Gate", i + 1), i, |s| &mut s.seq_gate)
                .bit(&format!("Step {} Slide", i + 1), i, |s| &mut s.seq_slide)
                .bit(&format!("Step {} Accent", i + 1), i, |s| &mut s.seq_accent);
        }
        l
    });

effect!("spacer", ClipSpacer, "Clip Spacer", b"JohnnyClipSpacer", [Dynamics], spacer::Settings, Device::Spacer,
    spacer::Settings::new(0), |l| l
        .pct("Amount", |s| &mut s.amount)
        .log("Low Cut", 20.0, 2000.0, " Hz", 0, |s| &mut s.low_cut)
        .log("High Cut", 500.0, 20000.0, " Hz", 0, |s| &mut s.high_cut)
        .log("Attack", 0.5, 100.0, " ms", 1, |s| &mut s.attack)
        .log("Release", 10.0, 1000.0, " ms", 0, |s| &mut s.release)
        .choice("Stereo", &["Linked", "L / R", "M / S"], |s| &mut s.stereo)
        .float("Output", -12.0, 12.0, " dB", 1, |s| &mut s.output), true);
