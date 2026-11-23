//! Four mono channels into a stereo master bus, independent of UI and audio devices.
use std::{
    f32::consts::{FRAC_1_SQRT_2, FRAC_PI_4},
    num::NonZeroU32,
};

use crate::sequencer::TRACKS;

pub type StereoFrame = [f32; 2];
pub const MAX_TRACK_GAIN: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackMix {
    pub gain: f32,
    /// -1 = left, 0 = center, 1 = right.
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
}

impl Default for TrackMix {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            muted: false,
            solo: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MixerState {
    pub tracks: [TrackMix; TRACKS],
}

impl MixerState {
    pub fn audible(self, track: usize) -> bool {
        self.tracks.get(track).is_some_and(|channel| {
            !channel.muted && (!self.tracks.iter().any(|t| t.solo) || channel.solo)
        })
    }

    pub(crate) fn is_valid(self) -> bool {
        self.tracks.iter().all(|t| {
            t.gain.is_finite()
                && (0.0..=MAX_TRACK_GAIN).contains(&t.gain)
                && t.pan.is_finite()
                && (-1.0..=1.0).contains(&t.pan)
        })
    }

    fn coefficients(self, track: usize) -> StereoFrame {
        if !self.audible(track) {
            return [0.0; 2];
        }
        let channel = self.tracks[track];
        if channel.pan == -1.0 {
            return [channel.gain, 0.0];
        }
        if channel.pan == 1.0 {
            return [0.0, channel.gain];
        }
        let angle = (channel.pan + 1.0) * FRAC_PI_4;
        [channel.gain * angle.cos(), channel.gain * angle.sin()]
    }
}

pub struct Mixer {
    state: MixerState,
    current: [StereoFrame; TRACKS],
    target: [StereoFrame; TRACKS],
    increment: [StereoFrame; TRACKS],
    remaining: [u32; TRACKS],
    ramp_frames: u32,
    levels: [f32; TRACKS],
    automated: [bool; TRACKS],
}

impl Mixer {
    pub fn new(rate: NonZeroU32) -> Self {
        Self {
            state: MixerState::default(),
            current: [[FRAC_1_SQRT_2; 2]; TRACKS],
            target: [[FRAC_1_SQRT_2; 2]; TRACKS],
            increment: [[0.0; 2]; TRACKS],
            remaining: [0; TRACKS],
            ramp_frames: rate.get().div_ceil(200),
            levels: [0.0; TRACKS],
            automated: [false; TRACKS],
        }
    }

    /// Reject invalid snapshots as a whole; preserve ramps already in progress.
    pub fn set_state(&mut self, state: MixerState) -> bool {
        if !state.is_valid() {
            return false;
        }
        if self.state == state {
            return true;
        }
        self.state = state;
        for track in 0..TRACKS {
            let target = state.coefficients(track);
            self.retarget(track, target);
        }
        true
    }

    fn retarget(&mut self, track: usize, target: StereoFrame) {
        if target != self.target[track] {
            self.target[track] = target;
            self.increment[track] = std::array::from_fn(|side| {
                (target[side] - self.current[track][side]) / self.ramp_frames as f32
            });
            self.remaining[track] = self.ramp_frames;
        }
    }
    pub fn reset_automation(&mut self) {
        for track in 0..TRACKS {
            if self.automated[track] {
                self.retarget(track, self.state.coefficients(track));
                self.automated[track] = false;
            }
        }
    }
    pub fn start_automation(&mut self, gains: [Option<f32>; TRACKS], pans: [Option<f32>; TRACKS]) {
        self.reset_automation();
        for track in 0..TRACKS {
            if gains[track].is_some() || pans[track].is_some() {
                let mut effective = self.state;
                if let Some(g) = gains[track] {
                    effective.tracks[track].gain = g;
                }
                if let Some(p) = pans[track] {
                    effective.tracks[track].pan = p;
                }
                if effective.is_valid() {
                    let coeff = effective.coefficients(track);
                    self.current[track] = coeff;
                    self.target[track] = coeff;
                    self.remaining[track] = 0;
                    self.increment[track] = [0.0; 2];
                    self.automated[track] = true;
                }
            }
        }
    }
    pub fn process_automated(
        &mut self,
        tracks: [f32; TRACKS],
        gains: [Option<f32>; TRACKS],
        pans: [Option<f32>; TRACKS],
    ) -> StereoFrame {
        for track in 0..TRACKS {
            let active = gains[track].is_some() || pans[track].is_some();
            if active || self.automated[track] {
                let mut effective = self.state;
                if let Some(gain) = gains[track] {
                    effective.tracks[track].gain = gain;
                }
                if let Some(pan) = pans[track] {
                    effective.tracks[track].pan = pan;
                }
                if effective.is_valid() {
                    self.retarget(track, effective.coefficients(track));
                }
            }
            self.automated[track] = active;
        }
        self.process(tracks)
    }

    pub fn process(&mut self, tracks: [f32; TRACKS]) -> StereoFrame {
        let mut master = [0.0; 2];
        for (track, sample) in tracks.into_iter().enumerate() {
            if self.remaining[track] > 0 {
                self.remaining[track] -= 1;
                for side in 0..2 {
                    self.current[track][side] = if self.remaining[track] == 0 {
                        self.target[track][side]
                    } else {
                        self.current[track][side] + self.increment[track][side]
                    };
                }
            }
            let left = sample * self.current[track][0];
            let right = sample * self.current[track][1];
            self.levels[track] = left.abs().max(right.abs());
            master[0] += left;
            master[1] += right;
        }
        master
    }

    /// Latest frame's post-fader, post-pan peaks, before master gain.
    pub fn levels(&self) -> [f32; TRACKS] {
        self.levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixer() -> Mixer {
        Mixer::new(NonZeroU32::new(48_000).unwrap())
    }
    fn settle(mixer: &mut Mixer, input: [f32; TRACKS]) -> StereoFrame {
        let mut output = [0.0; 2];
        for _ in 0..241 {
            output = mixer.process(input);
        }
        output
    }

    #[test]
    fn equal_power_pan_has_exact_endpoints_and_constant_settled_power() {
        let mut mixer = mixer();
        let mut state = MixerState::default();
        for pan in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            state.tracks[0].pan = pan;
            assert!(mixer.set_state(state));
            let frame = settle(&mut mixer, [1.0, 0.0, 0.0, 0.0]);
            assert!((frame[0].powi(2) + frame[1].powi(2) - 1.0).abs() < 1e-6);
            if pan == -1.0 {
                assert_eq!(frame, [1.0, 0.0]);
            }
            if pan == 1.0 {
                assert_eq!(frame, [0.0, 1.0]);
            }
        }
    }

    #[test]
    fn gains_and_meters_are_independent_before_master_summation() {
        let mut mixer = mixer();
        let mut state = MixerState::default();
        state.tracks[0].pan = -1.0;
        state.tracks[0].gain = 0.25;
        state.tracks[1].pan = 1.0;
        state.tracks[1].gain = 1.5;
        mixer.set_state(state);
        assert_eq!(settle(&mut mixer, [1.0, 0.5, 0.0, 0.0]), [0.25, 0.75]);
        assert_eq!(mixer.levels(), [0.25, 0.75, 0.0, 0.0]);
    }

    #[test]
    fn all_mute_and_multi_solo_combinations_are_coherent() {
        // Exhaustively exercise four mute bits and four solo bits together.
        let mut mixer = mixer();
        for mutes in 0..16 {
            for solos in 0..16 {
                let mut state = MixerState::default();
                for track in 0..TRACKS {
                    state.tracks[track].pan = -1.0;
                    state.tracks[track].muted = mutes & (1 << track) != 0;
                    state.tracks[track].solo = solos & (1 << track) != 0;
                }
                mixer.set_state(state);
                let expected = (0..TRACKS)
                    .filter(|&t| mutes & (1 << t) == 0 && (solos == 0 || solos & (1 << t) != 0))
                    .count() as f32;
                assert_eq!(settle(&mut mixer, [1.0; TRACKS]), [expected, 0.0]);
            }
        }
    }

    #[test]
    fn gain_pan_and_mute_changes_ramp_even_when_retargeted() {
        let mut mixer = mixer();
        let mut state = MixerState::default();
        let mut previous = mixer.process([1.0, 0.0, 0.0, 0.0]);
        state.tracks[0].pan = 1.0;
        mixer.set_state(state);
        for frame in 0..500 {
            if frame == 80 {
                state.tracks[0].muted = true;
                mixer.set_state(state);
            }
            let output = mixer.process([1.0, 0.0, 0.0, 0.0]);
            for side in 0..2 {
                assert!((output[side] - previous[side]).abs() < 0.01);
            }
            previous = output;
        }
        assert_eq!(previous, [0.0; 2]);
    }

    #[test]
    fn invalid_snapshots_cannot_poison_the_mix() {
        let mut mixer = mixer();
        let initial = mixer.process([1.0, 0.0, 0.0, 0.0]);
        for invalid in [f32::NAN, f32::INFINITY, -1.0, 3.0] {
            let mut state = MixerState::default();
            state.tracks[0].gain = invalid;
            assert!(!mixer.set_state(state));
            assert_eq!(mixer.process([1.0, 0.0, 0.0, 0.0]), initial);
        }
        for invalid in [f32::NAN, f32::NEG_INFINITY, -2.0, 2.0] {
            let mut state = MixerState::default();
            state.tracks[0].pan = invalid;
            assert!(!mixer.set_state(state));
            assert_eq!(mixer.process([1.0, 0.0, 0.0, 0.0]), initial);
        }
        assert!(!MixerState::default().audible(TRACKS));
    }
}
