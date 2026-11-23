//! Fixed-polyphony wavetable instrument. Tables and patch coefficients are prepared outside the callback.
use crate::{
    midi::{NoteEvent, NoteEventKind},
    sequencer::TRACKS,
};
use serde::{Deserialize, Serialize};
use std::{
    f64::consts::{PI, TAU},
    num::NonZeroU32,
};

pub const MAX_VOICES: usize = 16;
const TABLE_SIZE: usize = 2048;
const BANDS: usize = 11;
const VOICE_GAIN: f32 = 0.2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    #[default]
    Sine,
    Square,
    Triangle,
    Saw,
}
impl Waveform {
    pub const ALL: [Self; 4] = [Self::Sine, Self::Square, Self::Triangle, Self::Saw];
    pub fn name(self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Saw => "Saw",
        }
    }
    fn index(self) -> usize {
        match self {
            Self::Sine => 0,
            Self::Square => 1,
            Self::Triangle => 2,
            Self::Saw => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthPatch {
    pub waveform: Waveform,
    pub attack_ms: f32,
    pub decay_ms: f32,
    pub sustain: f32,
    pub release_ms: f32,
    pub cutoff_hz: f32,
}
impl Default for SynthPatch {
    fn default() -> Self {
        Self {
            waveform: Waveform::Sine,
            attack_ms: 5.0,
            decay_ms: 100.0,
            sustain: 0.65,
            release_ms: 180.0,
            cutoff_hz: 12_000.0,
        }
    }
}
impl SynthPatch {
    fn valid(self) -> bool {
        [self.attack_ms, self.decay_ms, self.release_ms]
            .iter()
            .all(|n| n.is_finite() && (0.0..=5000.0).contains(n))
            && self.sustain.is_finite()
            && (0.0..=1.0).contains(&self.sustain)
            && self.cutoff_hz.is_finite()
            && (20.0..=20_000.0).contains(&self.cutoff_hz)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthState {
    pub tracks: [SynthPatch; TRACKS],
}
impl SynthState {
    pub fn validate(self) -> Result<(), String> {
        if self.tracks.iter().all(|p| p.valid()) {
            Ok(())
        } else {
            Err("Parámetros del sintetizador no válidos".into())
        }
    }
    pub fn prepare(self, rate: NonZeroU32) -> Result<PreparedSynthState, String> {
        self.validate()?;
        if !(8_000..=192_000).contains(&rate.get()) {
            return Err("Sample rate del sintetizador no válido".into());
        }
        Ok(PreparedSynthState {
            rate,
            tracks: self.tracks.map(|p| PreparedPatch {
                waveform: p.waveform,
                attack: (p.attack_ms as f64 * f64::from(rate.get()) / 1000.0).round() as u32,
                decay: (p.decay_ms as f64 * f64::from(rate.get()) / 1000.0).round() as u32,
                sustain: p.sustain,
                release: (p.release_ms as f64 * f64::from(rate.get()) / 1000.0).round() as u32,
                filter: (1.0
                    - (-TAU * f64::from(p.cutoff_hz.min(rate.get() as f32 * 0.45))
                        / f64::from(rate.get()))
                    .exp()) as f32,
            }),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct PreparedPatch {
    waveform: Waveform,
    attack: u32,
    decay: u32,
    sustain: f32,
    release: u32,
    filter: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedSynthState {
    rate: NonZeroU32,
    tracks: [PreparedPatch; TRACKS],
}

pub fn midi_frequency(pitch: u8) -> f64 {
    440.0 * 2.0_f64.powf((f64::from(pitch) - 69.0) / 12.0)
}

struct WaveTables {
    tables: Vec<Box<[f32]>>,
    increments: [f64; 128],
}
impl WaveTables {
    fn prepare(rate: NonZeroU32) -> Self {
        let mut tables = Vec::with_capacity(BANDS * 4);
        for band in 0..BANDS {
            let highest = (band * 12 + 11).min(127) as u8;
            let limit = f64::from(rate.get()) * 0.45;
            let harmonics =
                ((limit / midi_frequency(highest).min(limit)).floor() as usize).min(256);
            let mut waves: [Vec<f32>; 4] = std::array::from_fn(|_| vec![0.0; TABLE_SIZE + 1]);
            for i in 0..TABLE_SIZE {
                let angle = TAU * i as f64 / TABLE_SIZE as f64;
                let mut values = [0.0_f64; 4];
                if harmonics > 0 {
                    values[0] = angle.sin();
                }
                for k in 1..=harmonics {
                    let sine = (angle * k as f64).sin();
                    values[3] -= 2.0 / PI * sine / k as f64;
                    if k % 2 == 1 {
                        values[1] += 4.0 / PI * sine / k as f64;
                        let sign = if k % 4 == 1 { 1.0 } else { -1.0 };
                        values[2] += 8.0 / (PI * PI) * sign * sine / (k * k) as f64;
                    }
                }
                for (wave, value) in waves.iter_mut().zip(values) {
                    wave[i] = value as f32;
                }
            }
            for mut wave in waves {
                let peak = wave.iter().fold(1.0_f32, |p, v| p.max(v.abs()));
                for value in &mut wave {
                    *value /= peak;
                }
                wave[TABLE_SIZE] = wave[0];
                tables.push(wave.into_boxed_slice());
            }
        }
        Self {
            tables,
            increments: std::array::from_fn(|pitch| {
                midi_frequency(pitch as u8) / f64::from(rate.get())
            }),
        }
    }
    fn sample(&self, waveform: Waveform, pitch: u8, phase: f64) -> f32 {
        if self.increments[usize::from(pitch)] >= 0.45 {
            return 0.0;
        }
        let table = &self.tables[usize::from(pitch / 12) * 4 + waveform.index()];
        let offset = phase * TABLE_SIZE as f64;
        let index = (offset as usize).min(TABLE_SIZE - 1);
        table[index] + (table[index + 1] - table[index]) * (offset - index as f64) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
    Idle,
}
#[derive(Clone, Copy)]
struct Envelope {
    stage: Stage,
    level: f32,
    remaining: u32,
    delta: f32,
    patch: PreparedPatch,
}
impl Envelope {
    fn new(patch: PreparedPatch) -> Self {
        let mut env = Self {
            stage: Stage::Attack,
            level: 0.0,
            remaining: patch.attack,
            delta: 1.0 / patch.attack.max(1) as f32,
            patch,
        };
        if patch.attack == 0 {
            env.decay();
        }
        env
    }
    fn decay(&mut self) {
        self.level = 1.0;
        self.stage = Stage::Decay;
        self.remaining = self.patch.decay;
        self.delta = (self.patch.sustain - 1.0) / self.patch.decay.max(1) as f32;
        if self.remaining == 0 {
            self.stage = Stage::Sustain;
            self.level = self.patch.sustain;
        }
    }
    fn release(&mut self) {
        if matches!(self.stage, Stage::Idle | Stage::Release) {
            return;
        }
        self.stage = Stage::Release;
        self.remaining = self.patch.release;
        self.delta = -self.level / self.patch.release.max(1) as f32;
        if self.remaining == 0 {
            self.stage = Stage::Idle;
            self.level = 0.0;
        }
    }
    fn next(&mut self) -> f32 {
        let value = self.level.clamp(0.0, 1.0);
        if self.remaining > 0 {
            self.level += self.delta;
            self.remaining -= 1;
            if self.remaining == 0 {
                match self.stage {
                    Stage::Attack => self.decay(),
                    Stage::Decay => {
                        self.stage = Stage::Sustain;
                        self.level = self.patch.sustain;
                    }
                    Stage::Release => {
                        self.stage = Stage::Idle;
                        self.level = 0.0;
                    }
                    _ => {}
                }
            }
        }
        value
    }
}
#[derive(Clone, Copy)]
struct Voice {
    id: u32,
    track: usize,
    pitch: u8,
    velocity: f32,
    phase: f64,
    filter: f32,
    env: Envelope,
    serial: u64,
    last: f32,
}
#[derive(Clone, Copy)]
struct Tail {
    track: usize,
    value: f32,
    remaining: u32,
}

pub struct Synth {
    state: PreparedSynthState,
    tables: WaveTables,
    voices: [Option<Voice>; MAX_VOICES],
    tails: [Option<Tail>; MAX_VOICES],
    serial: u64,
    fade_frames: u32,
}
impl Synth {
    pub fn new(rate: NonZeroU32, state: SynthState) -> Result<Self, String> {
        Ok(Self {
            state: state.prepare(rate)?,
            tables: WaveTables::prepare(rate),
            voices: [None; MAX_VOICES],
            tails: [None; MAX_VOICES],
            serial: 0,
            fade_frames: (rate.get() / 200).max(1),
        })
    }
    pub fn reset(&mut self) {
        self.voices.fill(None);
        self.tails.fill(None);
        self.serial = 0;
    }
    pub fn set_state(&mut self, state: PreparedSynthState) {
        if state.rate == self.state.rate {
            self.state = state;
        }
    }
    pub fn active_voices(&self) -> u32 {
        self.voices.iter().flatten().count() as u32
    }
    pub fn release_all(&mut self) {
        for voice in self.voices.iter_mut().flatten() {
            voice.env.release();
        }
    }
    pub fn handle(&mut self, event: NoteEvent) {
        if event.track >= TRACKS || event.pitch > 127 {
            return;
        }
        if event.kind == NoteEventKind::Off {
            for voice in self.voices.iter_mut().flatten() {
                if voice.id == event.id {
                    voice.env.release();
                }
            }
            return;
        }
        if event.velocity == 0 || event.velocity > 127 {
            return;
        }
        let slot = self
            .voices
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.map(|v| (v.env.stage != Stage::Release, v.serial)))
                    .map_or(0, |(i, _)| i)
            });
        if let Some(old) = self.voices[slot] {
            self.tails[slot] = Some(Tail {
                track: old.track,
                value: old.last,
                remaining: self.fade_frames,
            });
        }
        self.serial = self.serial.saturating_add(1);
        self.voices[slot] = Some(Voice {
            id: event.id,
            track: event.track,
            pitch: event.pitch,
            velocity: f32::from(event.velocity) / 127.0,
            phase: 0.0,
            filter: 0.0,
            env: Envelope::new(self.state.tracks[event.track]),
            serial: self.serial,
            last: 0.0,
        });
    }
    pub fn next_samples(&mut self) -> [f32; TRACKS] {
        let mut tracks = [0.0; TRACKS];
        for (slot, voice) in self.voices.iter_mut().enumerate() {
            if let Some(v) = voice {
                let raw = self.tables.sample(v.env.patch.waveform, v.pitch, v.phase);
                v.phase = (v.phase + self.tables.increments[usize::from(v.pitch)]).fract();
                v.filter += v.env.patch.filter * (raw - v.filter);
                // Avoid subnormal filter tails without changing audible values.
                if v.filter.abs() < 1e-20 {
                    v.filter = 0.0;
                }
                v.last = v.filter * v.env.next() * v.velocity * VOICE_GAIN;
                tracks[v.track] += v.last;
                if v.env.stage == Stage::Idle {
                    *voice = None;
                }
            }
            if let Some(tail) = &mut self.tails[slot] {
                tracks[tail.track] += tail.value * tail.remaining as f32 / self.fade_frames as f32;
                tail.remaining -= 1;
                if tail.remaining == 0 {
                    self.tails[slot] = None;
                }
            }
        }
        tracks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rate() -> NonZeroU32 {
        NonZeroU32::new(48_000).unwrap()
    }
    fn event(id: u32, pitch: u8, kind: NoteEventKind) -> NoteEvent {
        NoteEvent {
            tick: 0,
            id,
            track: 0,
            pitch,
            velocity: 127,
            kind,
        }
    }
    #[test]
    fn adsr_hits_segment_boundaries_and_releases_from_the_current_level() {
        let patch = SynthPatch {
            attack_ms: 10.0,
            decay_ms: 20.0,
            sustain: 0.25,
            release_ms: 40.0,
            ..Default::default()
        };
        let mut env = Envelope::new(
            SynthState {
                tracks: [patch; TRACKS],
            }
            .prepare(rate())
            .unwrap()
            .tracks[0],
        );
        assert_eq!(env.next(), 0.0);
        for _ in 1..480 {
            env.next();
        }
        assert_eq!(env.level, 1.0);
        for _ in 0..960 {
            env.next();
        }
        assert_eq!(env.level, 0.25);
        env.release();
        assert_eq!(env.next(), 0.25);
        for _ in 1..1920 {
            env.next();
        }
        assert_eq!(env.stage, Stage::Idle);
        assert_eq!(env.next(), 0.0);
        let mut early = Envelope::new(env.patch);
        for _ in 0..240 {
            early.next();
        }
        let before = early.level;
        early.release();
        assert_eq!(early.next(), before);
        for _ in 1..1920 {
            early.next();
        }
        assert_eq!(early.stage, Stage::Idle);
        let zero = SynthPatch {
            attack_ms: 0.0,
            decay_ms: 0.0,
            sustain: 0.7,
            release_ms: 0.0,
            ..patch
        };
        let mut env = Envelope::new(
            SynthState {
                tracks: [zero; TRACKS],
            }
            .prepare(rate())
            .unwrap()
            .tracks[0],
        );
        assert_eq!(env.next(), 0.7);
        env.release();
        assert_eq!(env.next(), 0.0);
    }
    #[test]
    fn waveforms_are_distinct_bounded_periodic_and_have_expected_harmonics() {
        let tables = WaveTables::prepare(rate());
        let pitch = 69;
        let mut spectra = Vec::new();
        for waveform in Waveform::ALL {
            let samples: Vec<_> = (0..TABLE_SIZE)
                .map(|i| tables.sample(waveform, pitch, i as f64 / TABLE_SIZE as f64))
                .collect();
            assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.00001));
            let harmonic = |k: usize| {
                (samples
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        f64::from(*s) * (TAU * k as f64 * i as f64 / TABLE_SIZE as f64).sin()
                    })
                    .sum::<f64>()
                    * 2.0
                    / TABLE_SIZE as f64)
                    .abs()
            };
            assert!(harmonic(1) > 0.5);
            if waveform == Waveform::Saw {
                assert!((harmonic(2) / harmonic(1) - 0.5).abs() < 1e-5);
            }
            spectra.push([harmonic(2), harmonic(3)]);
        }
        assert!(spectra[0][0] < 1e-6 && spectra[0][1] < 1e-6);
        assert!(spectra[1][0] < 1e-6 && spectra[1][1] > 0.2);
        assert!(spectra[2][0] < 1e-6 && spectra[2][1] > 0.05 && spectra[2][1] < 0.15);
        assert!(spectra[3][0] > 0.2);
        assert!((midi_frequency(69) - 440.0).abs() < 1e-9);
        assert!((midi_frequency(81) - 880.0).abs() < 1e-9);
    }
    #[test]
    fn voices_overlap_steal_deterministically_and_old_note_off_cannot_kill_replacement() {
        let mut synth = Synth::new(rate(), SynthState::default()).unwrap();
        for id in 1..=MAX_VOICES as u32 {
            synth.handle(event(id, 60, NoteEventKind::On));
        }
        for _ in 0..400 {
            synth.next_samples();
        }
        assert_eq!(synth.active_voices(), MAX_VOICES as u32);
        synth.handle(event(100, 67, NoteEventKind::On));
        assert!(!synth.voices.iter().flatten().any(|v| v.id == 1));
        synth.handle(event(1, 60, NoteEventKind::Off));
        assert!(
            synth
                .voices
                .iter()
                .flatten()
                .any(|v| v.id == 100 && v.env.stage != Stage::Release)
        );
        synth.release_all();
        for _ in 0..9000 {
            let values = synth.next_samples();
            assert!(values.iter().all(|v| v.is_finite()));
        }
        assert_eq!(synth.active_voices(), 0);
        assert_eq!(synth.next_samples(), [0.0; TRACKS]);
    }
    #[test]
    fn invalid_settings_and_sample_rates_are_rejected() {
        for field in 0..5 {
            let mut state = SynthState::default();
            match field {
                0 => state.tracks[0].attack_ms = f32::NAN,
                1 => state.tracks[0].decay_ms = -1.0,
                2 => state.tracks[0].sustain = 1.1,
                3 => state.tracks[0].release_ms = 5001.0,
                _ => state.tracks[0].cutoff_hz = f32::INFINITY,
            }
            assert!(state.prepare(rate()).is_err());
        }
        assert!(
            SynthState::default()
                .prepare(NonZeroU32::new(1000).unwrap())
                .is_err()
        );
    }

    #[test]
    fn cutoff_attenuates_high_frequencies_and_velocity_scales_amplitude() {
        let mut synth = Synth::new(rate(), SynthState::default()).unwrap();
        let mut patch = SynthPatch {
            attack_ms: 0.0,
            decay_ms: 0.0,
            sustain: 1.0,
            release_ms: 10.0,
            ..Default::default()
        };
        fn rms(synth: &mut Synth, patch: SynthPatch, velocity: u8) -> f64 {
            synth.reset();
            synth.set_state(
                SynthState {
                    tracks: [patch; TRACKS],
                }
                .prepare(rate())
                .unwrap(),
            );
            let mut on = event(1, 69, NoteEventKind::On);
            on.velocity = velocity;
            synth.handle(on);
            for _ in 0..4000 {
                synth.next_samples();
            }
            ((0..4800)
                .map(|_| f64::from(synth.next_samples()[0]).powi(2))
                .sum::<f64>()
                / 4800.0)
                .sqrt()
        }
        let loud = rms(&mut synth, patch, 127);
        assert!(loud > 0.1);
        let soft = rms(&mut synth, patch, 64);
        assert!((soft / loud - 64.0 / 127.0).abs() < 1e-5);
        patch.cutoff_hz = 40.0;
        assert!(rms(&mut synth, patch, 127) < loud * 0.15);
    }

    #[test]
    fn identical_pitches_release_by_id_and_patch_changes_only_affect_new_notes() {
        let mut synth = Synth::new(rate(), SynthState::default()).unwrap();
        synth.handle(event(1, 60, NoteEventKind::On));
        let mut changed = SynthState::default();
        changed.tracks[0].waveform = Waveform::Square;
        synth.set_state(changed.prepare(rate()).unwrap());
        synth.handle(event(2, 60, NoteEventKind::On));
        assert!(
            synth
                .voices
                .iter()
                .flatten()
                .any(|v| v.id == 1 && v.env.patch.waveform == Waveform::Sine)
        );
        assert!(
            synth
                .voices
                .iter()
                .flatten()
                .any(|v| v.id == 2 && v.env.patch.waveform == Waveform::Square)
        );
        for _ in 0..500 {
            synth.next_samples();
        }
        synth.handle(event(1, 60, NoteEventKind::Off));
        for _ in 0..9000 {
            synth.next_samples();
        }
        assert_eq!(synth.active_voices(), 1);
        assert_eq!(synth.voices.iter().flatten().next().unwrap().id, 2);
        synth.reset();
        assert_eq!(synth.next_samples(), [0.0; TRACKS]);
    }

    #[test]
    fn low_sample_rates_keep_supported_notes_and_silence_notes_above_the_guard_band() {
        let tables = WaveTables::prepare(NonZeroU32::new(8_000).unwrap());
        for wave in Waveform::ALL {
            assert!(tables.sample(wave, 96, 0.25).abs() > 0.1);
            assert_eq!(tables.sample(wave, 127, 0.25), 0.0);
        }
    }
}
