//! Stereo master inserts, with fixed delay storage and Copy-only parameter updates.
use crate::mixer::StereoFrame;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectsState {
    pub distortion_enabled: bool,
    pub drive: f32,
    pub distortion_mix: f32,
    pub delay_enabled: bool,
    pub delay_ms: f32,
    pub feedback: f32,
    pub delay_mix: f32,
}
impl Default for EffectsState {
    fn default() -> Self {
        Self {
            distortion_enabled: false,
            drive: 3.0,
            distortion_mix: 0.5,
            delay_enabled: false,
            delay_ms: 250.0,
            feedback: 0.35,
            delay_mix: 0.3,
        }
    }
}
impl EffectsState {
    pub fn validate(self) -> Result<(), String> {
        if !self.drive.is_finite()
            || !(1.0..=20.0).contains(&self.drive)
            || !self.delay_ms.is_finite()
            || !(10.0..=1000.0).contains(&self.delay_ms)
            || !self.feedback.is_finite()
            || !(0.0..=0.85).contains(&self.feedback)
            || [self.distortion_mix, self.delay_mix]
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("Parámetros de efectos no válidos".into());
        }
        Ok(())
    }
    pub fn prepare(self, rate: NonZeroU32) -> Result<PreparedEffectsState, String> {
        self.validate()?;
        if !(8_000..=192_000).contains(&rate.get()) {
            return Err("Sample rate de efectos no válido".into());
        }
        Ok(PreparedEffectsState {
            rate,
            values: [
                self.drive,
                1.0 / self.drive.tanh(),
                if self.distortion_enabled {
                    self.distortion_mix
                } else {
                    0.0
                },
                (self.delay_ms as f64 * f64::from(rate.get()) / 1000.0) as f32,
                self.feedback,
                if self.delay_enabled {
                    self.delay_mix
                } else {
                    0.0
                },
                if self.delay_enabled { 1.0 } else { 0.0 },
            ],
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedEffectsState {
    rate: NonZeroU32,
    values: [f32; 7],
}

#[derive(Clone, Copy)]
struct Ramp {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
}
impl Ramp {
    fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
        }
    }
    fn set(&mut self, target: f32, frames: u32) {
        if self.target != target {
            self.target = target;
            self.step = (target - self.current) / frames as f32;
            self.remaining = frames;
        }
    }
    fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
            } else {
                self.current += self.step;
            }
        }
        self.current
    }
    fn settle(&mut self) {
        self.current = self.target;
        self.remaining = 0;
        self.step = 0.0;
    }
}

pub struct Effects {
    rate: NonZeroU32,
    buffer: Vec<StereoFrame>,
    write: usize,
    valid: usize,
    ramps: [Ramp; 7],
    ramp_frames: u32,
}
impl Effects {
    /// Allocate once, before opening a stream. No buffer is transferred by a command.
    pub fn new(rate: NonZeroU32, state: EffectsState) -> Result<Self, String> {
        let prepared = state.prepare(rate)?;
        Ok(Self {
            rate,
            buffer: vec![[0.0; 2]; rate.get() as usize + 2],
            write: 0,
            valid: 0,
            ramps: prepared.values.map(Ramp::new),
            ramp_frames: rate.get().div_ceil(50),
        })
    }
    pub fn set_state(&mut self, state: PreparedEffectsState) -> bool {
        if self.rate != state.rate {
            return false;
        }
        for (ramp, value) in self.ramps.iter_mut().zip(state.values) {
            ramp.set(value, self.ramp_frames);
        }
        true
    }
    /// Logical clear: unread history is invalidated, without clearing a large buffer in the callback.
    pub fn reset(&mut self) {
        self.write = 0;
        self.valid = 0;
        for ramp in &mut self.ramps {
            ramp.settle();
        }
    }
    fn read_ago(&self, ago: usize) -> StereoFrame {
        if ago == 0 || ago > self.valid {
            return [0.0; 2];
        }
        self.buffer[(self.write + self.buffer.len() - ago) % self.buffer.len()]
    }
    pub fn process(&mut self, input: StereoFrame) -> StereoFrame {
        self.process_automated(input, None)
    }
    pub fn process_automated(
        &mut self,
        input: StereoFrame,
        automated_mix: Option<f32>,
    ) -> StereoFrame {
        let [
            drive,
            normalization,
            dist_mix,
            delay_frames,
            feedback,
            delay_mix,
            send,
        ] = self.ramps.each_mut().map(Ramp::next);
        let delay_mix = automated_mix
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .map_or(delay_mix, |v| v * send.clamp(0.0, 1.0));
        let dry = input.map(|s| if s.is_finite() { s } else { 0.0 });
        let distorted = if dist_mix == 0.0 {
            dry
        } else {
            dry.map(|s| {
                let wet = (s * drive).tanh() * normalization;
                s + (wet - s) * dist_mix
            })
        };
        let delay_frames = delay_frames.clamp(1.0, (self.buffer.len() - 2) as f32);
        let whole = delay_frames.floor() as usize;
        let fraction = delay_frames - whole as f32;
        let a = self.read_ago(whole);
        let b = self.read_ago(whole + 1);
        let echo = std::array::from_fn::<_, 2, _>(|side| a[side] + (b[side] - a[side]) * fraction);
        for side in 0..2 {
            let value = (distorted[side] * send + echo[side] * feedback.clamp(0.0, 0.85))
                .clamp(-32.0, 32.0);
            self.buffer[self.write][side] = if value.abs() < 1e-15 { 0.0 } else { value };
        }
        self.write = (self.write + 1) % self.buffer.len();
        self.valid = (self.valid + 1).min(self.buffer.len());
        if delay_mix == 0.0 {
            distorted
        } else {
            std::array::from_fn(|side| distorted[side] + (echo[side] - distorted[side]) * delay_mix)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rate() -> NonZeroU32 {
        NonZeroU32::new(48_000).unwrap()
    }
    #[test]
    fn default_bypass_is_bit_exact_and_disabled_delay_does_not_capture_input() {
        let mut fx = Effects::new(rate(), EffectsState::default()).unwrap();
        for i in 0..50_000 {
            let x = [(i as f32 * 0.013).sin(), -0.0];
            assert_eq!(fx.process(x).map(f32::to_bits), x.map(f32::to_bits));
        }
        let enabled = EffectsState {
            delay_enabled: true,
            delay_mix: 1.0,
            ..Default::default()
        };
        fx.set_state(enabled.prepare(rate()).unwrap());
        for _ in 0..50_000 {
            assert_eq!(fx.process([0.0; 2]), [0.0; 2]);
        }
    }
    #[test]
    fn delay_impulses_have_exact_time_feedback_and_stereo_separation() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000] {
            let rate = NonZeroU32::new(sample_rate).unwrap();
            let state = EffectsState {
                delay_enabled: true,
                delay_ms: 100.0,
                feedback: 0.5,
                delay_mix: 1.0,
                ..Default::default()
            };
            let mut fx = Effects::new(rate, state).unwrap();
            let d = sample_rate / 10;
            for frame in 0..=d * 3 {
                let output = fx.process(if frame == 0 { [1.0, 0.0] } else { [0.0; 2] });
                let expected = if frame == d {
                    1.0
                } else if frame == d * 2 {
                    0.5
                } else if frame == d * 3 {
                    0.25
                } else {
                    0.0
                };
                assert_eq!(output, [expected, 0.0], "{sample_rate} Hz, frame {frame}");
            }
        }
    }
    #[test]
    fn fractional_delay_interpolates_and_logical_reset_cannot_expose_old_audio() {
        let state = EffectsState {
            delay_enabled: true,
            delay_ms: 10.0625,
            feedback: 0.0,
            delay_mix: 1.0,
            ..Default::default()
        };
        let mut fx = Effects::new(NonZeroU32::new(8_000).unwrap(), state).unwrap();
        for frame in 0..=81 {
            let output = fx.process(if frame == 0 { [1.0, -1.0] } else { [0.0; 2] });
            let expected = if frame == 80 || frame == 81 { 0.5 } else { 0.0 };
            assert_eq!(output, [expected, -expected]);
        }
        fx.reset();
        for _ in 0..17_000 {
            assert_eq!(fx.process([0.0; 2]), [0.0; 2]);
        }
    }
    #[test]
    fn distortion_is_odd_bounded_and_drive_changes_the_transfer_curve() {
        let mut low = Effects::new(
            rate(),
            EffectsState {
                distortion_enabled: true,
                drive: 1.0,
                distortion_mix: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        let mut high = Effects::new(
            rate(),
            EffectsState {
                distortion_enabled: true,
                drive: 20.0,
                distortion_mix: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        let a = low.process([0.1, -0.1]);
        let b = high.process([0.1, -0.1]);
        assert_eq!(a[0], -a[1]);
        assert_eq!(b[0], -b[1]);
        assert!(b[0] > a[0] * 3.0);
        for i in 0..1000 {
            let y = high.process([i as f32, -(i as f32)]);
            assert!(y.iter().all(|v| v.is_finite() && v.abs() <= 1.00001));
        }
        assert_eq!(high.process([f32::NAN, f32::INFINITY]), [0.0; 2]);
    }
    #[test]
    fn live_updates_are_smoothed_bounded_and_do_not_resize_or_reset_history() {
        let mut fx = Effects::new(rate(), EffectsState::default()).unwrap();
        let capacity = fx.buffer.capacity();
        for i in 0..100_000 {
            if i % 400 == 0 {
                let state = EffectsState {
                    distortion_enabled: i % 800 == 0,
                    drive: 20.0,
                    distortion_mix: 1.0,
                    delay_enabled: true,
                    delay_ms: if i % 800 == 0 { 10.0 } else { 1000.0 },
                    feedback: 0.85,
                    delay_mix: 0.9,
                };
                assert!(fx.set_state(state.prepare(rate()).unwrap()));
            }
            let output = fx.process([0.25, -0.25]);
            assert!(output.iter().all(|v| v.is_finite() && v.abs() < 32.0));
        }
        assert_eq!(fx.buffer.capacity(), capacity);
        assert!(fx.valid > 0);
        let mut ramp = Ramp::new(0.0);
        ramp.set(1.0, 960);
        assert!(ramp.next() < 0.002);
        for _ in 1..960 {
            ramp.next();
        }
        assert_eq!(ramp.current, 1.0);
    }
    #[test]
    fn invalid_parameters_and_mismatched_rates_are_rejected() {
        for state in [
            EffectsState {
                drive: f32::NAN,
                ..Default::default()
            },
            EffectsState {
                delay_ms: 1001.0,
                ..Default::default()
            },
            EffectsState {
                feedback: 1.0,
                ..Default::default()
            },
            EffectsState {
                delay_mix: -0.1,
                ..Default::default()
            },
            EffectsState {
                distortion_mix: f32::INFINITY,
                ..Default::default()
            },
        ] {
            assert!(state.prepare(rate()).is_err());
        }
        let mut fx = Effects::new(rate(), EffectsState::default()).unwrap();
        assert!(
            !fx.set_state(
                EffectsState::default()
                    .prepare(NonZeroU32::new(44_100).unwrap())
                    .unwrap()
            )
        );
        assert!(Effects::new(NonZeroU32::new(1).unwrap(), EffectsState::default()).is_err());
    }

    #[test]
    fn automated_delay_mix_overrides_manual_mix_but_respects_bypass() {
        let state = EffectsState {
            delay_enabled: true,
            delay_ms: 10.0,
            feedback: 0.0,
            delay_mix: 0.0,
            ..Default::default()
        };
        let mut fx = Effects::new(rate(), state).unwrap();
        assert_eq!(fx.process_automated([1.0, 0.0], Some(1.0)), [0.0; 2]);
        for _ in 1..480 {
            assert_eq!(fx.process_automated([0.0; 2], Some(1.0)), [0.0; 2]);
        }
        assert_eq!(fx.process_automated([0.0; 2], Some(1.0)), [1.0, 0.0]);
        let mut bypass = Effects::new(rate(), EffectsState::default()).unwrap();
        assert_eq!(
            bypass.process_automated([0.7, -0.3], Some(1.0)),
            [0.7, -0.3]
        );
    }
}
