//! CPAL adapter. Device discovery and stream construction happen on the caller thread.
use std::{
    borrow::Borrow,
    num::{NonZeroU16, NonZeroU32},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

use cpal::{
    FromSample, Sample, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::engine::{Command, Engine};
use crate::{
    automation::{Automation, MAX_LANES, Target},
    effects::EffectsState,
    midi::{MidiPlayer, NoteSequence},
    mixer::MixerState,
    samples::{SampleBank, SampleSource},
    sequencer::{Bpm, Pattern, TRACKS},
    synth::SynthState,
    timeline::{Arrangement, TimelinePlayer},
};

pub const COMMAND_CAPACITY: usize = 64;

pub struct PlaybackSettings<'a> {
    pub bpm: Bpm,
    pub mixer: MixerState,
    pub synth: SynthState,
    pub effects: EffectsState,
    pub automation: &'a Automation,
    pub record_clock: Option<Arc<crate::recording::CaptureClock>>,
}

pub struct OutputDevice {
    pub name: String,
    pub is_default: bool,
    pub device: cpal::Device,
}

pub fn output_devices() -> Result<Vec<OutputDevice>, String> {
    let host = cpal::default_host();
    let default = host.default_output_device();
    let devices = host
        .output_devices()
        .map_err(|e| format!("No se pudieron enumerar las salidas: {e}"))?;
    // WASAPI represents the system default as a logical device, distinct from
    // physical endpoints. Keep that choice explicitly instead of comparing handles.
    Ok(default
        .into_iter()
        .map(|device| (device, true))
        .chain(devices.map(|device| (device, false)))
        .map(|(device, is_default)| OutputDevice {
            name: device.description().map_or_else(
                |_| "Dispositivo no disponible".into(),
                |description| description.name().to_owned(),
            ),
            is_default,
            device,
        })
        .collect())
}

#[derive(Default)]
struct Telemetry {
    position: AtomicU64,
    playing: AtomicBool,
    callbacks: AtomicU64,
    frames: AtomicU64,
    buffer_frames: AtomicU32,
    peak: AtomicU32,
    errors: AtomicU32,
    step: AtomicU32,
    step_events: AtomicU64,
    stereo_peaks: [AtomicU32; 2],
    track_peaks: [AtomicU32; TRACKS],
    musical_ticks: AtomicU64,
    clip_triggers: AtomicU64,
    note_ons: AtomicU64,
    note_offs: AtomicU64,
    active_notes: AtomicU32,
    synth_voices: AtomicU32,
    automation_mask: AtomicU32,
    automation_values: [AtomicU32; MAX_LANES],
}

/// Approximate, latest-callback telemetry; never the source of the audio clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct Snapshot {
    pub position: u64,
    pub playing: bool,
    pub callbacks: u64,
    pub frames: u64,
    pub buffer_frames: u32,
    pub peak: f32,
    pub errors: u32,
    pub step: Option<u8>,
    pub step_events: u64,
    pub stereo_peaks: [f32; 2],
    pub track_peaks: [f32; TRACKS],
    pub musical_ticks: u64,
    pub clip_triggers: u64,
    pub note_ons: u64,
    pub note_offs: u64,
    pub active_notes: u32,
    pub synth_voices: u32,
    pub automation_values: [Option<f32>; MAX_LANES],
}

#[derive(Debug)]
pub struct StreamInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: cpal::SampleFormat,
}

pub struct AudioOutput {
    // Drop the stream before the producer and telemetry.
    _stream: cpal::Stream,
    commands: Producer<Command>,
    telemetry: Arc<Telemetry>,
    pub info: StreamInfo,
}

impl AudioOutput {
    pub fn open(device: &cpal::Device) -> Result<Self, String> {
        Self::open_with(device, |rate| Ok(Engine::new(rate)))
    }

    pub fn open_drums(
        device: &cpal::Device,
        sources: &[impl Borrow<SampleSource>; TRACKS],
        pattern: Pattern,
        bpm: Bpm,
        mixer: MixerState,
    ) -> Result<Self, String> {
        Self::open_drums_with_effects(
            device,
            sources,
            pattern,
            bpm,
            mixer,
            EffectsState::default(),
        )
    }

    pub fn open_drums_with_effects(
        device: &cpal::Device,
        sources: &[impl Borrow<SampleSource>; TRACKS],
        pattern: Pattern,
        bpm: Bpm,
        mixer: MixerState,
        effects: EffectsState,
    ) -> Result<Self, String> {
        Self::open_drums_with_settings(
            device,
            sources,
            pattern,
            &PlaybackSettings {
                bpm,
                mixer,
                synth: SynthState::default(),
                effects,
                automation: &Automation::default(),
                record_clock: None,
            },
        )
    }
    pub fn open_drums_with_settings(
        device: &cpal::Device,
        sources: &[impl Borrow<SampleSource>; TRACKS],
        pattern: Pattern,
        settings: &PlaybackSettings<'_>,
    ) -> Result<Self, String> {
        Self::open_with_capture(device, settings.record_clock.clone(), |rate| {
            let mut engine =
                Engine::with_drums(SampleBank::prepare(sources, rate)?, pattern, settings.bpm);
            engine.configure_effects(settings.effects)?;
            engine.configure_automation(settings.automation)?;
            engine.apply(Command::SetMixer(settings.mixer));
            Ok(engine)
        })
    }

    fn open_with(
        device: &cpal::Device,
        make_engine: impl FnOnce(NonZeroU32) -> Result<Engine, String>,
    ) -> Result<Self, String> {
        Self::open_with_capture(device, None, make_engine)
    }
    fn open_with_capture(
        device: &cpal::Device,
        record_clock: Option<Arc<crate::recording::CaptureClock>>,
        make_engine: impl FnOnce(NonZeroU32) -> Result<Engine, String>,
    ) -> Result<Self, String> {
        let supported = device
            .default_output_config()
            .map_err(|e| format!("Configuración de salida no disponible: {e}"))?;
        let sample_format = supported.sample_format();
        let config = supported.config();
        let sample_rate = NonZeroU32::new(config.sample_rate)
            .ok_or("El dispositivo indicó un sample rate de cero")?;
        let channels =
            NonZeroU16::new(config.channels).ok_or("El dispositivo indicó cero canales")?;
        let (commands, consumer) = RingBuffer::new(COMMAND_CAPACITY);
        let telemetry = Arc::new(Telemetry::default());
        let renderer = Renderer {
            engine: make_engine(sample_rate)?,
            commands: consumer,
            telemetry: Arc::clone(&telemetry),
            channels,
            record_clock,
        };
        let error_telemetry = Arc::clone(&telemetry);
        let on_error = move |_error| {
            // No formatting or logging on CPAL's error thread. UI handles recovery.
            error_telemetry.errors.fetch_add(1, Ordering::Relaxed);
        };
        macro_rules! stream {
            ($ty:ty) => {
                build_stream::<$ty>(device, config, renderer, on_error)
            };
        }
        let stream = match sample_format {
            cpal::SampleFormat::F32 => stream!(f32),
            cpal::SampleFormat::F64 => stream!(f64),
            cpal::SampleFormat::I8 => stream!(i8),
            cpal::SampleFormat::I16 => stream!(i16),
            cpal::SampleFormat::I24 => stream!(cpal::I24),
            cpal::SampleFormat::I32 => stream!(i32),
            cpal::SampleFormat::I64 => stream!(i64),
            cpal::SampleFormat::U8 => stream!(u8),
            cpal::SampleFormat::U16 => stream!(u16),
            cpal::SampleFormat::U24 => stream!(cpal::U24),
            cpal::SampleFormat::U32 => stream!(u32),
            cpal::SampleFormat::U64 => stream!(u64),
            other => return Err(format!("Formato de audio no compatible: {other}")),
        }
        .map_err(|e| format!("No se pudo abrir la salida: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("No se pudo iniciar el stream: {e}"))?;
        Ok(Self {
            _stream: stream,
            commands,
            telemetry,
            info: StreamInfo {
                sample_rate: sample_rate.get(),
                channels: channels.get(),
                sample_format,
            },
        })
    }

    pub fn open_timeline(
        device: &cpal::Device,
        sources: &[Arc<SampleSource>],
        arrangement: &Arrangement,
        bpm: Bpm,
        mixer: MixerState,
    ) -> Result<Self, String> {
        Self::open_timeline_with_effects(
            device,
            sources,
            arrangement,
            bpm,
            mixer,
            EffectsState::default(),
        )
    }

    pub fn open_timeline_with_effects(
        device: &cpal::Device,
        sources: &[Arc<SampleSource>],
        arrangement: &Arrangement,
        bpm: Bpm,
        mixer: MixerState,
        effects: EffectsState,
    ) -> Result<Self, String> {
        Self::open_timeline_with_settings(
            device,
            sources,
            arrangement,
            &PlaybackSettings {
                bpm,
                mixer,
                synth: SynthState::default(),
                effects,
                automation: &Automation::default(),
                record_clock: None,
            },
        )
    }
    pub fn open_timeline_with_settings(
        device: &cpal::Device,
        sources: &[Arc<SampleSource>],
        arrangement: &Arrangement,
        settings: &PlaybackSettings<'_>,
    ) -> Result<Self, String> {
        Self::open_with_capture(device, settings.record_clock.clone(), |rate| {
            let mut engine = Engine::with_timeline(
                rate,
                TimelinePlayer::prepare(arrangement, sources, rate)?,
                settings.bpm,
            );
            engine.configure_effects(settings.effects)?;
            engine.configure_automation(settings.automation)?;
            engine.apply(Command::SetMixer(settings.mixer));
            Ok(engine)
        })
    }

    pub fn open_midi(
        device: &cpal::Device,
        notes: &NoteSequence,
        bpm: Bpm,
        mixer: MixerState,
    ) -> Result<Self, String> {
        Self::open_with(device, |rate| {
            let mut engine = Engine::with_midi(rate, MidiPlayer::prepare(notes), bpm);
            engine.apply(Command::SetMixer(mixer));
            Ok(engine)
        })
    }

    pub fn open_synth(
        device: &cpal::Device,
        notes: &NoteSequence,
        bpm: Bpm,
        mixer: MixerState,
        state: SynthState,
    ) -> Result<Self, String> {
        Self::open_synth_with_effects(device, notes, bpm, mixer, state, EffectsState::default())
    }

    pub fn open_synth_with_effects(
        device: &cpal::Device,
        notes: &NoteSequence,
        bpm: Bpm,
        mixer: MixerState,
        state: SynthState,
        effects: EffectsState,
    ) -> Result<Self, String> {
        Self::open_synth_with_settings(
            device,
            notes,
            &PlaybackSettings {
                bpm,
                mixer,
                synth: state,
                effects,
                automation: &Automation::default(),
                record_clock: None,
            },
        )
    }
    pub fn open_synth_with_settings(
        device: &cpal::Device,
        notes: &NoteSequence,
        settings: &PlaybackSettings<'_>,
    ) -> Result<Self, String> {
        Self::open_with_capture(device, settings.record_clock.clone(), |rate| {
            let mut engine = Engine::with_synth(
                rate,
                MidiPlayer::prepare(notes),
                settings.bpm,
                settings.synth,
            )?;
            engine.configure_effects(settings.effects)?;
            engine.configure_automation(settings.automation)?;
            engine.apply(Command::SetMixer(settings.mixer));
            Ok(engine)
        })
    }

    /// A full queue rejects the command explicitly; callers can retry, without waiting.
    pub fn send(&mut self, command: Command) -> Result<(), &'static str> {
        self.commands
            .push(command)
            .map_err(|_| "La cola de audio está llena. Reintenta la acción.")
    }

    pub fn snapshot(&self) -> Snapshot {
        let t = &self.telemetry;
        Snapshot {
            position: t.position.load(Ordering::Relaxed),
            playing: t.playing.load(Ordering::Relaxed),
            callbacks: t.callbacks.load(Ordering::Relaxed),
            frames: t.frames.load(Ordering::Relaxed),
            buffer_frames: t.buffer_frames.load(Ordering::Relaxed),
            peak: f32::from_bits(t.peak.load(Ordering::Relaxed)),
            errors: t.errors.load(Ordering::Relaxed),
            step: t
                .step
                .load(Ordering::Relaxed)
                .checked_sub(1)
                .map(|s| s as u8),
            step_events: t.step_events.load(Ordering::Relaxed),
            stereo_peaks: std::array::from_fn(|side| {
                f32::from_bits(t.stereo_peaks[side].load(Ordering::Relaxed))
            }),
            track_peaks: std::array::from_fn(|track| {
                f32::from_bits(t.track_peaks[track].load(Ordering::Relaxed))
            }),
            musical_ticks: t.musical_ticks.load(Ordering::Relaxed),
            clip_triggers: t.clip_triggers.load(Ordering::Relaxed),
            note_ons: t.note_ons.load(Ordering::Relaxed),
            note_offs: t.note_offs.load(Ordering::Relaxed),
            active_notes: t.active_notes.load(Ordering::Relaxed),
            synth_voices: t.synth_voices.load(Ordering::Relaxed),
            automation_values: std::array::from_fn(|index| {
                if t.automation_mask.load(Ordering::Relaxed) & (1 << index) != 0 {
                    Some(f32::from_bits(
                        t.automation_values[index].load(Ordering::Relaxed),
                    ))
                } else {
                    None
                }
            }),
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut renderer: Renderer,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    device.build_output_stream(
        config,
        move |data: &mut [T], info| renderer.render_timed(data, Some(info)),
        on_error,
        Some(Duration::from_secs(3)),
    )
}

struct Renderer {
    engine: Engine,
    commands: Consumer<Command>,
    telemetry: Arc<Telemetry>,
    channels: NonZeroU16,
    record_clock: Option<Arc<crate::recording::CaptureClock>>,
}

impl Renderer {
    #[cfg(test)]
    fn render<T: Sample + FromSample<f32>>(&mut self, data: &mut [T]) {
        self.render_timed(data, None);
    }
    fn render_timed<T: Sample + FromSample<f32>>(
        &mut self,
        data: &mut [T],
        info: Option<&cpal::OutputCallbackInfo>,
    ) {
        // Bound command work even if the UI keeps sending during the callback.
        for _ in 0..COMMAND_CAPACITY {
            let Ok(command) = self.commands.pop() else {
                break;
            };
            if let Some(clock) = &self.record_clock {
                match command {
                    Command::Play if !self.engine.is_playing() => {
                        clock.output_transition(true, info)
                    }
                    Command::Stop if self.engine.is_playing() => {
                        clock.output_transition(false, info)
                    }
                    _ => {}
                }
            }
            self.engine.apply(command);
        }
        let channels = usize::from(self.channels.get());
        let mut frames = data.chunks_exact_mut(channels);
        let mut peak = 0.0_f32;
        let mut stereo_peaks = [0.0_f32; 2];
        let mut track_peaks = [0.0_f32; TRACKS];
        for frame in &mut frames {
            let stereo = self.engine.next_frame();
            for (side, value) in stereo.into_iter().enumerate() {
                stereo_peaks[side] = stereo_peaks[side].max(value.abs());
            }
            for (track, value) in self.engine.track_levels().into_iter().enumerate() {
                track_peaks[track] = track_peaks[track].max(value);
            }
            if channels == 1 {
                let mono = (stereo[0] + stereo[1]) * 0.5;
                frame[0] = T::from_sample(mono);
                peak = peak.max(mono.abs());
            } else {
                frame[0] = T::from_sample(stereo[0]);
                frame[1] = T::from_sample(stereo[1]);
                frame[2..].fill(T::EQUILIBRIUM);
                peak = peak.max(stereo_peaks[0]).max(stereo_peaks[1]);
            }
        }
        // Defensive handling of a malformed partial frame: silence, no clock advance.
        frames.into_remainder().fill(T::EQUILIBRIUM);
        let count = data.len() / channels;
        let t = &self.telemetry;
        t.position.store(self.engine.position(), Ordering::Relaxed);
        t.musical_ticks
            .store(self.engine.musical_ticks(), Ordering::Relaxed);
        t.clip_triggers
            .store(self.engine.clip_triggers(), Ordering::Relaxed);
        let (ons, offs, active) = self.engine.note_counts();
        t.note_ons.store(ons, Ordering::Relaxed);
        t.note_offs.store(offs, Ordering::Relaxed);
        t.active_notes.store(active, Ordering::Relaxed);
        t.synth_voices
            .store(self.engine.synth_voices(), Ordering::Relaxed);
        let values = self.engine.automation_values();
        let mut mask = 0;
        for (index, target) in Target::ALL.into_iter().enumerate() {
            if let Some(value) = values.get(target) {
                mask |= 1 << index;
                t.automation_values[index].store(value.to_bits(), Ordering::Relaxed);
            }
        }
        t.automation_mask.store(mask, Ordering::Relaxed);
        t.playing.store(self.engine.is_playing(), Ordering::Relaxed);
        t.step.store(
            self.engine
                .current_step()
                .map_or(0, |step| u32::from(step) + 1),
            Ordering::Relaxed,
        );
        t.step_events
            .store(self.engine.step_events(), Ordering::Relaxed);
        t.frames.fetch_add(count as u64, Ordering::Relaxed);
        t.buffer_frames.store(count as u32, Ordering::Relaxed);
        t.peak.store(peak.to_bits(), Ordering::Relaxed);
        for (side, value) in stereo_peaks.into_iter().enumerate() {
            t.stereo_peaks[side].store(value.to_bits(), Ordering::Relaxed);
        }
        for (track, value) in track_peaks.into_iter().enumerate() {
            t.track_peaks[track].store(value.to_bits(), Ordering::Relaxed);
        }
        t.callbacks.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer(channels: u16) -> (Producer<Command>, Renderer) {
        let (producer, consumer) = RingBuffer::new(COMMAND_CAPACITY);
        (
            producer,
            Renderer {
                engine: Engine::new(NonZeroU32::new(48_000).unwrap()),
                commands: consumer,
                telemetry: Arc::new(Telemetry::default()),
                channels: NonZeroU16::new(channels).unwrap(),
                record_clock: None,
            },
        )
    }

    #[test]
    fn clock_counts_frames_and_duplicates_channels() {
        let (mut commands, mut renderer) = renderer(2);
        commands.push(Command::Play).unwrap();
        let mut stereo = [0.0; 513];
        renderer.render(&mut stereo);
        assert_eq!(renderer.engine.position(), 256);
        assert!(
            stereo[..512]
                .chunks_exact(2)
                .all(|frame| frame[0] == frame[1])
        );
        assert_eq!(stereo[512], 0.0);
        assert!(stereo.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn buffer_boundaries_do_not_change_tone_or_clock() {
        let (mut a_commands, mut a) = renderer(1);
        let (mut b_commands, mut b) = renderer(1);
        a_commands.push(Command::Play).unwrap();
        b_commands.push(Command::Play).unwrap();
        let mut whole = [0.0; 4096];
        let mut partitioned = [0.0; 4096];
        a.render(&mut whole);
        for chunk in partitioned.chunks_mut(127) {
            b.render(chunk);
        }
        assert_eq!(whole, partitioned);
        assert_eq!(a.engine.position(), b.engine.position());
    }

    #[test]
    fn full_command_queue_rejects_without_overwriting_stop() {
        let (mut commands, mut renderer) = renderer(1);
        commands.push(Command::Play).unwrap();
        for _ in 1..COMMAND_CAPACITY - 1 {
            commands.push(Command::SetGain(0.1)).unwrap();
        }
        commands.push(Command::Stop).unwrap();
        assert!(commands.push(Command::Play).is_err());
        renderer.render(&mut [0.0_f32; 128]);
        assert!(!renderer.engine.is_playing());
        assert_eq!(renderer.engine.position(), 0);
        assert!(commands.push(Command::Play).is_ok());
    }

    #[test]
    fn recording_is_armed_when_the_audio_callback_consumes_play() {
        let (mut commands, mut renderer) = renderer(2);
        let clock = Arc::new(crate::recording::CaptureClock::new());
        renderer.record_clock = Some(Arc::clone(&clock));
        renderer.render(&mut [0.0_f32; 128]);
        assert!(!clock.started());
        commands.push(Command::Play).unwrap();
        assert!(!clock.started());
        renderer.render(&mut [0.0_f32; 128]);
        assert!(clock.started());
        assert_eq!(renderer.engine.position(), 64);
        commands.push(Command::Stop).unwrap();
        renderer.render(&mut [0.0_f32; 128]);
        assert!(!renderer.engine.is_playing());
    }

    #[test]
    fn unsigned_silence_is_midpoint() {
        let (_commands, mut renderer) = renderer(1);
        let mut output = [0_u16; 32];
        renderer.render(&mut output);
        assert!(output.iter().all(|&sample| sample == u16::EQUILIBRIUM));
    }

    #[test]
    fn stereo_pan_reaches_the_device_and_extra_channels_remain_silent() {
        for channels in [1, 2, 6] {
            let (mut commands, mut renderer) = renderer(channels);
            let bank = SampleBank {
                rate: NonZeroU32::new(48_000).unwrap(),
                samples: std::array::from_fn(|_| vec![1.0; 4096].into()),
            };
            let mut pattern = Pattern::default();
            pattern.set(0, 0, true);
            renderer.engine = Engine::with_drums(bank, pattern, Bpm::default());
            let mut mixer = MixerState::default();
            mixer.tracks[0].pan = 1.0;
            commands.push(Command::SetMixer(mixer)).unwrap();
            let mut output = vec![0.0_f32; usize::from(channels) * 512];
            renderer.render(&mut output); // settle mixer while transport is stopped
            commands.push(Command::Play).unwrap();
            renderer.render(&mut output);
            let last = &output[output.len() - usize::from(channels)..];
            if channels == 1 {
                assert_eq!(last[0], crate::engine::DEFAULT_GAIN * 0.5);
            } else {
                assert_eq!(last[0], 0.0);
                assert_eq!(last[1], crate::engine::DEFAULT_GAIN);
                assert!(last[2..].iter().all(|&s| s == 0.0));
            }
            assert_eq!(renderer.engine.position(), 512);
        }
    }

    #[test]
    fn stereo_mixing_is_independent_of_callback_partitioning() {
        let (mut a_commands, mut a) = renderer(2);
        let (mut b_commands, mut b) = renderer(2);
        let sources = crate::samples::starter_kit();
        let bank = SampleBank::prepare(&sources, NonZeroU32::new(48_000).unwrap()).unwrap();
        a.engine = Engine::with_drums(bank.clone(), Pattern::demo(), Bpm::new(137).unwrap());
        b.engine = Engine::with_drums(bank, Pattern::demo(), Bpm::new(137).unwrap());
        let mut mixer = MixerState::default();
        mixer.tracks[0].pan = -1.0;
        mixer.tracks[2].pan = 0.7;
        for commands in [&mut a_commands, &mut b_commands] {
            commands.push(Command::SetMixer(mixer)).unwrap();
            commands.push(Command::Play).unwrap();
        }
        let mut whole = vec![0.0; 100_000];
        let mut partitioned = vec![0.0; 100_000];
        a.render(&mut whole);
        for buffer in partitioned.chunks_mut(254) {
            b.render(buffer);
        }
        assert_eq!(whole, partitioned);
        assert_eq!(a.engine.position(), b.engine.position());
        assert_eq!(a.engine.step_events(), b.engine.step_events());
    }

    #[test]
    fn midi_events_and_activity_do_not_depend_on_callback_boundaries() {
        let (mut a_commands, mut a) = renderer(2);
        let (mut b_commands, mut b) = renderer(2);
        let mut notes = NoteSequence::default();
        notes.add(0, 60, 99, 0, 240).unwrap();
        notes.add(1, 67, 127, 240, 480).unwrap();
        let rate = NonZeroU32::new(48_000).unwrap();
        a.engine = Engine::with_midi(rate, MidiPlayer::prepare(&notes), Bpm::new(137).unwrap());
        b.engine = Engine::with_midi(rate, MidiPlayer::prepare(&notes), Bpm::new(137).unwrap());
        a_commands.push(Command::Play).unwrap();
        b_commands.push(Command::Play).unwrap();
        let mut whole = vec![0.0_f32; 100_000];
        let mut chunks = whole.clone();
        a.render(&mut whole);
        for buffer in chunks.chunks_mut(254) {
            b.render(buffer);
        }
        assert!(whole.iter().all(|v| *v == 0.0));
        assert_eq!(whole, chunks);
        assert_eq!(a.engine.note_counts(), (2, 2, 0));
        assert_eq!(a.engine.note_counts(), b.engine.note_counts());
        assert_eq!(a.engine.musical_ticks(), b.engine.musical_ticks());
        assert_eq!(b.telemetry.note_offs.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn effects_and_live_commands_are_independent_of_callback_partitioning() {
        let (mut a_commands, mut a) = renderer(2);
        let (mut b_commands, mut b) = renderer(2);
        let mut state = EffectsState {
            delay_enabled: true,
            delay_ms: 25.0,
            feedback: 0.7,
            delay_mix: 0.6,
            distortion_enabled: true,
            drive: 4.0,
            distortion_mix: 0.8,
        };
        a.engine.configure_effects(state).unwrap();
        b.engine.configure_effects(state).unwrap();
        a_commands.push(Command::Play).unwrap();
        b_commands.push(Command::Play).unwrap();
        let mut whole = vec![0.0_f32; 120_000];
        let mut parts = whole.clone();
        a.render(&mut whole[..20000]);
        for buffer in parts[..20000].chunks_mut(254) {
            b.render(buffer);
        }
        state.delay_ms = 130.5;
        state.feedback = 0.4;
        state.distortion_enabled = false;
        let command = Command::SetEffects(state.prepare(NonZeroU32::new(48_000).unwrap()).unwrap());
        a_commands.push(command).unwrap();
        b_commands.push(command).unwrap();
        a.render(&mut whole[20000..]);
        for buffer in parts[20000..].chunks_mut(510) {
            b.render(buffer);
        }
        assert_eq!(whole, parts);
        assert!(whole.iter().any(|v| v.abs() > 0.001));
        assert_eq!(a.engine.position(), 60000);
        assert_eq!(a.engine.position(), b.engine.position());
    }

    #[test]
    fn automation_audio_and_telemetry_do_not_depend_on_callback_partitioning() {
        let (mut ac, mut a) = renderer(2);
        let (mut bc, mut b) = renderer(2);
        let rate = NonZeroU32::new(48_000).unwrap();
        let sources = crate::samples::starter_kit();
        let bank = SampleBank::prepare(&sources, rate).unwrap();
        a.engine = Engine::with_drums(bank.clone(), Pattern::demo(), Bpm::new(137).unwrap());
        b.engine = Engine::with_drums(bank, Pattern::demo(), Bpm::new(137).unwrap());
        let mut curves = Automation::default();
        for (target, start, end) in [
            (Target::TrackGain(0), 0.0, 2.0),
            (Target::TrackPan(0), -1.0, 1.0),
            (Target::DelayMix, 0.0, 0.8),
        ] {
            curves.add(target, 0, start).unwrap();
            curves.add(target, 1920, end).unwrap();
        }
        for e in [&mut a.engine, &mut b.engine] {
            e.configure_effects(EffectsState {
                delay_enabled: true,
                ..Default::default()
            })
            .unwrap();
            e.configure_automation(&curves).unwrap();
        }
        ac.push(Command::Play).unwrap();
        bc.push(Command::Play).unwrap();
        let mut whole = vec![0.0_f32; 120_000];
        let mut parts = whole.clone();
        a.render(&mut whole[..20000]);
        for buffer in parts[..20000].chunks_mut(254) {
            b.render(buffer);
        }
        ac.push(Command::SetBpm(Bpm::new(240).unwrap())).unwrap();
        bc.push(Command::SetBpm(Bpm::new(240).unwrap())).unwrap();
        a.render(&mut whole[20000..]);
        for buffer in parts[20000..].chunks_mut(510) {
            b.render(buffer);
        }
        assert_eq!(whole, parts);
        assert!(whole.iter().any(|v| v.abs() > 0.001));
        assert_eq!(a.engine.automation_values(), b.engine.automation_values());
        assert_eq!(
            a.telemetry
                .automation_mask
                .load(Ordering::Relaxed)
                .count_ones(),
            3
        );
    }

    #[test]
    fn synth_render_is_identical_across_callback_partitions_and_releases_all_voices() {
        let (mut a_commands, mut a) = renderer(2);
        let (mut b_commands, mut b) = renderer(2);
        let mut notes = NoteSequence::default();
        notes.add(0, 60, 100, 0, 240).unwrap();
        notes.add(1, 64, 91, 240, 480).unwrap();
        let rate = NonZeroU32::new(48_000).unwrap();
        let mut state = SynthState::default();
        state.tracks[1].waveform = crate::synth::Waveform::Saw;
        a.engine = Engine::with_synth(
            rate,
            MidiPlayer::prepare(&notes),
            Bpm::new(137).unwrap(),
            state,
        )
        .unwrap();
        b.engine = Engine::with_synth(
            rate,
            MidiPlayer::prepare(&notes),
            Bpm::new(137).unwrap(),
            state,
        )
        .unwrap();
        a_commands.push(Command::Play).unwrap();
        b_commands.push(Command::Play).unwrap();
        let mut whole = vec![0.0_f32; 100_000];
        let mut chunks = whole.clone();
        a.render(&mut whole);
        for buffer in chunks.chunks_mut(254) {
            b.render(buffer);
        }
        assert_eq!(whole, chunks);
        assert!(whole.iter().any(|v| v.abs() > 0.001));
        assert!(whole.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        assert_eq!(a.engine.synth_voices(), 0);
        assert_eq!(b.engine.synth_voices(), 0);
        assert_eq!(a.engine.note_counts(), (2, 2, 0));
        assert!(whole[whole.len() - 200..].iter().all(|v| *v == 0.0));
    }
}
