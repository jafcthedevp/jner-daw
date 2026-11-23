//! Offline stereo WAV rendering. Uses the playback engine without opening CPAL.
use std::{
    io::{BufWriter, Write},
    num::NonZeroU32,
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use crate::{
    engine::{Command, Engine},
    midi::MidiPlayer,
    project::Project,
    samples::SampleBank,
    timeline::TimelinePlayer,
};

pub const MAX_BEATS: u32 = 512;
const BLOCK_FRAMES: u64 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    Pattern,
    Timeline,
    PianoRoll,
}
impl RenderMode {
    pub fn from_project(project: &Project) -> Self {
        if project.workspace.piano_playback {
            Self::PianoRoll
        } else if project.workspace.timeline_playback {
            Self::Timeline
        } else {
            Self::Pattern
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Pattern => "Patrón",
            Self::Timeline => "Timeline",
            Self::PianoRoll => "Piano roll",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ExportOptions {
    pub mode: RenderMode,
    pub sample_rate: u32,
    /// Interval [beat 1, beat 1 + beats). Includes any tails within that interval.
    pub beats: u32,
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            mode: RenderMode::Pattern,
            sample_rate: 48_000,
            beats: 4,
        }
    }
}
impl ExportOptions {
    pub fn frames(self, project: &Project) -> Result<u64, String> {
        if !(8_000..=192_000).contains(&self.sample_rate) || !(1..=MAX_BEATS).contains(&self.beats)
        {
            return Err("Exportación: usa 8–192 kHz y entre 1 y 512 beats".into());
        }
        Ok((u64::from(self.beats) * 60 * u64::from(self.sample_rate))
            .div_ceil(u64::from(project.bpm.get())))
    }
}

/// Whole beats covering the sources in one mode. Delay tails remain part of
/// the user-selected interval; pattern playback loops throughout it.
pub fn suggested_beats(project: &Project, mode: RenderMode) -> u32 {
    let end = match mode {
        RenderMode::Pattern => 4.0,
        RenderMode::Timeline => project
            .arrangement
            .clips()
            .iter()
            .filter_map(|c| {
                project.timeline_sources.get(c.asset).map(|s| {
                    c.start_tick as f64 / 960.0
                        + c.duration_ticks(s.sample_rate(), project.bpm) / 960.0
                })
            })
            .fold(0.0_f64, f64::max),
        RenderMode::PianoRoll => project
            .notes
            .notes()
            .iter()
            .map(|n| {
                (n.start_tick + n.duration_tick) as f64 / 960.0
                    + project
                        .synth
                        .tracks
                        .get(n.track)
                        .map_or(0.0, |p| f64::from(p.release_ms))
                        * f64::from(project.bpm.get())
                        / 60_000.0
            })
            .fold(0.0_f64, f64::max),
    };
    end.ceil().clamp(1.0, f64::from(MAX_BEATS)) as u32
}

#[derive(Default)]
pub struct ExportControl {
    cancelled: AtomicBool,
    completed: AtomicU64,
    total: AtomicU64,
}
impl ExportControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn progress(&self) -> (u64, u64) {
        (
            self.completed.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
    fn check_cancelled(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err("Exportación cancelada; el destino anterior se conserva".into())
        } else {
            Ok(())
        }
    }
}

fn prepare(project: &Project, options: ExportOptions) -> Result<Engine, String> {
    let rate = NonZeroU32::new(options.sample_rate).ok_or("Sample rate cero")?;
    let mut engine = match options.mode {
        RenderMode::Pattern => Engine::with_drums(
            SampleBank::prepare(&project.sources, rate)?,
            project.pattern,
            project.bpm,
        ),
        RenderMode::Timeline => Engine::with_timeline(
            rate,
            TimelinePlayer::prepare(&project.arrangement, &project.timeline_sources, rate)?,
            project.bpm,
        ),
        RenderMode::PianoRoll => Engine::with_synth(
            rate,
            MidiPlayer::prepare(&project.notes),
            project.bpm,
            project.synth,
        )?,
    };
    engine.configure_effects(project.effects)?;
    engine.configure_automation(&project.automation)?;
    engine.apply(Command::SetMixer(project.mixer));
    engine.apply(Command::SetGain(project.master_gain));
    // Settle the mixer's 5 ms parameter ramp in silence, as a stopped output
    // stream does before Play. Muted tracks must not leak into the first frames.
    // The stopped engine does not advance its musical/sample transport clocks.
    for _ in 0..rate.get().div_ceil(200) {
        engine.next_frame();
    }
    engine.apply(Command::Play);
    Ok(engine)
}

fn pcm16(value: f32) -> Result<i16, String> {
    if !value.is_finite() {
        return Err("El render produjo una muestra no finita".into());
    }
    Ok((value * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
}

/// Writes to a sibling temporary file, then replaces the destination only on success.
/// Cancellation is checked between 1024-frame blocks and again before committing.
pub fn export_wav(
    path: &Path,
    project: &Project,
    options: ExportOptions,
    control: &ExportControl,
) -> Result<u64, String> {
    project.validate()?;
    let frames = options.frames(project)?;
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
    {
        return Err("El destino de exportación debe tener extensión .wav".into());
    }
    control.check_cancelled()?;
    control.total.store(frames, Ordering::Relaxed);
    control.completed.store(0, Ordering::Relaxed);
    let mut engine = prepare(project, options)?;
    control.check_cancelled()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("No se pudo preparar el WAV: {e}"))?;
    {
        let mut buffered = BufWriter::new(temporary.as_file_mut());
        {
            let spec = hound::WavSpec {
                channels: 2,
                sample_rate: options.sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut wav = hound::WavWriter::new(&mut buffered, spec).map_err(|e| e.to_string())?;
            let mut done = 0;
            while done < frames {
                control.check_cancelled()?;
                let count = BLOCK_FRAMES.min(frames - done);
                for _ in 0..count {
                    for sample in engine.next_frame() {
                        wav.write_sample(pcm16(sample)?)
                            .map_err(|e| format!("No se pudo escribir el WAV: {e}"))?;
                    }
                }
                done += count;
                control.completed.store(done, Ordering::Relaxed);
            }
            wav.finalize()
                .map_err(|e| format!("No se pudo finalizar el WAV: {e}"))?;
        }
        buffered.flush().map_err(|e| e.to_string())?;
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    control.check_cancelled()?;
    temporary
        .persist(path)
        .map_err(|e| format!("No se pudo reemplazar el WAV: {}", e.error))?;
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        automation::Target,
        mixer::MixerState,
        samples::starter_kit,
        sequencer::{Bpm, Pattern},
    };
    use std::sync::Arc;

    #[test]
    fn configurable_rate_duration_rounding_and_pcm_endpoints_are_bounded() {
        let mut project = Project::empty();
        project.bpm = Bpm::new(137).unwrap();
        for rate in [8_000, 44_100, 48_000, 96_000, 192_000] {
            let o = ExportOptions {
                sample_rate: rate,
                beats: 7,
                ..Default::default()
            };
            assert_eq!(
                o.frames(&project).unwrap(),
                (7_u64 * 60 * u64::from(rate)).div_ceil(137)
            );
        }
        for (rate, beats) in [(0, 4), (192_001, 4), (48_000, 0), (48_000, 513)] {
            assert!(
                ExportOptions {
                    sample_rate: rate,
                    beats,
                    ..Default::default()
                }
                .frames(&project)
                .is_err()
            );
        }
        assert_eq!(pcm16(-1.0).unwrap(), i16::MIN);
        assert_eq!(pcm16(1.0).unwrap(), i16::MAX);
        assert_eq!(pcm16(0.0).unwrap(), 0);
        assert!(pcm16(f32::NAN).is_err());
    }

    #[test]
    fn wav_matches_playback_engine_with_mixer_effects_and_automation_at_two_rates() {
        let mut p = Project::empty();
        p.pattern = Pattern::demo();
        p.bpm = Bpm::new(137).unwrap();
        p.master_gain = 0.2;
        p.mixer.tracks[0].pan = -1.0;
        p.mixer.tracks[1].pan = 1.0;
        p.effects.distortion_enabled = true;
        p.effects.delay_enabled = true;
        p.effects.delay_ms = 50.0;
        p.automation.add(Target::TrackGain(0), 0, 0.1).unwrap();
        p.automation.add(Target::TrackGain(0), 960, 1.5).unwrap();
        let dir = tempfile::tempdir().unwrap();
        for rate in [44_100, 48_000] {
            let o = ExportOptions {
                sample_rate: rate,
                beats: 2,
                ..Default::default()
            };
            let path = dir.path().join(format!("{rate}.wav"));
            let control = ExportControl::default();
            let count = export_wav(&path, &p, o, &control).unwrap();
            assert_eq!(control.progress(), (count, count));
            let mut wav = hound::WavReader::open(path).unwrap();
            assert_eq!(
                (
                    wav.spec().channels,
                    wav.spec().sample_rate,
                    wav.spec().bits_per_sample
                ),
                (2, rate, 16)
            );
            assert_eq!(u64::from(wav.duration()), count);
            let nz = NonZeroU32::new(rate).unwrap();
            let mut live = Engine::with_drums(
                SampleBank::prepare(&p.sources, nz).unwrap(),
                p.pattern,
                p.bpm,
            );
            live.configure_effects(p.effects).unwrap();
            live.configure_automation(&p.automation).unwrap();
            live.apply(Command::SetMixer(p.mixer));
            live.apply(Command::SetGain(p.master_gain));
            for _ in 0..rate.div_ceil(200) {
                live.next_frame();
            }
            live.apply(Command::Play);
            let samples = wav.samples::<i16>().map(Result::unwrap).collect::<Vec<_>>();
            let mut audible = false;
            for frame in samples.chunks_exact(2) {
                let expected = live.next_frame();
                for side in 0..2 {
                    assert_eq!(frame[side], pcm16(expected[side]).unwrap());
                    audible |= frame[side] != 0;
                }
            }
            assert!(audible);
        }
    }

    #[test]
    fn timeline_recorded_source_and_synth_modes_do_not_mix_in_the_pattern() {
        let mut p = Project::empty();
        p.pattern = Pattern::demo();
        p.timeline_sources
            .push(Arc::new(crate::recording::synthetic_take().unwrap()));
        p.arrangement.add(0, 0, 0, 11_025).unwrap();
        p.notes.add(0, 60, 100, 0, 480).unwrap();
        p.mixer = MixerState::default();
        let dir = tempfile::tempdir().unwrap();
        for mode in [RenderMode::Timeline, RenderMode::PianoRoll] {
            let o = ExportOptions {
                mode,
                beats: 2,
                ..Default::default()
            };
            let path = dir.path().join("mode.wav");
            export_wav(&path, &p, o, &ExportControl::default()).unwrap();
            let samples = hound::WavReader::open(&path)
                .unwrap()
                .samples::<i16>()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            assert!(samples.iter().any(|v| *v != 0));
            assert!(samples[samples.len() - 2000..].iter().all(|v| *v == 0));
        }
        assert_eq!(p.sources[0].frame_count(), starter_kit()[0].frame_count());
    }

    #[test]
    fn invalid_cancelled_or_failed_exports_preserve_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.wav");
        std::fs::write(&path, b"previous").unwrap();
        let mut p = Project::empty();
        let control = ExportControl::default();
        control.cancel();
        assert!(export_wav(&path, &p, ExportOptions::default(), &control).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        p.master_gain = f32::NAN;
        assert!(
            export_wav(
                &path,
                &p,
                ExportOptions::default(),
                &ExportControl::default()
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        p = Project::empty();
        assert!(
            export_wav(
                &dir.path().join("project.jner"),
                &p,
                ExportOptions::default(),
                &ExportControl::default()
            )
            .is_err()
        );
        let dest_dir = dir.path().join("folder.wav");
        std::fs::create_dir(&dest_dir).unwrap();
        assert!(
            export_wav(
                &dest_dir,
                &p,
                ExportOptions {
                    beats: 1,
                    sample_rate: 8_000,
                    ..Default::default()
                },
                &ExportControl::default()
            )
            .is_err()
        );
        assert!(dest_dir.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn muted_project_exports_silence_from_the_first_frame() {
        let mut p = Project::empty();
        p.pattern = Pattern::demo();
        for track in &mut p.mixer.tracks {
            track.muted = true;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("muted.wav");
        export_wav(
            &path,
            &p,
            ExportOptions {
                beats: 1,
                sample_rate: 8_000,
                ..Default::default()
            },
            &ExportControl::default(),
        )
        .unwrap();
        assert!(
            hound::WavReader::open(path)
                .unwrap()
                .samples::<i16>()
                .all(|v| v.unwrap() == 0)
        );
    }

    #[test]
    fn cancellation_during_render_keeps_the_old_file_and_removes_the_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel.wav");
        std::fs::write(&path, b"old wav").unwrap();
        let mut project = Project::empty();
        project.pattern = Pattern::demo();
        let control = Arc::new(ExportControl::default());
        let worker_control = Arc::clone(&control);
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            export_wav(
                &worker_path,
                &project,
                ExportOptions {
                    beats: MAX_BEATS,
                    sample_rate: 192_000,
                    ..Default::default()
                },
                &worker_control,
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while control.progress().0 == 0 {
            if std::time::Instant::now() >= deadline {
                control.cancel();
                let _ = worker.join();
                panic!("El render no empezó dentro del plazo");
            }
            std::thread::yield_now();
        }
        control.cancel();
        assert!(worker.join().unwrap().is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"old wav");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn suggested_range_covers_late_clips_and_note_release_and_mode_follows_playback() {
        let mut p = Project::empty();
        assert_eq!(RenderMode::from_project(&p), RenderMode::Pattern);
        assert_eq!(suggested_beats(&p, RenderMode::Pattern), 4);
        let source = Arc::new(crate::recording::synthetic_take().unwrap());
        p.timeline_sources.push(source);
        p.arrangement.add(0, 0, 256 * 960, 11_025).unwrap();
        assert_eq!(suggested_beats(&p, RenderMode::Timeline), 257);
        p.notes.add(0, 60, 100, 0, 960).unwrap();
        p.synth.tracks[0].release_ms = 1000.0;
        assert_eq!(suggested_beats(&p, RenderMode::PianoRoll), 3);
        p.workspace.timeline_playback = true;
        assert_eq!(RenderMode::from_project(&p), RenderMode::Timeline);
        p.workspace.timeline_playback = false;
        p.workspace.piano_playback = true;
        assert_eq!(RenderMode::from_project(&p), RenderMode::PianoRoll);
    }
}
