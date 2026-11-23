//! Non-destructive clip edits and a bounded, preloaded timeline player.
use std::{num::NonZeroU32, sync::Arc};

use crate::{
    samples::SampleSource,
    sequencer::{Bpm, TRACKS},
};

pub const TICKS_PER_BEAT: u64 = 960;
pub const SNAP_TICKS: u64 = TICKS_PER_BEAT / 4;
pub const MAX_START_TICK: u64 = 256 * TICKS_PER_BEAT;
pub const MAX_CLIPS: usize = 32;
pub const MAX_ASSETS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioClip {
    pub id: u32,
    pub track: usize,
    pub asset: usize,
    pub start_tick: u64,
    /// Source frame offsets, with an exclusive end. Never modify the source buffer.
    pub in_frame: u32,
    pub out_frame: u32,
}

impl AudioClip {
    pub fn duration_ticks(self, rate: u32, bpm: Bpm) -> f64 {
        f64::from(self.out_frame.saturating_sub(self.in_frame)) / f64::from(rate.max(1))
            * f64::from(bpm.get())
            / 60.0
            * TICKS_PER_BEAT as f64
    }

    pub fn moved(self, delta_ticks: f64, track: usize, snap: bool) -> Self {
        Self {
            start_tick: quantize(self.start_tick as f64 + delta_ticks, snap),
            track: track.min(TRACKS - 1),
            ..self
        }
    }

    pub fn trim_left(self, delta_ticks: f64, source_rate: u32, bpm: Bpm, snap: bool) -> Self {
        if !delta_ticks.is_finite() || source_rate == 0 {
            return self;
        }
        let desired = quantize(self.start_tick as f64 + delta_ticks, snap);
        let frames_per_tick =
            f64::from(source_rate) * 60.0 / f64::from(bpm.get()) / TICKS_PER_BEAT as f64;
        let requested =
            ((desired as f64 - self.start_tick as f64) * frames_per_tick).round() as i64;
        let delta = requested.clamp(
            -i64::from(self.in_frame),
            i64::from(self.out_frame) - i64::from(self.in_frame) - 1,
        );
        let actual_tick_delta = (delta as f64 / frames_per_tick).round();
        Self {
            start_tick: (self.start_tick as f64 + actual_tick_delta)
                .clamp(0.0, MAX_START_TICK as f64) as u64,
            in_frame: (i64::from(self.in_frame) + delta) as u32,
            ..self
        }
    }

    pub fn trim_right(
        self,
        delta_ticks: f64,
        source_rate: u32,
        source_frames: u32,
        bpm: Bpm,
        snap: bool,
    ) -> Self {
        if !delta_ticks.is_finite() || source_rate == 0 || source_frames < self.out_frame {
            return self;
        }
        let original_end = self.start_tick as f64 + self.duration_ticks(source_rate, bpm);
        // End positions may exceed MAX_START_TICK when the last clip has a long tail.
        let desired = snap_position((original_end + delta_ticks).max(0.0), snap);
        let frames_per_tick =
            f64::from(source_rate) * 60.0 / f64::from(bpm.get()) / TICKS_PER_BEAT as f64;
        let frames =
            (f64::from(self.out_frame) + (desired - original_end) * frames_per_tick).round();
        Self {
            out_frame: frames.clamp(f64::from(self.in_frame) + 1.0, f64::from(source_frames))
                as u32,
            ..self
        }
    }
}

fn snap_position(ticks: f64, snap: bool) -> f64 {
    if snap {
        (ticks / SNAP_TICKS as f64).round() * SNAP_TICKS as f64
    } else {
        ticks.round()
    }
}

pub fn quantize(ticks: f64, snap: bool) -> u64 {
    if !ticks.is_finite() {
        return 0;
    }
    snap_position(ticks, snap).clamp(0.0, MAX_START_TICK as f64) as u64
}

#[derive(Clone, Default)]
pub struct Arrangement {
    clips: Vec<AudioClip>,
    next_id: u32,
}

impl Arrangement {
    pub(crate) fn from_clips(clips: Vec<AudioClip>) -> Result<Self, String> {
        if clips.len() > MAX_CLIPS {
            return Err("El proyecto excede el máximo de clips".into());
        }
        for (index, clip) in clips.iter().enumerate() {
            if clip.id == 0
                || clips[..index].iter().any(|c| c.id == clip.id)
                || clip.track >= TRACKS
                || clip.asset >= MAX_ASSETS
                || clip.start_tick > MAX_START_TICK
                || clip.in_frame >= clip.out_frame
            {
                return Err("IDs, posición o recorte de clip no válidos".into());
            }
        }
        let next_id = clips.iter().map(|c| c.id).max().unwrap_or(0);
        Ok(Self { clips, next_id })
    }
    pub fn clips(&self) -> &[AudioClip] {
        &self.clips
    }
    pub fn get(&self, id: u32) -> Option<AudioClip> {
        self.clips.iter().find(|c| c.id == id).copied()
    }

    pub fn add(
        &mut self,
        track: usize,
        asset: usize,
        start_tick: u64,
        source_frames: u32,
    ) -> Result<u32, &'static str> {
        if self.clips.len() >= MAX_CLIPS {
            return Err("Máximo de 32 clips por sesión");
        }
        if track >= TRACKS
            || asset >= MAX_ASSETS
            || start_tick > MAX_START_TICK
            || source_frames == 0
        {
            return Err("Posición, pista o fuente del clip no válida");
        }
        let id = self
            .next_id
            .checked_add(1)
            .ok_or("Se agotaron los identificadores de clip")?;
        self.next_id = id;
        self.clips.push(AudioClip {
            id,
            track,
            asset,
            start_tick,
            in_frame: 0,
            out_frame: source_frames,
        });
        Ok(id)
    }

    pub fn replace(&mut self, clip: AudioClip) -> Result<(), &'static str> {
        if clip.track >= TRACKS
            || clip.asset >= MAX_ASSETS
            || clip.start_tick > MAX_START_TICK
            || clip.in_frame >= clip.out_frame
        {
            return Err("Recorte o posición del clip no válida");
        }
        let destination = self
            .clips
            .iter_mut()
            .find(|c| c.id == clip.id)
            .ok_or("El clip ya no existe")?;
        if clip.asset != destination.asset {
            return Err("Editar un clip no puede cambiar su fuente");
        }
        *destination = clip;
        Ok(())
    }

    pub fn remove(&mut self, id: u32) {
        self.clips.retain(|c| c.id != id);
    }

    /// Called only with the audio stream closed. Keep live asset indexes stable otherwise.
    pub fn prune_assets(&mut self, assets: &mut Vec<Arc<SampleSource>>) {
        for index in (0..assets.len()).rev() {
            if !self.clips.iter().any(|c| c.asset == index) {
                assets.remove(index);
                for clip in &mut self.clips {
                    if clip.asset > index {
                        clip.asset -= 1;
                    }
                }
            }
        }
    }
}

pub struct MusicalClock {
    ticks: u64,
    phase: u64,
    threshold: u64,
    bpm: Bpm,
}

impl MusicalClock {
    pub fn new(rate: NonZeroU32, bpm: Bpm) -> Self {
        Self {
            ticks: 0,
            phase: 0,
            threshold: u64::from(rate.get()) * 60,
            bpm,
        }
    }
    pub fn reset(&mut self) {
        self.ticks = 0;
        self.phase = 0;
    }
    pub fn set_bpm(&mut self, bpm: Bpm) {
        self.bpm = bpm;
    }
    pub fn ticks(&self) -> u64 {
        self.ticks
    }
    pub fn position(&self) -> f64 {
        self.ticks as f64 + self.phase as f64 / self.threshold as f64
    }
    pub fn advance(&mut self) {
        self.phase += u64::from(self.bpm.get()) * TICKS_PER_BEAT;
        self.ticks = self.ticks.saturating_add(self.phase / self.threshold);
        self.phase %= self.threshold;
    }
}

#[derive(Clone, Copy)]
struct PreparedClip {
    track: usize,
    asset: usize,
    start_tick: u64,
    start: usize,
    end: usize,
}

pub struct TimelinePlayer {
    sources: Vec<Arc<[f32]>>,
    clips: [Option<PreparedClip>; MAX_CLIPS],
    voices: [Option<usize>; MAX_CLIPS],
    previous_tick: Option<u64>,
    triggers: u64,
}

impl TimelinePlayer {
    pub fn prepare(
        arrangement: &Arrangement,
        sources: &[Arc<SampleSource>],
        rate: NonZeroU32,
    ) -> Result<Self, String> {
        if sources.len() > MAX_ASSETS || !(8_000..=192_000).contains(&rate.get()) {
            return Err("Configuración de timeline no válida".into());
        }
        let prepared = sources
            .iter()
            .map(|s| s.prepare(rate))
            .collect::<Result<Vec<_>, _>>()?;
        let mut clips = [None; MAX_CLIPS];
        for (slot, clip) in arrangement.clips().iter().enumerate() {
            let source = sources.get(clip.asset).ok_or("Falta la fuente del clip")?;
            if clip.track >= TRACKS
                || clip.in_frame >= clip.out_frame
                || clip.out_frame > source.frame_count()
            {
                return Err("El recorte excede la fuente de audio".into());
            }
            let start =
                u64::from(clip.in_frame) * u64::from(rate.get()) / u64::from(source.sample_rate());
            let end = (u64::from(clip.out_frame) * u64::from(rate.get()))
                .div_ceil(u64::from(source.sample_rate()));
            clips[slot] = Some(PreparedClip {
                track: clip.track,
                asset: clip.asset,
                start_tick: clip.start_tick,
                start: start as usize,
                end: (end as usize).min(prepared[clip.asset].len()),
            });
        }
        Ok(Self {
            sources: prepared,
            clips,
            voices: [None; MAX_CLIPS],
            previous_tick: None,
            triggers: 0,
        })
    }

    pub fn reset(&mut self) {
        self.voices = [None; MAX_CLIPS];
        self.previous_tick = None;
        self.triggers = 0;
    }
    pub fn triggers(&self) -> u64 {
        self.triggers
    }

    pub fn next_samples(&mut self, tick: u64, playing: bool) -> [f32; TRACKS] {
        let mut tracks = [0.0; TRACKS];
        for (slot, clip) in self.clips.iter().enumerate() {
            let Some(clip) = clip else {
                continue;
            };
            if playing
                && clip.start_tick <= tick
                && self
                    .previous_tick
                    .is_none_or(|previous| clip.start_tick > previous)
            {
                self.voices[slot] = Some(clip.start);
                self.triggers = self.triggers.saturating_add(1);
            }
            if let Some(position) = self.voices[slot] {
                if position < clip.end {
                    tracks[clip.track] += self.sources[clip.asset][position];
                    self.voices[slot] = (position + 1 < clip.end).then_some(position + 1);
                } else {
                    self.voices[slot] = None;
                }
            }
        }
        if playing {
            self.previous_tick = Some(tick);
        }
        tracks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_and_trims_preserve_source_and_respect_its_bounds() {
        let original = AudioClip {
            id: 1,
            asset: 2,
            track: 0,
            start_tick: 960,
            in_frame: 0,
            out_frame: 48_000,
        };
        let bpm = Bpm::default();
        let moved = original.moved(260.0, 3, true);
        assert_eq!(moved.start_tick, 1200);
        assert_eq!(
            (moved.asset, moved.in_frame, moved.out_frame),
            (2, 0, 48_000)
        );
        assert_eq!(original.moved(-5000.0, 0, true).start_tick, 0);
        let trimmed = original.trim_left(480.0, 48_000, bpm, true);
        assert_eq!(
            (trimmed.start_tick, trimmed.in_frame, trimmed.out_frame),
            (1440, 12_000, 48_000)
        );
        let restored = trimmed.trim_left(-480.0, 48_000, bpm, true);
        assert_eq!(restored, original);
        let shorter = original.trim_right(-480.0, 48_000, 48_000, bpm, true);
        assert_eq!(shorter.out_frame, 36_000);
        assert_eq!(
            shorter
                .trim_right(99_999.0, 48_000, 48_000, bpm, true)
                .out_frame,
            48_000
        );
        assert!(original.trim_left(99_999.0, 48_000, bpm, false).in_frame < original.out_frame);
        assert_eq!(
            original
                .trim_right(-99_999.0, 48_000, 48_000, bpm, false)
                .out_frame,
            1
        );
    }

    #[test]
    fn clock_matches_absolute_sample_positions_and_tempo_changes() {
        for tempo in [30, 123, 137, 300] {
            let rate = NonZeroU32::new(44_100).unwrap();
            let bpm = Bpm::new(tempo).unwrap();
            let mut clock = MusicalClock::new(rate, bpm);
            for frame in 0..100_000_u64 {
                assert_eq!(
                    clock.ticks(),
                    frame * u64::from(tempo) * TICKS_PER_BEAT / (u64::from(rate.get()) * 60)
                );
                clock.advance();
            }
            let previous = clock.ticks();
            clock.set_bpm(Bpm::new(240).unwrap());
            clock.advance();
            assert!(clock.ticks() >= previous);
            clock.reset();
            assert_eq!(clock.ticks(), 0);
        }
    }

    #[test]
    fn arrangement_limits_ids_and_source_invariants_are_checked() {
        let mut arrangement = Arrangement::default();
        assert!(arrangement.add(4, 0, 0, 100).is_err());
        assert!(arrangement.add(0, 16, 0, 100).is_err());
        assert!(arrangement.add(0, 0, 0, 0).is_err());
        for _ in 0..MAX_CLIPS {
            arrangement.add(0, 0, 0, 100).unwrap();
        }
        assert!(arrangement.add(0, 0, 0, 100).is_err());
        let mut clip = arrangement.clips()[0];
        clip.asset = 1;
        assert!(arrangement.replace(clip).is_err());
        arrangement.remove(clip.id);
        assert!(arrangement.get(clip.id).is_none());
        assert!(arrangement.add(0, 0, 0, 100).unwrap() > MAX_CLIPS as u32);
    }

    #[test]
    fn player_starts_on_exact_samples_and_keeps_non_destructive_offsets() {
        let rate = NonZeroU32::new(48_000).unwrap();
        let sources: Vec<_> = crate::samples::starter_kit()
            .into_iter()
            .map(Arc::new)
            .collect();
        let mut arrangement = Arrangement::default();
        let id = arrangement
            .add(2, 0, 240, sources[0].frame_count())
            .unwrap();
        let clip = AudioClip {
            in_frame: 100,
            out_frame: 110,
            ..arrangement.get(id).unwrap()
        };
        arrangement.replace(clip).unwrap();
        let prepared = sources[0].prepare(rate).unwrap();
        let mut player = TimelinePlayer::prepare(&arrangement, &sources, rate).unwrap();
        let mut clock = MusicalClock::new(rate, Bpm::default());
        for frame in 0..6011 {
            let output = player.next_samples(clock.ticks(), true);
            if (6000..6010).contains(&frame) {
                assert_eq!(output[2], prepared[100 + frame - 6000]);
            } else {
                assert_eq!(output, [0.0; TRACKS]);
            }
            clock.advance();
        }
        assert_eq!(player.triggers(), 1);
        player.reset();
        clock.reset();
        assert_eq!(player.triggers(), 0);
        assert_eq!(player.next_samples(clock.ticks(), false), [0.0; TRACKS]);
    }

    #[test]
    fn resampled_overlapping_clips_start_at_the_quantized_musical_frame() {
        let rate = NonZeroU32::new(44_100).unwrap();
        let bpm = Bpm::new(137).unwrap();
        let sources: Vec<_> = crate::samples::starter_kit()
            .into_iter()
            .map(Arc::new)
            .collect();
        let mut arrangement = Arrangement::default();
        for _ in 0..2 {
            let id = arrangement
                .add(1, 0, 480, sources[0].frame_count())
                .unwrap();
            arrangement
                .replace(AudioClip {
                    in_frame: 100,
                    out_frame: 110,
                    ..arrangement.get(id).unwrap()
                })
                .unwrap();
        }
        let prepared = sources[0].prepare(rate).unwrap();
        let mut player = TimelinePlayer::prepare(&arrangement, &sources, rate).unwrap();
        let mut clock = MusicalClock::new(rate, bpm);
        let start = bpm.step_frame(2, rate) as usize;
        for frame in 0..start + 15 {
            let output = player.next_samples(clock.ticks(), true);
            if (start..start + 11).contains(&frame) {
                assert_eq!(output[1], 2.0 * prepared[91 + frame - start]);
            } else {
                assert_eq!(output, [0.0; TRACKS]);
            }
            clock.advance();
        }
        assert_eq!(player.triggers(), 2);
    }

    #[test]
    fn removing_clips_reclaims_only_unused_sources_and_remaps_remaining_clips() {
        let mut sources: Vec<_> = crate::samples::starter_kit()
            .into_iter()
            .map(Arc::new)
            .collect();
        let retained = Arc::clone(&sources[2]);
        let mut arrangement = Arrangement::default();
        let a = arrangement.add(0, 0, 0, sources[0].frame_count()).unwrap();
        let b = arrangement.add(1, 2, 0, sources[2].frame_count()).unwrap();
        arrangement.remove(a);
        arrangement.prune_assets(&mut sources);
        assert_eq!(sources.len(), 1);
        assert!(Arc::ptr_eq(&sources[0], &retained));
        assert_eq!(arrangement.get(b).unwrap().asset, 0);
        assert_eq!(retained.name, "Hi-hat interno");
    }
}
