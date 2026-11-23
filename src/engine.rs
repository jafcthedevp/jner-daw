//! Device- and GUI-independent audio state. One call to `next_sample` is one frame.
use std::{f64::consts::TAU, num::NonZeroU32};

use crate::{
    automation::{Automation, AutomationPlayer, Values as AutomationValues},
    effects::{Effects, EffectsState, PreparedEffectsState},
    midi::MidiPlayer,
    mixer::{Mixer, MixerState, StereoFrame},
    samples::SampleBank,
    sequencer::{Bpm, Pattern, StepClock, TRACKS},
    synth::{PreparedSynthState, Synth, SynthState},
    timeline::{MusicalClock, TimelinePlayer},
};

pub const TONE_HZ: f64 = 440.0;
pub const DEFAULT_GAIN: f32 = 0.08;
pub const MAX_GAIN: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Play,
    Stop,
    SetGain(f32),
    SetBpm(Bpm),
    SetPattern(Pattern),
    SetMixer(MixerState),
    SetSynth(PreparedSynthState),
    SetEffects(PreparedEffectsState),
}

pub struct Engine {
    sample_rate: NonZeroU32,
    position: u64,
    playing: bool,
    phase: f64,
    gain: f32,
    amplitude: f32,
    ramp_step: f32,
    drums: Option<DrumMachine>,
    mixer: Mixer,
    musical_clock: MusicalClock,
    timeline: Option<TimelinePlayer>,
    midi: Option<MidiPlayer>,
    synth: Option<Synth>,
    effects: Option<Effects>,
    automation: Option<AutomationPlayer>,
    automation_values: AutomationValues,
}

impl Engine {
    pub fn new(sample_rate: NonZeroU32) -> Self {
        Self {
            sample_rate,
            position: 0,
            playing: false,
            phase: 0.0,
            gain: DEFAULT_GAIN,
            amplitude: 0.0,
            // At most 5 ms from silence to the maximum test-tone level.
            ramp_step: MAX_GAIN / (sample_rate.get() as f32 * 0.005).max(1.0),
            drums: None,
            mixer: Mixer::new(sample_rate),
            musical_clock: MusicalClock::new(sample_rate, Bpm::default()),
            timeline: None,
            midi: None,
            synth: None,
            effects: None,
            automation: None,
            automation_values: AutomationValues::default(),
        }
    }

    pub fn with_drums(bank: SampleBank, pattern: Pattern, bpm: Bpm) -> Self {
        let mut engine = Self::new(bank.rate);
        engine.musical_clock.set_bpm(bpm);
        engine.drums = Some(DrumMachine {
            clock: StepClock::new(bank.rate, bpm),
            bank,
            pattern,
            voices: [[None; VOICES_PER_TRACK]; TRACKS],
            next_voice: [0; TRACKS],
        });
        engine
    }

    pub fn with_timeline(rate: NonZeroU32, player: TimelinePlayer, bpm: Bpm) -> Self {
        let mut engine = Self::new(rate);
        engine.musical_clock.set_bpm(bpm);
        engine.timeline = Some(player);
        engine
    }

    pub fn with_midi(rate: NonZeroU32, player: MidiPlayer, bpm: Bpm) -> Self {
        let mut engine = Self::new(rate);
        engine.musical_clock.set_bpm(bpm);
        engine.midi = Some(player);
        engine
    }

    pub fn note_counts(&self) -> (u64, u64, u32) {
        self.midi.as_ref().map_or((0, 0, 0), MidiPlayer::counts)
    }

    pub fn with_synth(
        rate: NonZeroU32,
        player: MidiPlayer,
        bpm: Bpm,
        state: SynthState,
    ) -> Result<Self, String> {
        let mut engine = Self::with_midi(rate, player, bpm);
        engine.synth = Some(Synth::new(rate, state)?);
        Ok(engine)
    }
    pub fn synth_voices(&self) -> u32 {
        self.synth.as_ref().map_or(0, Synth::active_voices)
    }

    /// Prepare delay storage on the caller thread, before this engine enters a stream.
    pub fn configure_effects(&mut self, state: EffectsState) -> Result<(), String> {
        self.effects = Some(Effects::new(self.sample_rate, state)?);
        Ok(())
    }
    pub fn configure_automation(&mut self, state: &Automation) -> Result<(), String> {
        self.automation = Some(AutomationPlayer::prepare(state)?);
        Ok(())
    }
    pub fn automation_values(&self) -> AutomationValues {
        self.automation_values
    }

    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Play if !self.playing => {
                self.position = 0;
                self.phase = 0.0;
                self.amplitude = 0.0;
                self.playing = true;
                self.musical_clock.reset();
                if let Some(automation) = &mut self.automation {
                    automation.reset();
                }
                self.automation_values = self
                    .automation
                    .as_mut()
                    .map_or_else(AutomationValues::default, |a| a.evaluate(0.0));
                self.mixer
                    .start_automation(self.automation_values.gains, self.automation_values.pans);
                if let Some(effects) = &mut self.effects {
                    effects.reset();
                }
                if let Some(synth) = &mut self.synth {
                    synth.reset();
                }
                if let Some(midi) = &mut self.midi {
                    midi.reset();
                }
                if let Some(timeline) = &mut self.timeline {
                    timeline.reset();
                }
                if let Some(drums) = &mut self.drums {
                    drums.reset();
                }
            }
            Command::Play => {}
            Command::Stop => {
                self.playing = false;
                self.position = 0;
                self.musical_clock.reset();
                if let Some(automation) = &mut self.automation {
                    automation.reset();
                }
                self.automation_values = AutomationValues::default();
                self.mixer.reset_automation();
                if let Some(synth) = &mut self.synth {
                    synth.release_all();
                }
                if let Some(midi) = &mut self.midi {
                    midi.reset();
                }
                if let Some(drums) = &mut self.drums {
                    drums.clock.reset();
                }
                // Keep oscillator phase for a short release; reset once silent.
            }
            Command::SetGain(gain) if gain.is_finite() => {
                self.gain = gain.clamp(0.0, MAX_GAIN);
            }
            Command::SetGain(_) => {}
            Command::SetBpm(bpm) => {
                self.musical_clock.set_bpm(bpm);
                if let Some(drums) = &mut self.drums {
                    drums.clock.set_bpm(bpm);
                }
            }
            Command::SetPattern(pattern) => {
                if let Some(drums) = &mut self.drums {
                    drums.pattern = pattern;
                }
            }
            Command::SetMixer(state) => {
                self.mixer.set_state(state);
            }
            Command::SetSynth(state) => {
                if let Some(synth) = &mut self.synth {
                    synth.set_state(state);
                }
            }
            Command::SetEffects(state) => {
                if let Some(effects) = &mut self.effects {
                    effects.set_state(state);
                }
            }
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        let [left, right] = self.next_frame();
        (left + right) * 0.5
    }

    pub fn next_frame(&mut self) -> StereoFrame {
        self.automation_values = if self.playing {
            self.automation
                .as_mut()
                .map_or_else(AutomationValues::default, |a| {
                    a.evaluate(self.musical_clock.position())
                })
        } else {
            AutomationValues::default()
        };
        let automation = self.automation_values;
        let target = if self.playing { self.gain } else { 0.0 };
        self.amplitude += (target - self.amplitude).clamp(-self.ramp_step, self.ramp_step);
        let source = if let Some(midi) = &mut self.midi {
            if self.playing {
                midi.advance(self.musical_clock.ticks(), |event| {
                    if let Some(synth) = &mut self.synth {
                        synth.handle(event);
                    }
                });
            }
            let tracks = if let Some(synth) = &mut self.synth {
                if self.playing || self.amplitude > 0.0 {
                    synth.next_samples()
                } else {
                    synth.reset();
                    [0.0; TRACKS]
                }
            } else {
                [0.0; TRACKS]
            };
            self.mixer
                .process_automated(tracks, automation.gains, automation.pans)
        } else if let Some(timeline) = &mut self.timeline {
            let tracks = if self.playing || self.amplitude > 0.0 {
                timeline.next_samples(self.musical_clock.ticks(), self.playing)
            } else {
                timeline.reset();
                [0.0; TRACKS]
            };
            self.mixer
                .process_automated(tracks, automation.gains, automation.pans)
        } else if let Some(drums) = &mut self.drums {
            let tracks = if self.playing || self.amplitude > 0.0 {
                drums.next_samples(self.playing)
            } else {
                drums.clear_voices();
                [0.0; TRACKS]
            };
            self.mixer
                .process_automated(tracks, automation.gains, automation.pans)
        } else {
            let sample = self.phase.sin() as f32;
            if self.playing || self.amplitude > 0.0 {
                self.phase = (self.phase + TAU * TONE_HZ / self.sample_rate.get() as f64) % TAU;
            } else {
                self.phase = 0.0;
            }
            [sample; 2]
        };
        let source = if let Some(effects) = &mut self.effects {
            if self.playing || self.amplitude > 0.0 {
                effects.process_automated(source, automation.delay_mix)
            } else {
                effects.reset();
                [0.0; 2]
            }
        } else {
            source
        };
        let frame = source.map(|sample| (sample * self.amplitude).clamp(-1.0, 1.0));
        if self.playing {
            self.position = self.position.saturating_add(1);
            self.musical_clock.advance();
        }
        frame
    }

    pub fn position(&self) -> u64 {
        self.position
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn current_step(&self) -> Option<u8> {
        if !self.playing {
            None
        } else {
            self.drums.as_ref().map(|d| d.clock.step())
        }
    }

    pub fn step_events(&self) -> u64 {
        self.drums.as_ref().map_or(0, |d| d.clock.events())
    }

    pub fn track_levels(&self) -> [f32; TRACKS] {
        self.mixer.levels()
    }

    pub fn musical_ticks(&self) -> u64 {
        self.musical_clock.ticks()
    }
    pub fn clip_triggers(&self) -> u64 {
        self.timeline.as_ref().map_or(0, TimelinePlayer::triggers)
    }
}

const VOICES_PER_TRACK: usize = 16;

struct DrumMachine {
    bank: SampleBank,
    pattern: Pattern,
    clock: StepClock,
    voices: [[Option<usize>; VOICES_PER_TRACK]; TRACKS],
    next_voice: [usize; TRACKS],
}

impl DrumMachine {
    fn clear_voices(&mut self) {
        self.voices = [[None; VOICES_PER_TRACK]; TRACKS];
        self.next_voice = [0; TRACKS];
    }

    fn reset(&mut self) {
        self.clock.reset();
        self.clear_voices();
    }

    fn next_samples(&mut self, playing: bool) -> [f32; TRACKS] {
        if let Some(step) = playing.then(|| self.clock.tick()).flatten() {
            for track in 0..TRACKS {
                if self.pattern.active(track, usize::from(step)) {
                    let voice = self.next_voice[track];
                    self.voices[track][voice] = Some(0);
                    self.next_voice[track] = (voice + 1) % VOICES_PER_TRACK;
                }
            }
        }
        let mut mixed = [0.0; TRACKS];
        for (track, voices) in self.voices.iter_mut().enumerate() {
            let sample = &self.bank.samples[track];
            for voice in voices {
                if let Some(position) = *voice {
                    if let Some(value) = sample.get(position) {
                        mixed[track] += value;
                        *voice = (position + 1 < sample.len()).then_some(position + 1);
                    } else {
                        *voice = None;
                    }
                }
            }
        }
        mixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(rate: u32) -> Engine {
        Engine::new(NonZeroU32::new(rate).unwrap())
    }

    #[test]
    fn internal_midi_uses_the_musical_clock_and_stop_clears_notes_before_restart() {
        use crate::midi::{MidiPlayer, NoteSequence};
        let mut notes = NoteSequence::default();
        notes.add(0, 60, 100, 0, 960).unwrap();
        let mut engine = Engine::with_midi(
            NonZeroU32::new(48_000).unwrap(),
            MidiPlayer::prepare(&notes),
            Bpm::default(),
        );
        engine.apply(Command::Play);
        for _ in 0..6000 {
            assert_eq!(engine.next_frame(), [0.0, 0.0]);
        }
        assert_eq!(engine.note_counts(), (1, 0, 1));
        engine.apply(Command::SetBpm(Bpm::new(240).unwrap()));
        for _ in 0..9000 {
            engine.next_frame();
        }
        assert_eq!(engine.note_counts(), (1, 0, 1));
        engine.next_frame();
        assert_eq!(engine.note_counts(), (1, 1, 0));
        engine.apply(Command::Stop);
        assert_eq!(engine.note_counts(), (0, 0, 0));
        assert_eq!(engine.musical_ticks(), 0);
        engine.apply(Command::Play);
        engine.next_frame();
        assert_eq!(engine.note_counts(), (1, 0, 1));
        engine.apply(Command::Stop);
        for _ in 0..1000 {
            engine.next_frame();
        }
        assert_eq!(engine.note_counts(), (0, 0, 0));
    }

    #[test]
    fn effects_bypass_is_exact_live_edits_keep_clock_and_stop_clears_echo_before_restart() {
        let rate = NonZeroU32::new(48_000).unwrap();
        let mut dry = Engine::new(rate);
        let mut wet = Engine::new(rate);
        wet.configure_effects(EffectsState::default()).unwrap();
        dry.apply(Command::Play);
        wet.apply(Command::Play);
        for _ in 0..4096 {
            assert_eq!(dry.next_frame(), wet.next_frame());
        }
        let state = EffectsState {
            delay_enabled: true,
            delay_ms: 40.0,
            feedback: 0.7,
            delay_mix: 0.5,
            distortion_enabled: true,
            drive: 10.0,
            distortion_mix: 0.8,
        };
        wet.apply(Command::SetEffects(state.prepare(rate).unwrap()));
        for _ in 0..10000 {
            let frame = wet.next_frame();
            assert!(frame.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        }
        assert_eq!(wet.position(), 14096);
        wet.apply(Command::Stop);
        for _ in 0..1000 {
            wet.next_frame();
        }
        assert_eq!(wet.position(), 0);
        assert_eq!(wet.next_frame(), [0.0; 2]);
        let mut fresh = Engine::new(rate);
        fresh.configure_effects(state).unwrap();
        wet.apply(Command::Play);
        fresh.apply(Command::Play);
        for _ in 0..6000 {
            assert_eq!(wet.next_frame(), fresh.next_frame());
        }
    }

    #[test]
    fn automated_gain_and_pan_control_audio_and_preserve_manual_values_after_stop() {
        use crate::automation::Target;
        let rate = NonZeroU32::new(48_000).unwrap();
        let bank = SampleBank {
            rate,
            samples: std::array::from_fn(|_| vec![1.0; 48_000].into()),
        };
        let mut pattern = Pattern::default();
        pattern.set(0, 0, true);
        let mut engine = Engine::with_drums(bank, pattern, Bpm::default());
        let mut a = Automation::default();
        a.add(Target::TrackGain(0), 0, 1.0).unwrap();
        a.add(Target::TrackGain(0), 960, 0.0).unwrap();
        a.add(Target::TrackPan(0), 0, -1.0).unwrap();
        a.add(Target::TrackPan(0), 960, 1.0).unwrap();
        engine.configure_automation(&a).unwrap();
        engine.apply(Command::Play);
        let mut near_start = [0.0; 2];
        let mut middle = [0.0; 2];
        for i in 0..25000 {
            let frame = engine.next_frame();
            if i == 300 {
                near_start = frame;
            }
            if i == 12000 {
                middle = frame;
            }
        }
        assert!(near_start[0] > near_start[1] * 10.0);
        assert!((middle[0] - middle[1]).abs() < 0.003);
        assert_eq!(engine.next_frame(), [0.0; 2]);
        assert_eq!(engine.automation_values().gains[0], Some(0.0));
        let mut manual = MixerState::default();
        manual.tracks[0].gain = 1.5;
        engine.apply(Command::SetMixer(manual));
        for _ in 0..500 {
            assert_eq!(engine.next_frame(), [0.0; 2]);
        }
        engine.apply(Command::Stop);
        for _ in 0..1000 {
            engine.next_frame();
        }
        assert_eq!(engine.automation_values(), AutomationValues::default());
        engine.configure_automation(&Automation::default()).unwrap();
        engine.apply(Command::Play);
        let mut frame = [0.0; 2];
        for _ in 0..500 {
            frame = engine.next_frame();
        }
        assert!((frame[0] - 1.5 * DEFAULT_GAIN * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(frame[0], frame[1]);
    }

    #[test]
    fn synth_routes_through_pan_mute_and_master_and_stop_restarts_identically() {
        use crate::midi::NoteSequence;
        let mut notes = NoteSequence::default();
        notes.add(0, 69, 100, 0, 1920).unwrap();
        notes.add(1, 72, 100, 0, 1920).unwrap();
        let mut engine = Engine::with_synth(
            NonZeroU32::new(48_000).unwrap(),
            MidiPlayer::prepare(&notes),
            Bpm::default(),
            SynthState::default(),
        )
        .unwrap();
        let mut mix = MixerState::default();
        mix.tracks[0].pan = 1.0;
        mix.tracks[1].muted = true;
        engine.apply(Command::SetMixer(mix));
        for _ in 0..500 {
            assert_eq!(engine.next_frame(), [0.0, 0.0]);
        }
        engine.apply(Command::Play);
        let first: Vec<_> = (0..4096).map(|_| engine.next_frame()).collect();
        assert!(first.iter().all(|v| v[0] == 0.0 && v[1].is_finite()));
        assert!(first.iter().any(|v| v[1].abs() > 0.001));
        assert_eq!(engine.synth_voices(), 2);
        engine.apply(Command::Stop);
        for _ in 0..500 {
            engine.next_frame();
        }
        assert_eq!(engine.synth_voices(), 0);
        assert_eq!(engine.next_frame(), [0.0, 0.0]);
        engine.apply(Command::Play);
        assert_eq!(
            (0..4096).map(|_| engine.next_frame()).collect::<Vec<_>>(),
            first
        );
        engine.apply(Command::SetGain(0.0));
        for _ in 0..500 {
            engine.next_frame();
        }
        assert_eq!(engine.next_frame(), [0.0, 0.0]);
        assert_eq!(engine.note_counts().0, 2);
    }

    #[test]
    fn silence_does_not_advance_transport() {
        let mut engine = engine(48_000);
        for _ in 0..4096 {
            assert_eq!(engine.next_sample(), 0.0);
        }
        assert_eq!(engine.position(), 0);
    }

    #[test]
    fn stop_resets_clock_releases_tone_and_play_restarts_identically() {
        let mut engine = engine(48_000);
        engine.apply(Command::Play);
        let first: Vec<_> = (0..1024).map(|_| engine.next_sample()).collect();
        assert_eq!(engine.position(), 1024);
        engine.apply(Command::Play);
        assert_eq!(engine.position(), 1024, "Play must be idempotent");
        engine.apply(Command::Stop);
        assert_eq!(engine.position(), 0);
        assert!(!engine.is_playing());
        for _ in 0..241 {
            engine.next_sample();
        }
        assert_eq!(engine.next_sample(), 0.0);
        assert_eq!(engine.position(), 0);
        engine.apply(Command::Play);
        let restarted: Vec<_> = (0..1024).map(|_| engine.next_sample()).collect();
        assert_eq!(first, restarted);
    }

    #[test]
    fn tone_has_440_cycles_per_second_at_common_rates() {
        for rate in [44_100, 48_000, 96_000] {
            let mut engine = engine(rate);
            engine.apply(Command::Play);
            let mut crossings = 0;
            let mut previous = 0.0;
            for _ in 0..rate {
                let sample = engine.next_sample();
                assert!(sample.is_finite() && sample.abs() <= MAX_GAIN);
                if previous <= 0.0 && sample > 0.0 {
                    crossings += 1;
                }
                previous = sample;
            }
            assert_eq!(crossings, 440);
            assert_eq!(engine.position(), u64::from(rate));
        }
    }

    #[test]
    fn invalid_gain_cannot_poison_or_clip_output() {
        let mut engine = engine(48_000);
        for gain in [f32::NAN, f32::INFINITY, -1.0, 5.0] {
            engine.apply(Command::SetGain(gain));
            engine.apply(Command::Play);
            for _ in 0..1024 {
                let sample = engine.next_sample();
                assert!(sample.is_finite() && sample.abs() <= MAX_GAIN);
            }
        }
    }

    fn test_bank(values: [Vec<f32>; TRACKS]) -> SampleBank {
        SampleBank {
            rate: NonZeroU32::new(48_000).unwrap(),
            samples: values.map(Into::into),
        }
    }

    #[test]
    fn sample_triggers_at_exact_frame_and_live_edits_affect_future_steps() {
        let bank = test_bank([vec![1.0], vec![0.5], vec![0.25], vec![-0.5]]);
        let mut pattern = Pattern::default();
        pattern.set(0, 1, true);
        let mut engine = Engine::with_drums(bank, pattern, Bpm::default());
        engine.apply(Command::Play);
        for _ in 0..6000 {
            assert_eq!(engine.next_sample(), 0.0);
        }
        assert!(
            (engine.next_sample() - DEFAULT_GAIN * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6
        );
        pattern.set(1, 2, true);
        engine.apply(Command::SetPattern(pattern));
        for _ in 6001..12000 {
            assert_eq!(engine.next_sample(), 0.0);
        }
        assert!(
            (engine.next_sample() - DEFAULT_GAIN * 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs()
                < 1e-6
        );
        assert_eq!(engine.current_step(), Some(2));
        engine.apply(Command::Stop);
        assert_eq!(engine.current_step(), None);
        assert_eq!(engine.step_events(), 0);
        engine.apply(Command::Play);
        assert_eq!(engine.next_sample(), 0.0);
        assert_eq!(engine.current_step(), Some(0));
    }

    #[test]
    fn repeated_hits_overlap_and_stop_releases_all_voices() {
        let bank = test_bank([vec![0.5; 20_000], vec![], vec![], vec![]]);
        let mut pattern = Pattern::default();
        pattern.set(0, 0, true);
        pattern.set(0, 1, true);
        let mut engine = Engine::with_drums(bank, pattern, Bpm::default());
        engine.apply(Command::Play);
        for _ in 0..6000 {
            engine.next_sample();
        }
        assert!(
            (engine.next_sample() - DEFAULT_GAIN * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6
        );
        engine.apply(Command::Stop);
        for _ in 0..241 {
            engine.next_sample();
        }
        assert_eq!(engine.next_sample(), 0.0);
        assert_eq!(engine.position(), 0);
        engine.apply(Command::SetPattern(Pattern::default()));
        engine.apply(Command::Play);
        for _ in 0..7000 {
            assert_eq!(engine.next_sample(), 0.0);
        }
    }

    #[test]
    fn master_changes_ramp_and_dense_voices_stay_within_output_range() {
        let bank = test_bank([
            vec![1.0; 50_000],
            vec![1.0; 50_000],
            vec![1.0; 50_000],
            vec![1.0; 50_000],
        ]);
        let mut pattern = Pattern::default();
        pattern.set(0, 0, true);
        let mut engine = Engine::with_drums(bank.clone(), pattern, Bpm::default());
        engine.apply(Command::Play);
        for _ in 0..1000 {
            engine.next_sample();
        }
        let before = engine.next_sample();
        engine.apply(Command::SetGain(MAX_GAIN));
        let after = engine.next_sample();
        assert!(
            after > before && after - before < 0.002,
            "Master must ramp rather than jump"
        );
        for _ in 0..241 {
            engine.next_sample();
        }
        assert!((engine.next_sample() - MAX_GAIN * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        for track in 0..TRACKS {
            for step in 0..16 {
                pattern.set(track, step, true);
            }
        }
        let mut dense = Engine::with_drums(bank, pattern, Bpm::new(300).unwrap());
        dense.apply(Command::SetGain(MAX_GAIN));
        dense.apply(Command::Play);
        for _ in 0..80_000 {
            let sample = dense.next_sample();
            assert!(sample.is_finite() && sample.abs() <= 1.0);
        }
    }

    #[test]
    fn live_mute_preserves_clock_and_sample_progress_instead_of_retriggering() {
        let mut sample = vec![0.5; 1000];
        sample.extend(vec![0.25; 2000]);
        let bank = test_bank([sample, vec![], vec![], vec![]]);
        let mut pattern = Pattern::default();
        pattern.set(0, 0, true);
        let mut engine = Engine::with_drums(bank, pattern, Bpm::default());
        engine.apply(Command::Play);
        for _ in 0..500 {
            engine.next_frame();
        }
        let mut mixer = MixerState::default();
        mixer.tracks[0].muted = true;
        engine.apply(Command::SetMixer(mixer));
        for _ in 0..500 {
            engine.next_frame();
        }
        assert_eq!(engine.next_frame(), [0.0; 2]);
        assert_eq!(engine.position(), 1001);
        assert!(engine.is_playing());
        mixer.tracks[0].muted = false;
        engine.apply(Command::SetMixer(mixer));
        for _ in 0..240 {
            engine.next_frame();
        }
        let expected = 0.25 * DEFAULT_GAIN * std::f32::consts::FRAC_1_SQRT_2;
        for channel in engine.next_frame() {
            assert!((channel - expected).abs() < 1e-6);
        }
        assert_eq!(engine.position(), 1242);
    }

    #[test]
    fn timeline_routes_through_the_mixer_and_stop_resets_both_clocks_and_voices() {
        use crate::timeline::{Arrangement, TimelinePlayer};
        use std::sync::Arc;
        let rate = NonZeroU32::new(48_000).unwrap();
        let sources: Vec<_> = crate::samples::starter_kit()
            .into_iter()
            .map(Arc::new)
            .collect();
        let mut arrangement = Arrangement::default();
        let id = arrangement
            .add(0, 0, 240, sources[0].frame_count())
            .unwrap();
        let clip = crate::timeline::AudioClip {
            in_frame: 100,
            ..arrangement.get(id).unwrap()
        };
        arrangement.replace(clip).unwrap();
        let mut engine = Engine::with_timeline(
            rate,
            TimelinePlayer::prepare(&arrangement, &sources, rate).unwrap(),
            Bpm::default(),
        );
        let mut mixer = MixerState::default();
        mixer.tracks[0].pan = 1.0;
        engine.apply(Command::SetMixer(mixer));
        for _ in 0..241 {
            engine.next_frame();
        }
        for _ in 0..2 {
            engine.apply(Command::Play);
            for _ in 0..6000 {
                assert_eq!(engine.next_frame(), [0.0; 2]);
            }
            let output = engine.next_frame();
            assert_eq!(output[0], 0.0);
            assert!(output[1].abs() > 0.001);
            assert_eq!(engine.clip_triggers(), 1);
            assert_eq!(engine.musical_ticks(), 240);
            assert_eq!(engine.current_step(), None);
            engine.apply(Command::Stop);
            assert_eq!(engine.position(), 0);
            assert_eq!(engine.musical_ticks(), 0);
            for _ in 0..241 {
                engine.next_frame();
            }
            assert_eq!(engine.next_frame(), [0.0; 2]);
        }
    }
}
