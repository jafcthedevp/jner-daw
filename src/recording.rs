//! Bounded mono capture. Callbacks exchange Copy packets; assembly runs in a worker.
use crate::samples::SampleSource;
use cpal::{
    FromSample, Sample, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub const MAX_SECONDS: u32 = 10;
const CAPACITY: usize = 32_768;
const UNSET: u64 = u64::MAX;

pub struct InputDevice {
    pub name: String,
    pub is_default: bool,
    pub device: cpal::Device,
}
pub fn input_devices() -> Result<Vec<InputDevice>, String> {
    let host = cpal::default_host();
    let default = host.default_input_device();
    let devices = host
        .input_devices()
        .map_err(|e| format!("No se pudieron enumerar entradas: {e}"))?;
    Ok(default
        .into_iter()
        .map(|d| (d, true))
        .chain(devices.map(|d| (d, false)))
        .map(|(device, is_default)| InputDevice {
            name: device
                .description()
                .map_or_else(|_| "Entrada no disponible".into(), |d| d.name().to_owned()),
            is_default,
            device,
        })
        .collect())
}

pub struct CaptureClock {
    epoch: Instant,
    start: AtomicU64,
    stop: AtomicU64,
    done: AtomicBool,
    cancelled: AtomicBool,
    error: AtomicU32,
    frames: AtomicU32,
    peak: AtomicU32,
    estimated_timing: AtomicBool,
}
impl CaptureClock {
    pub(crate) fn new() -> Self {
        Self {
            epoch: Instant::now(),
            start: AtomicU64::new(UNSET),
            stop: AtomicU64::new(UNSET),
            done: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            error: AtomicU32::new(0),
            frames: AtomicU32::new(0),
            peak: AtomicU32::new(0),
            estimated_timing: AtomicBool::new(false),
        }
    }
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_nanos().min(u128::from(UNSET - 1)) as u64
    }
    fn fail(&self, code: u32) {
        let _ = self
            .error
            .compare_exchange(0, code, Ordering::Relaxed, Ordering::Relaxed);
        self.done.store(true, Ordering::Release);
    }
    pub(crate) fn output_transition(&self, playing: bool, info: Option<&cpal::OutputCallbackInfo>) {
        // CPAL does not guarantee a shared StreamInstant origin across streams.
        // Map only within-stream latency to our common monotonic epoch. This is
        // approximate physical alignment, not a calibrated duplex sample clock.
        let delay = if let Some(info) = info {
            let t = info.timestamp();
            match t.playback.checked_duration_since(t.callback) {
                Some(d) => d,
                None => {
                    self.estimated_timing.store(true, Ordering::Relaxed);
                    Duration::ZERO
                }
            }
        } else {
            Duration::ZERO
        };
        let time = self
            .now()
            .saturating_add(delay.as_nanos().min(u128::from(UNSET - 1)) as u64)
            .min(UNSET - 1);
        let slot = if playing { &self.start } else { &self.stop };
        let _ = slot.compare_exchange(UNSET, time, Ordering::Release, Ordering::Relaxed);
    }
    pub fn elapsed_seconds(&self, rate: u32) -> f32 {
        self.frames.load(Ordering::Relaxed) as f32 / rate as f32
    }
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }
    pub fn started(&self) -> bool {
        self.start.load(Ordering::Acquire) != UNSET
    }
    pub fn uses_estimated_timing(&self) -> bool {
        self.estimated_timing.load(Ordering::Relaxed)
    }
    fn capture_anchor(&self, now: u64, reported: Option<Duration>, packet: Duration) -> u64 {
        let latency = reported.unwrap_or_else(|| {
            self.estimated_timing.store(true, Ordering::Relaxed);
            packet
        });
        now.saturating_sub(latency.as_nanos().min(u128::from(UNSET - 1)) as u64)
    }
}
#[derive(Clone, Copy)]
struct CapturedFrame {
    index: u32,
    value: f32,
}
struct InputProcessor {
    rate: u32,
    channels: usize,
    anchor: Option<u64>,
    frames: u64,
    producer: Producer<CapturedFrame>,
    clock: Arc<CaptureClock>,
}
impl InputProcessor {
    fn process<T: Sample + Copy>(&mut self, data: &[T], capture_ns: u64)
    where
        f32: FromSample<T>,
    {
        if self.clock.done.load(Ordering::Acquire) || self.clock.cancelled.load(Ordering::Relaxed) {
            return;
        }
        if !data.len().is_multiple_of(self.channels)
            || data.len() / self.channels > self.rate as usize
        {
            self.clock.fail(4);
            return;
        }
        let anchor = *self.anchor.get_or_insert(capture_ns);
        // Advance by captured samples after the first timestamp. Later callback
        // arrival jitter must not move or duplicate samples within the take.
        let start = self.clock.start.load(Ordering::Acquire);
        let stop = self.clock.stop.load(Ordering::Acquire);
        let mut peak = 0.0_f32;
        for frame in data.chunks_exact(self.channels) {
            let time = i128::from(anchor) * i128::from(self.rate)
                + i128::from(self.frames) * 1_000_000_000;
            self.frames += 1;
            if start == UNSET {
                continue;
            }
            let relative = time - i128::from(start) * i128::from(self.rate);
            if relative < 0 {
                continue;
            }
            let index = (relative / 1_000_000_000) as u64;
            if index >= u64::from(self.rate) * u64::from(MAX_SECONDS)
                || (stop != UNSET && time >= i128::from(stop) * i128::from(self.rate))
            {
                self.clock.done.store(true, Ordering::Release);
                break;
            }
            let mut mono = 0.0_f64;
            for sample in frame {
                let value = f32::from_sample(*sample);
                if !value.is_finite() {
                    self.clock.fail(5);
                    return;
                }
                mono += f64::from(value);
            }
            let mono = (mono / self.channels as f64).clamp(-1.0, 1.0) as f32;
            peak = peak.max(mono.abs());
            if self
                .producer
                .push(CapturedFrame {
                    index: index as u32,
                    value: mono,
                })
                .is_err()
            {
                self.clock.fail(2);
                return;
            }
            self.clock.frames.store(index as u32 + 1, Ordering::Relaxed);
        }
        self.clock.peak.store(peak.to_bits(), Ordering::Relaxed);
    }
}

fn collect(
    mut consumer: Consumer<CapturedFrame>,
    clock: Arc<CaptureClock>,
    rate: u32,
    name: String,
) -> Result<SampleSource, String> {
    let mut pcm = Vec::with_capacity((rate * MAX_SECONDS) as usize);
    loop {
        while let Ok(frame) = consumer.pop() {
            let index = frame.index as usize;
            if index >= (rate * MAX_SECONDS) as usize {
                return Err("La toma excedió el límite de duración".into());
            }
            if index < pcm.len() {
                return Err("El reloj de captura retrocedió".into());
            }
            pcm.resize(index, 0.0);
            pcm.push(frame.value);
        }
        if clock.done.load(Ordering::Acquire) && consumer.is_empty() {
            break;
        }
        if clock.cancelled.load(Ordering::Relaxed) {
            return Err("Toma cancelada".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if clock.cancelled.load(Ordering::Relaxed) {
        return Err("Toma cancelada".into());
    }
    match clock.error.load(Ordering::Relaxed) {
        0 => {}
        1 => return Err("Se interrumpió la entrada de audio".into()),
        2 => return Err("Se desbordó el buffer de grabación; repite la toma".into()),
        3 => return Err("El dispositivo informó timestamps no válidos".into()),
        4 => return Err("La entrada entregó un buffer no válido".into()),
        _ => return Err("La entrada entregó audio no finito".into()),
    }
    if pcm.is_empty() {
        return Err("La toma no contiene frames de audio".into());
    }
    let start = clock.start.load(Ordering::Acquire);
    let stop = clock.stop.load(Ordering::Acquire);
    if start != UNSET && stop != UNSET {
        let frames =
            (u128::from(stop.saturating_sub(start)) * u128::from(rate)).div_ceil(1_000_000_000);
        pcm.truncate(frames.min(pcm.len() as u128) as usize);
    }
    if pcm.is_empty() {
        return Err("La toma no contiene frames de audio".into());
    }
    SampleSource::from_pcm(name, rate, pcm)
}

pub struct RecordingSession {
    stream: Option<cpal::Stream>,
    pub clock: Arc<CaptureClock>,
    receiver: mpsc::Receiver<Result<SampleSource, String>>,
    pub sample_rate: u32,
    pub channels: u16,
}
impl RecordingSession {
    /// Opens the input only on an explicit recording action. No input is opened at app startup.
    pub fn open(device: &cpal::Device, name: String) -> Result<Self, String> {
        let supported = device
            .default_input_config()
            .map_err(|e| format!("Configuración de entrada no disponible: {e}"))?;
        let config = supported.config();
        let rate = config.sample_rate;
        if !(8_000..=192_000).contains(&rate) || !(1..=8).contains(&config.channels) {
            return Err("Entrada compatible: 8–192 kHz, de uno a ocho canales".into());
        }
        let clock = Arc::new(CaptureClock::new());
        let (producer, consumer) = RingBuffer::new(CAPACITY);
        let processor = InputProcessor {
            rate,
            channels: usize::from(config.channels),
            anchor: None,
            frames: 0,
            producer,
            clock: Arc::clone(&clock),
        };
        macro_rules! build {
            ($ty:ty) => {
                build_input::<$ty>(device, &config, processor)
            };
        }
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build!(f32),
            cpal::SampleFormat::F64 => build!(f64),
            cpal::SampleFormat::I8 => build!(i8),
            cpal::SampleFormat::I16 => build!(i16),
            cpal::SampleFormat::I24 => build!(cpal::I24),
            cpal::SampleFormat::I32 => build!(i32),
            cpal::SampleFormat::I64 => build!(i64),
            cpal::SampleFormat::U8 => build!(u8),
            cpal::SampleFormat::U16 => build!(u16),
            cpal::SampleFormat::U24 => build!(cpal::U24),
            cpal::SampleFormat::U32 => build!(u32),
            cpal::SampleFormat::U64 => build!(u64),
            format => return Err(format!("Formato de entrada no compatible: {format}")),
        }
        .map_err(|e| format!("No se pudo abrir la entrada: {e}"))?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_clock = Arc::clone(&clock);
        std::thread::Builder::new()
            .name("recording-collector".into())
            .spawn(move || {
                let _ = sender.send(collect(consumer, worker_clock, rate, name));
            })
            .map_err(|e| format!("No se pudo preparar la toma: {e}"))?;
        if let Err(error) = stream.play() {
            clock.cancelled.store(true, Ordering::Relaxed);
            clock.done.store(true, Ordering::Release);
            return Err(format!("No se pudo iniciar la entrada: {error}"));
        }
        Ok(Self {
            stream: Some(stream),
            clock,
            receiver,
            sample_rate: rate,
            channels: config.channels,
        })
    }
    pub fn poll(&self) -> Option<Result<SampleSource, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("El worker de grabación se interrumpió".into()))
            }
        }
    }
}
impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.clock.cancelled.store(true, Ordering::Relaxed);
        self.clock.done.store(true, Ordering::Release);
        self.stream.take();
    }
}
fn build_input<T: SizedSample + Copy>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut processor: InputProcessor,
) -> Result<cpal::Stream, cpal::Error>
where
    f32: FromSample<T>,
{
    let errors = Arc::clone(&processor.clock);
    device.build_input_stream(
        *config,
        move |data: &[T], info: &cpal::InputCallbackInfo| {
            // Only the first packet establishes the anchor. Some devices report
            // a capture timestamp newer than their callback clock snapshot.
            // Fall back explicitly to arrival minus this packet's duration;
            // never claim calibrated synchronization for such a take.
            let capture_ns = if processor.anchor.is_none() {
                let timestamp = info.timestamp();
                let reported = timestamp.callback.checked_duration_since(timestamp.capture);
                let packet_duration = Duration::from_secs_f64(
                    data.len() as f64 / processor.channels as f64 / f64::from(processor.rate),
                );
                processor
                    .clock
                    .capture_anchor(processor.clock.now(), reported, packet_duration)
            } else {
                0 // Subsequent packets use the sample counter, not arrival time.
            };
            processor.process(data, capture_ns);
        },
        move |_| errors.fail(1),
        Some(Duration::from_secs(3)),
    )
}

/// Deterministic capture-path check. Does not enumerate/open microphones or emit sound.
pub fn synthetic_take() -> Result<SampleSource, String> {
    let rate = 44_100;
    let clock = Arc::new(CaptureClock::new());
    let (producer, consumer) = RingBuffer::new(CAPACITY);
    let mut processor = InputProcessor {
        rate,
        channels: 2,
        anchor: None,
        frames: 0,
        producer,
        clock: Arc::clone(&clock),
    };
    processor.process(&vec![0.0_f32; 8820], 0); // 100 ms before the transport is armed.
    clock.start.store(100_000_000, Ordering::Release);
    clock.stop.store(350_000_000, Ordering::Release);
    let audio: Vec<_> = (0..11_026)
        .flat_map(|i| {
            let value = (std::f32::consts::TAU * 440.0 * i as f32 / rate as f32).sin() * 0.4;
            [value, value]
        })
        .collect();
    for block in audio.chunks(514) {
        processor.process(block, 0);
    }
    collect(consumer, clock, rate, "Toma sintética".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reversed_capture_timestamp_uses_a_visible_packet_estimate() {
        let clock = CaptureClock::new();
        assert_eq!(
            clock.capture_anchor(
                50_000_000,
                Some(Duration::from_millis(5)),
                Duration::from_millis(10)
            ),
            45_000_000
        );
        assert!(!clock.uses_estimated_timing());
        assert_eq!(
            clock.capture_anchor(50_000_000, None, Duration::from_millis(10)),
            40_000_000
        );
        assert!(clock.uses_estimated_timing());
        assert_eq!(clock.capture_anchor(1, None, Duration::from_millis(10)), 0);
    }
    fn setup(
        rate: u32,
        channels: usize,
        capacity: usize,
    ) -> (InputProcessor, Consumer<CapturedFrame>, Arc<CaptureClock>) {
        let clock = Arc::new(CaptureClock::new());
        let (producer, consumer) = RingBuffer::new(capacity);
        (
            InputProcessor {
                rate,
                channels,
                anchor: None,
                frames: 0,
                producer,
                clock: Arc::clone(&clock),
            },
            consumer,
            clock,
        )
    }
    #[test]
    fn recording_gate_aligns_start_stop_downmix_and_ignores_unarmed_audio() {
        let (mut p, c, clock) = setup(8_000, 2, 1024);
        p.process(&[0.9_f32; 160], 0);
        assert_eq!(clock.frames.load(Ordering::Relaxed), 0);
        clock.start.store(10_000_000, Ordering::Release);
        clock.stop.store(20_000_000, Ordering::Release);
        p.process(&[0.4_f32; 162], 10_000_000);
        let sample = collect(c, clock, 8_000, "own".into()).unwrap();
        assert_eq!(sample.frame_count(), 80);
        assert!(sample.pcm().iter().all(|v| *v == 0.4));
    }
    #[test]
    fn synthetic_capture_is_portable_mono_and_exact_quarter_second() {
        let sample = synthetic_take().unwrap();
        assert_eq!(sample.sample_rate(), 44_100);
        assert_eq!(sample.frame_count(), 11_025);
        assert_eq!(sample.pcm()[0], 0.0);
        assert!(sample.pcm().iter().any(|v| v.abs() > 0.3));
    }
    #[test]
    fn overflow_invalid_audio_and_cancel_are_errors_instead_of_shifted_clips() {
        let (mut p, c, clock) = setup(8_000, 1, 2);
        clock.start.store(0, Ordering::Release);
        p.process(&[0.1_f32; 4], 0);
        assert!(collect(c, clock, 8_000, "x".into()).is_err());
        let (mut p, c, clock) = setup(8_000, 1, 10);
        clock.start.store(0, Ordering::Release);
        p.process(&[f32::NAN], 0);
        assert!(collect(c, clock, 8_000, "x".into()).is_err());
        let (_, c, clock) = setup(8_000, 1, 10);
        clock.cancelled.store(true, Ordering::Relaxed);
        clock.done.store(true, Ordering::Release);
        assert!(collect(c, clock, 8_000, "x".into()).is_err());
    }
    #[test]
    fn late_input_is_padded_and_limit_stops_at_exactly_ten_seconds() {
        let (mut p, c, clock) = setup(8_000, 1, 81_000);
        clock.start.store(0, Ordering::Release);
        for _ in 0..11 {
            p.process(&[0.2_f32; 8_000], 1_000_000);
        }
        let sample = collect(c, clock, 8_000, "x".into()).unwrap();
        assert_eq!(sample.frame_count(), 80_000);
        assert_eq!(&sample.pcm()[..8], &[0.0; 8]);
    }
    #[test]
    fn integer_formats_downmix_and_partial_frames_are_rejected() {
        let (mut p, c, clock) = setup(8_000, 2, 16);
        clock.start.store(0, Ordering::Release);
        clock.stop.store(250_000, Ordering::Release);
        p.process(&[16_384_i16, 0, -16_384, 0, 0, 0], 0);
        let sample = collect(c, clock, 8_000, "signed".into()).unwrap();
        assert_eq!(sample.pcm(), &[0.25, -0.25]);
        let (mut p, c, clock) = setup(8_000, 1, 16);
        clock.start.store(0, Ordering::Release);
        clock.stop.store(250_000, Ordering::Release);
        p.process(&[32_768_u16, 49_152, 32_768], 0);
        assert_eq!(
            collect(c, clock, 8_000, "unsigned".into()).unwrap().pcm(),
            &[0.0, 0.5]
        );
        let (mut p, c, clock) = setup(8_000, 2, 16);
        p.process(&[0.0_f32; 3], 0);
        assert!(collect(c, clock, 8_000, "partial".into()).is_err());
    }
    #[test]
    fn stop_published_after_a_buffer_trims_already_queued_frames() {
        let (mut p, c, clock) = setup(8_000, 1, 128);
        clock.start.store(0, Ordering::Release);
        p.process(&[0.2_f32; 80], 0);
        clock.stop.store(5_000_000, Ordering::Release);
        p.process(&[0.2_f32], 10_000_000);
        assert_eq!(
            collect(c, clock, 8_000, "trim".into())
                .unwrap()
                .frame_count(),
            40
        );
    }
    #[test]
    fn capture_is_independent_of_callback_partition_and_later_callback_jitter() {
        fn capture(block: usize) -> SampleSource {
            let (mut p, c, clock) = setup(44_100, 2, 2048);
            clock.start.store(0, Ordering::Release);
            clock.stop.store(10_000_000, Ordering::Release);
            let input: Vec<_> = (0..442).flat_map(|i| [i as f32 / 442.0, 0.0]).collect();
            for (i, samples) in input.chunks(block * 2).enumerate() {
                p.process(samples, i as u64 * 123_456);
            }
            collect(c, clock, 44_100, "partition".into()).unwrap()
        }
        let whole = capture(442);
        assert_eq!(whole.frame_count(), 441);
        for block in [1, 17, 127] {
            assert_eq!(whole.pcm(), capture(block).pcm());
        }
    }
}
