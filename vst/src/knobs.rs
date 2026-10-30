//! A plugin's controls, described in a table rather than a params struct, so that each effect
//! is a short list and one generic plugin serves them all.

use std::sync::Arc;

use nih_plug::prelude::*;

/// A live host parameter.
pub enum Knob {
    Float(FloatParam),
    Int(IntParam),
    Bool(BoolParam),
}

impl Knob {
    pub fn ptr(&self) -> ParamPtr {
        match self {
            Knob::Float(p) => p.as_ptr(),
            Knob::Int(p) => p.as_ptr(),
            Knob::Bool(p) => p.as_ptr(),
        }
    }

    /// Its value `n` frames on: gliding if it's a float, in raw units.
    pub fn next(&self, n: usize) -> f32 {
        match self {
            Knob::Float(p) => p.smoothed.next_step(n as u32),
            Knob::Int(p) => p.value() as f32,
            Knob::Bool(p) => p.value() as u32 as f32,
        }
    }
}

enum Kind {
    Float { min: f32, max: f32, log: bool, scale: f32, unit: &'static str, digits: usize },
    Choice(Vec<String>),
    Toggle,
}

/// One control: how it looks to the host, and how its value lands in the settings.
pub struct Spec<S> {
    name: String,
    default: f32,
    kind: Kind,
    put: Box<dyn Fn(&mut S, f32) + Send + Sync>,
}

impl<S> Spec<S> {
    pub fn put(&self, s: &mut S, v: f32) {
        (self.put)(s, v);
    }

    pub fn id(&self) -> String {
        self.name.to_lowercase().replace(' ', "_")
    }

    pub fn knob(&self) -> Knob {
        match &self.kind {
            Kind::Float { min, max, log, scale, unit, digits } => {
                let (min, max) = (*min, *max);
                let range = if *log { FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) } } else { FloatRange::Linear { min, max } };
                let (scale, unit, digits) = (*scale, *unit, *digits);
                Knob::Float(
                    FloatParam::new(self.name.clone(), self.default, range)
                        .with_smoother(SmoothingStyle::Linear(20.0))
                        .with_value_to_string(Arc::new(move |v| format!("{:.*}{unit}", digits, v * scale)))
                        .with_string_to_value(Arc::new(move |t| t.trim().trim_end_matches(unit.trim()).trim().parse::<f32>().ok().map(|v| v / scale))),
                )
            }
            Kind::Choice(names) => {
                let shown = names.clone();
                let parsed = names.clone();
                Knob::Int(
                    IntParam::new(self.name.clone(), self.default as i32, IntRange::Linear { min: 0, max: names.len() as i32 - 1 })
                        .with_value_to_string(Arc::new(move |v| shown.get(v as usize).cloned().unwrap_or_default()))
                        .with_string_to_value(Arc::new(move |t| parsed.iter().position(|n| n.eq_ignore_ascii_case(t.trim())).map(|i| i as i32))),
                )
            }
            Kind::Toggle => Knob::Bool(BoolParam::new(self.name.clone(), self.default != 0.0)),
        }
    }
}

/// The controls so far, with the settings they start from.
pub struct List<S> {
    pub specs: Vec<Spec<S>>,
    base: S,
}

impl<S: Copy + 'static> List<S> {
    pub fn new(base: S) -> Self {
        List { specs: Vec::new(), base }
    }

    fn add(mut self, name: &str, default: f32, kind: Kind, put: impl Fn(&mut S, f32) + Send + Sync + 'static) -> Self {
        self.specs.push(Spec { name: name.to_string(), default, kind, put: Box::new(put) });
        self
    }

    fn field(self, name: &str, kind: Kind, get: impl Fn(&mut S) -> &mut f32 + Send + Sync + 'static) -> Self {
        let mut base = self.base;
        let default = *get(&mut base);
        self.add(name, default, kind, move |s, v| *get(s) = v)
    }

    /// 0..1 shown as a percentage.
    pub fn pct(self, name: &str, get: impl Fn(&mut S) -> &mut f32 + Send + Sync + 'static) -> Self {
        self.pct_range(name, 0.0, 1.0, get)
    }

    pub fn pct_range(self, name: &str, min: f32, max: f32, get: impl Fn(&mut S) -> &mut f32 + Send + Sync + 'static) -> Self {
        self.field(name, Kind::Float { min, max, log: false, scale: 100.0, unit: " %", digits: 0 }, get)
    }

    pub fn float(self, name: &str, min: f32, max: f32, unit: &'static str, digits: usize, get: impl Fn(&mut S) -> &mut f32 + Send + Sync + 'static) -> Self {
        self.field(name, Kind::Float { min, max, log: false, scale: 1.0, unit, digits }, get)
    }

    /// Swept in octaves rather than evenly, as frequencies and times want.
    pub fn log(self, name: &str, min: f32, max: f32, unit: &'static str, digits: usize, get: impl Fn(&mut S) -> &mut f32 + Send + Sync + 'static) -> Self {
        self.field(name, Kind::Float { min, max, log: true, scale: 1.0, unit, digits }, get)
    }

    /// A multiple of something else, 1 in the middle; `put` gets the multiple.
    pub fn times(self, name: &str, min: f32, max: f32, put: impl Fn(&mut S, f32) + Send + Sync + 'static) -> Self {
        self.add(name, 1.0, Kind::Float { min, max, log: true, scale: 1.0, unit: " x", digits: 2 }, put)
    }

    pub fn toggle(self, name: &str, get: impl Fn(&mut S) -> &mut bool + Send + Sync + 'static) -> Self {
        let mut base = self.base;
        let default = *get(&mut base) as u32 as f32;
        self.add(name, default, Kind::Toggle, move |s, v| *get(s) = v != 0.0)
    }

    /// A bit of a step mask.
    pub fn bit(self, name: &str, bit: usize, get: impl Fn(&mut S) -> &mut u16 + Send + Sync + 'static) -> Self {
        let mut base = self.base;
        let default = (*get(&mut base) >> bit & 1) as f32;
        self.add(name, default, Kind::Toggle, move |s, v| {
            let m = get(s);
            *m = *m & !(1 << bit) | ((v != 0.0) as u16) << bit;
        })
    }

    /// One of `names`, by index.
    pub fn choice(self, name: &str, names: &[&str], get: impl Fn(&mut S) -> &mut u8 + Send + Sync + 'static) -> Self {
        let mut base = self.base;
        let default = *get(&mut base) as f32;
        self.add(name, default, Kind::Choice(names.iter().map(|n| n.to_string()).collect()), move |s, v| *get(s) = v as u8)
    }

    /// A count from 1 to `max`.
    pub fn count(self, name: &str, max: u8, get: impl Fn(&mut S) -> &mut u8 + Send + Sync + 'static) -> Self {
        let mut base = self.base;
        let default = *get(&mut base) as f32 - 1.0;
        self.add(name, default, Kind::Choice((1..=max).map(|n| n.to_string()).collect()), move |s, v| *get(s) = v as u8 + 1)
    }

    /// One of a list of named values; starts on the one nearest the setting.
    pub fn values(self, name: &str, values: &'static [(&str, f32)], get: impl Fn(&mut S) -> &mut f32 + Send + Sync + 'static) -> Self {
        let mut base = self.base;
        let was = *get(&mut base);
        let nearest = (0..values.len()).min_by(|&a, &b| (values[a].1 - was).abs().total_cmp(&(values[b].1 - was).abs())).unwrap_or(0);
        self.add(name, nearest as f32, Kind::Choice(values.iter().map(|v| v.0.to_string()).collect()), move |s, v| *get(s) = values[v as usize].1)
    }
}
