//! Sixteenth-note timing using an integer phase accumulator, without rounded step lengths.
use std::num::NonZeroU32;

pub const TRACKS: usize = 4;
pub const STEPS: usize = 16;
pub const TRACK_NAMES: [&str; TRACKS] = ["Kick", "Snare", "Hi-hat", "Clap"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bpm(u16);

impl Bpm {
    pub const MIN: u16 = 30;
    pub const MAX: u16 = 300;

    pub fn new(value: u16) -> Result<Self, &'static str> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err("El BPM debe estar entre 30 y 300")
        }
    }

    pub fn get(self) -> u16 {
        self.0
    }

    /// Absolute quantized frame of a sixteenth note at a constant tempo.
    pub fn step_frame(self, step: u64, sample_rate: NonZeroU32) -> u64 {
        let numerator = u128::from(step) * u128::from(sample_rate.get()) * 60;
        numerator
            .div_ceil(u128::from(self.0) * 4)
            .min(u128::from(u64::MAX)) as u64
    }
}

impl Default for Bpm {
    fn default() -> Self {
        Self(120)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pattern([u16; TRACKS]);

impl Pattern {
    pub fn active(self, track: usize, step: usize) -> bool {
        step < STEPS
            && self
                .0
                .get(track)
                .is_some_and(|mask| mask & (1 << step) != 0)
    }

    pub fn set(&mut self, track: usize, step: usize, active: bool) {
        if step >= STEPS {
            return;
        }
        if let Some(mask) = self.0.get_mut(track) {
            if active {
                *mask |= 1 << step;
            } else {
                *mask &= !(1 << step);
            }
        }
    }

    pub fn demo() -> Self {
        Self([
            0b0001_0001_0001_0001,
            0b0001_0000_0001_0000,
            0b0101_0101_0101_0101,
            0b0001_0000_0000_0000,
        ])
    }
}

pub struct StepClock {
    threshold: u64,
    phase: u64,
    bpm: Bpm,
    step: u8,
    first: bool,
    events: u64,
}

impl StepClock {
    pub fn new(sample_rate: NonZeroU32, bpm: Bpm) -> Self {
        Self {
            threshold: u64::from(sample_rate.get()) * 60,
            phase: 0,
            bpm,
            step: 0,
            first: true,
            events: 0,
        }
    }

    /// Tempo changes preserve the fractional progress within the current step.
    pub fn set_bpm(&mut self, bpm: Bpm) {
        self.bpm = bpm;
    }

    pub fn reset(&mut self) {
        self.phase = 0;
        self.step = 0;
        self.first = true;
        self.events = 0;
    }

    /// Called immediately before generating each playing frame.
    pub fn tick(&mut self) -> Option<u8> {
        let event = if self.first {
            self.first = false;
            Some(0)
        } else if self.phase >= self.threshold {
            let elapsed = self.phase / self.threshold;
            self.phase %= self.threshold;
            self.step = ((u64::from(self.step) + elapsed) % STEPS as u64) as u8;
            Some(self.step)
        } else {
            None
        };
        if event.is_some() {
            self.events = self.events.saturating_add(1);
        }
        self.phase += u64::from(self.bpm.get()) * 4;
        event
    }

    pub fn step(&self) -> u8 {
        self.step
    }
    pub fn events(&self) -> u64 {
        self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_step_positions_and_loop_boundaries_do_not_drift() {
        for rate in [44_100, 48_000, 96_000] {
            for tempo in [30, 123, 137, 300] {
                let rate = NonZeroU32::new(rate).unwrap();
                let bpm = Bpm::new(tempo).unwrap();
                let mut clock = StepClock::new(rate, bpm);
                let mut event = 0;
                for frame in 0..=bpm.step_frame(128, rate) {
                    if let Some(step) = clock.tick() {
                        assert_eq!(frame, bpm.step_frame(event, rate));
                        assert_eq!(u64::from(step), event % 16);
                        event += 1;
                    }
                }
                assert_eq!(event, 129);
            }
        }
    }

    #[test]
    fn tempo_change_preserves_fractional_progress_and_reset_retriggers_step_zero() {
        let rate = NonZeroU32::new(48_000).unwrap();
        let mut clock = StepClock::new(rate, Bpm::default());
        assert_eq!(clock.tick(), Some(0));
        for _ in 1..3000 {
            assert_eq!(clock.tick(), None);
        }
        clock.set_bpm(Bpm::new(240).unwrap());
        for _ in 3000..4500 {
            assert_eq!(clock.tick(), None);
        }
        assert_eq!(clock.tick(), Some(1));
        clock.reset();
        assert_eq!(clock.events(), 0);
        assert_eq!(clock.tick(), Some(0));
    }

    #[test]
    fn invalid_tempo_and_step_indexes_are_rejected_or_ignored() {
        assert!(Bpm::new(0).is_err());
        assert!(Bpm::new(301).is_err());
        let mut pattern = Pattern::default();
        pattern.set(4, 0, true);
        pattern.set(0, 16, true);
        assert_eq!(pattern, Pattern::default());
        pattern.set(3, 15, true);
        assert!(pattern.active(3, 15));
        assert!(!pattern.active(3, 16));
        assert!(!pattern.active(4, 0));
        pattern.set(3, 15, false);
        assert_eq!(pattern, Pattern::default());
    }
}
