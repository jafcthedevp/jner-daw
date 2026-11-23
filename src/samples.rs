//! WAV decoding and resampling. Everything here runs outside the audio callback.
use std::{borrow::Borrow, f64::consts::TAU, io::Read, num::NonZeroU32, path::Path, sync::Arc};

use rubato::{Fft, FixedSync, Resampler, audioadapter_buffers::direct::InterleavedSlice};

use crate::sequencer::TRACKS;

const MAX_SECONDS: u32 = 10;

pub struct SampleSource {
    pub name: String,
    rate: NonZeroU32,
    mono: Vec<f32>,
}

#[derive(Clone)]
pub struct SampleBank {
    pub rate: NonZeroU32,
    pub samples: [Arc<[f32]>; TRACKS],
}

impl SampleSource {
    pub(crate) fn pcm(&self) -> &[f32] {
        &self.mono
    }

    pub(crate) fn from_pcm(name: String, rate: u32, mono: Vec<f32>) -> Result<Self, String> {
        if name.len() > 1024
            || !(8_000..=192_000).contains(&rate)
            || mono.is_empty()
            || mono.len() > rate as usize * MAX_SECONDS as usize
            || mono.iter().any(|v| !v.is_finite() || v.abs() > 1.0)
        {
            return Err("Fuente de audio del proyecto no válida".into());
        }
        let rate = NonZeroU32::new(rate).ok_or("Sample rate cero")?;
        Ok(Self { name, rate, mono })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate.get()
    }
    pub fn frame_count(&self) -> u32 {
        self.mono.len() as u32
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let file =
            std::fs::File::open(path).map_err(|e| format!("No se pudo abrir el WAV: {e}"))?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Self::decode(std::io::BufReader::new(file), name)
    }

    pub fn decode(reader: impl Read, name: String) -> Result<Self, String> {
        let mut reader =
            hound::WavReader::new(reader).map_err(|e| format!("WAV no válido: {e}"))?;
        let spec = reader.spec();
        let rate = NonZeroU32::new(spec.sample_rate).ok_or("El WAV indica sample rate cero")?;
        if !(8_000..=192_000).contains(&spec.sample_rate) || !(1..=2).contains(&spec.channels) {
            return Err("Admite WAV mono o estéreo, de 8 a 192 kHz".into());
        }
        if reader.duration() == 0 || reader.duration() > spec.sample_rate * MAX_SECONDS {
            return Err("El sample debe durar más de cero y como máximo 10 segundos".into());
        }
        if !reader.len().is_multiple_of(u32::from(spec.channels)) {
            return Err("El WAV contiene un frame incompleto".into());
        }
        let interleaved: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float if spec.bits_per_sample == 32 => reader
                .samples::<f32>()
                .collect::<Result<_, _>>()
                .map_err(|e| format!("Datos WAV no válidos: {e}"))?,
            hound::SampleFormat::Int if matches!(spec.bits_per_sample, 8 | 16 | 24 | 32) => {
                let scale = (1_u64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / scale))
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("Datos PCM no válidos: {e}"))?
            }
            _ => return Err("Admite WAV PCM de 8/16/24/32 bits o float de 32 bits".into()),
        };
        if interleaved.iter().any(|s| !s.is_finite()) {
            return Err("El WAV contiene valores de audio no finitos".into());
        }
        let channels = usize::from(spec.channels);
        let mono = interleaved
            .chunks_exact(channels)
            .map(|frame| {
                (frame.iter().map(|&v| f64::from(v)).sum::<f64>() / channels as f64)
                    .clamp(-1.0, 1.0) as f32
            })
            .collect();
        Ok(Self { name, rate, mono })
    }

    pub(crate) fn prepare(&self, target: NonZeroU32) -> Result<Arc<[f32]>, String> {
        if self.rate == target {
            return Ok(Arc::from(self.mono.as_slice()));
        }
        let input =
            InterleavedSlice::new(&self.mono, 1, self.mono.len()).map_err(|e| e.to_string())?;
        let mut resampler = Fft::<f32>::new(
            self.rate.get() as usize,
            target.get() as usize,
            1024,
            1,
            FixedSync::Both,
        )
        .map_err(|e| format!("No se pudo preparar el resampling: {e}"))?;
        let mut output = resampler
            .process_all(&input, self.mono.len(), None)
            .map_err(|e| format!("No se pudo remuestrear el WAV: {e}"))?
            .take_data();
        for value in &mut output {
            *value = value.clamp(-1.0, 1.0);
        }
        Ok(output.into())
    }
}

impl SampleBank {
    pub fn prepare(
        sources: &[impl Borrow<SampleSource>; TRACKS],
        target: NonZeroU32,
    ) -> Result<Self, String> {
        if !(8_000..=192_000).contains(&target.get()) {
            return Err("La salida debe estar entre 8 y 192 kHz".into());
        }
        Ok(Self {
            rate: target,
            samples: [
                sources[0].borrow().prepare(target)?,
                sources[1].borrow().prepare(target)?,
                sources[2].borrow().prepare(target)?,
                sources[3].borrow().prepare(target)?,
            ],
        })
    }
}

/// Small synthesized one-shots, generated once on the application thread.
/// No external samples, assets or licensing requirements.
pub fn starter_kit() -> [SampleSource; TRACKS] {
    let rate = NonZeroU32::new(48_000).unwrap_or(NonZeroU32::MIN);
    let names = [
        "Kick interno",
        "Snare interno",
        "Hi-hat interno",
        "Clap interno",
    ];
    std::array::from_fn(|track| {
        let duration = [0.5, 0.3, 0.12, 0.3][track];
        let mut noise = 0x1234_5678_u32;
        let mut phase = 0.0;
        let mut previous_noise = 0.0;
        let mono = (0..(duration * f64::from(rate.get())) as usize)
            .map(|index| {
                let t = index as f64 / f64::from(rate.get());
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                let n = f64::from(noise) / f64::from(u32::MAX) * 2.0 - 1.0;
                let high = (n - previous_noise) * 0.5;
                previous_noise = n;
                phase += TAU * (48.0 + 110.0 * (-t * 35.0).exp()) / f64::from(rate.get());
                let value = match track {
                    0 => phase.sin() * (-t * 12.0).exp(),
                    1 => (n * 0.7 + (TAU * 185.0 * t).sin() * 0.3) * (-t * 20.0).exp(),
                    2 => high * (-t * 55.0).exp(),
                    _ => {
                        let pulse = if t < 0.035 {
                            (-((t % 0.012) * 180.0)).exp()
                        } else {
                            (-(t - 0.035) * 25.0).exp() * 0.5
                        };
                        high * pulse
                    }
                };
                let edge = (t * 2000.0).min(1.0) * ((duration - t) * 2000.0).min(1.0);
                (value * edge * 0.8) as f32
            })
            .collect();
        SampleSource {
            name: names[track].into(),
            rate,
            mono,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn wav(
        channels: u16,
        rate: u32,
        bits: u16,
        format: hound::SampleFormat,
        values: &[f32],
    ) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: bits,
                sample_format: format,
            },
        )
        .unwrap();
        for &value in values {
            if format == hound::SampleFormat::Float {
                writer.write_sample(value).unwrap();
            } else {
                writer
                    .write_sample((value * ((1_u64 << (bits - 1)) - 1) as f32) as i32)
                    .unwrap();
            }
        }
        writer.finalize().unwrap();
        cursor.into_inner()
    }

    #[test]
    fn decodes_pcm_depths_and_float_and_downmixes_stereo() {
        for bits in [8, 16, 24, 32] {
            let bytes = wav(
                2,
                48_000,
                bits,
                hound::SampleFormat::Int,
                &[0.5, -0.5, 0.5, 0.5],
            );
            let source = SampleSource::decode(Cursor::new(bytes), "PCM".into()).unwrap();
            assert_eq!(source.mono[0], 0.0);
            assert!((source.mono[1] - 0.5).abs() < 0.01);
        }
        let bytes = wav(1, 48_000, 32, hound::SampleFormat::Float, &[0.25, -0.5]);
        let source = SampleSource::decode(Cursor::new(bytes), "Float".into()).unwrap();
        assert_eq!(source.mono, [0.25, -0.5]);
    }

    #[test]
    fn rejects_corrupt_empty_nonfinite_and_oversized_samples() {
        assert!(SampleSource::decode(Cursor::new(b"bad"), "Bad".into()).is_err());
        for values in [vec![], vec![f32::NAN], vec![0.0; 8000 * 11]] {
            let bytes = wav(1, 8_000, 32, hound::SampleFormat::Float, &values);
            assert!(SampleSource::decode(Cursor::new(bytes), "Bad".into()).is_err());
        }
        let mut bytes = wav(1, 48_000, 16, hound::SampleFormat::Int, &[0.5; 128]);
        bytes.truncate(bytes.len() - 7);
        assert!(SampleSource::decode(Cursor::new(bytes), "Truncated".into()).is_err());
    }

    #[test]
    fn resampling_preserves_duration_and_signal_without_startup_delay() {
        let rate = NonZeroU32::new(44_100).unwrap();
        let source = SampleSource {
            name: "Test".into(),
            rate,
            mono: vec![0.5; 4410],
        };
        let prepared = source.prepare(NonZeroU32::new(48_000).unwrap()).unwrap();
        assert_eq!(prepared.len(), 4800);
        assert!(prepared.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        assert!(prepared[100..4700].iter().all(|v| (v - 0.5).abs() < 0.01));
        assert!(prepared[10].abs() > 0.1, "Resampler delay must be trimmed");
    }

    #[test]
    fn starter_kit_is_deterministic_and_prepared_at_device_rate() {
        let a = starter_kit();
        let b = starter_kit();
        for (a, b) in a.iter().zip(&b) {
            assert_eq!(a.mono, b.mono);
        }
        let bank = SampleBank::prepare(&a, NonZeroU32::new(48_000).unwrap()).unwrap();
        assert!(
            bank.samples
                .iter()
                .all(|s| !s.is_empty() && s.iter().any(|v| v.abs() > 0.01))
        );
    }
}
