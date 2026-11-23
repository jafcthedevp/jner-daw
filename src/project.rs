//! Versioned, self-contained JSON projects. Disk I/O stays outside the audio engine.
use std::{
    fs::File,
    io::{BufWriter, Read, Write},
    path::Path,
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use crate::{
    automation::{Automation, Lane as AutomationLane},
    effects::EffectsState,
    engine::{DEFAULT_GAIN, MAX_GAIN},
    midi::{MAX_NOTES, Note, NoteSequence},
    mixer::{MixerState, TrackMix},
    samples::{SampleSource, starter_kit},
    sequencer::{Bpm, Pattern, STEPS, TRACKS},
    synth::SynthState,
    timeline::{Arrangement, AudioClip, MAX_ASSETS, MAX_CLIPS, MAX_START_TICK},
};

pub const MAX_PROJECT_BYTES: u64 = 256 * 1024 * 1024;
const VERSION: u32 = 5;
const FORMAT: &str = "jner-daw-project";

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectView {
    #[default]
    Sequencer,
    Mixer,
    Timeline,
    PianoRoll,
    Automation,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub view: ProjectView,
    pub timeline_playback: bool,
    #[serde(default)]
    pub piano_playback: bool,
    pub cursor_tick: u64,
    pub insert_track: usize,
    pub zoom: f32,
    pub snap: bool,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            view: ProjectView::Sequencer,
            timeline_playback: false,
            piano_playback: false,
            cursor_tick: 0,
            insert_track: 0,
            zoom: 64.0,
            snap: true,
        }
    }
}

#[derive(Clone)]
pub struct Project {
    pub bpm: Bpm,
    pub master_gain: f32,
    pub pattern: Pattern,
    pub sources: [Arc<SampleSource>; TRACKS],
    pub mixer: MixerState,
    pub timeline_sources: Vec<Arc<SampleSource>>,
    pub arrangement: Arrangement,
    pub notes: NoteSequence,
    pub synth: SynthState,
    pub effects: EffectsState,
    pub automation: Automation,
    pub workspace: Workspace,
}

impl Project {
    pub fn empty() -> Self {
        Self {
            bpm: Bpm::default(),
            master_gain: DEFAULT_GAIN,
            pattern: Pattern::default(),
            sources: starter_kit().map(Arc::new),
            mixer: MixerState::default(),
            timeline_sources: Vec::new(),
            arrangement: Arrangement::default(),
            notes: NoteSequence::default(),
            synth: SynthState::default(),
            effects: EffectsState::default(),
            automation: Automation::default(),
            workspace: Workspace::default(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.synth.validate()?;
        self.effects.validate()?;
        self.automation.validate()?;
        if !self.master_gain.is_finite()
            || !(0.0..=MAX_GAIN).contains(&self.master_gain)
            || !self.mixer.is_valid()
        {
            return Err("Ganancias o panorama del proyecto no válidos".into());
        }
        let w = self.workspace;
        if !w.zoom.is_finite()
            || !(24.0..=160.0).contains(&w.zoom)
            || w.cursor_tick > MAX_START_TICK
            || w.insert_track >= TRACKS
            || (w.timeline_playback && w.piano_playback)
            || (w.view == ProjectView::Sequencer && (w.timeline_playback || w.piano_playback))
            || (w.view == ProjectView::Timeline && !w.timeline_playback)
            || (w.view == ProjectView::PianoRoll && !w.piano_playback)
        {
            return Err("Vista o modo de reproducción del proyecto no válido".into());
        }
        if self.timeline_sources.len() > MAX_ASSETS || self.arrangement.clips().len() > MAX_CLIPS {
            return Err("El proyecto excede los límites de fuentes o clips".into());
        }
        for source in self.sources.iter().chain(&self.timeline_sources) {
            if source.name.len() > 1024 {
                return Err("Nombre de fuente demasiado largo".into());
            }
        }
        // Rebuild to check all metadata, including duplicate IDs, before opening any stream.
        Arrangement::from_clips(self.arrangement.clips().to_vec())?;
        NoteSequence::from_notes(self.notes.notes().to_vec())?;
        for clip in self.arrangement.clips() {
            let source = self
                .timeline_sources
                .get(clip.asset)
                .ok_or("Falta una fuente de clip")?;
            if clip.out_frame > source.frame_count() {
                return Err("El recorte excede la fuente del clip".into());
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectDocument {
    format: String,
    version: u32,
    bpm: u16,
    master_gain: f32,
    pattern: [[bool; STEPS]; TRACKS],
    tracks: [TrackDocument; TRACKS],
    sources: Vec<SourceDocument>,
    timeline_sources: Vec<usize>,
    clips: Vec<ClipDocument>,
    #[serde(default)]
    notes: Vec<Note>,
    #[serde(default)]
    synth: Option<SynthState>,
    #[serde(default)]
    effects: Option<EffectsState>,
    #[serde(default)]
    automation: Option<Vec<AutomationLane>>,
    workspace: Workspace,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackDocument {
    source: usize,
    gain: f32,
    pan: f32,
    muted: bool,
    solo: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceDocument {
    name: String,
    sample_rate: u32,
    frames: u32,
    encoding: String,
    data: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipDocument {
    id: u32,
    track: usize,
    source: usize,
    start_tick: u64,
    in_frame: u32,
    out_frame: u32,
}

fn intern(sources: &mut Vec<Arc<SampleSource>>, source: &Arc<SampleSource>) -> usize {
    if let Some(index) = sources.iter().position(|s| Arc::ptr_eq(s, source)) {
        index
    } else {
        sources.push(Arc::clone(source));
        sources.len() - 1
    }
}

impl ProjectDocument {
    fn capture(project: &Project) -> Result<Self, String> {
        project.validate()?;
        let mut sources = Vec::new();
        let tracks = std::array::from_fn(|track| {
            let mix = project.mixer.tracks[track];
            TrackDocument {
                source: intern(&mut sources, &project.sources[track]),
                gain: mix.gain,
                pan: mix.pan,
                muted: mix.muted,
                solo: mix.solo,
            }
        });
        let timeline_sources = project
            .timeline_sources
            .iter()
            .map(|s| intern(&mut sources, s))
            .collect();
        let sources = sources
            .iter()
            .map(|source| {
                let bytes: Vec<u8> = source
                    .pcm()
                    .iter()
                    .flat_map(|sample| sample.to_le_bytes())
                    .collect();
                SourceDocument {
                    name: source.name.clone(),
                    sample_rate: source.sample_rate(),
                    frames: source.frame_count(),
                    encoding: "mono_f32le_base64".into(),
                    data: STANDARD.encode(bytes),
                }
            })
            .collect();
        let clips = project
            .arrangement
            .clips()
            .iter()
            .map(|c| ClipDocument {
                id: c.id,
                track: c.track,
                source: c.asset,
                start_tick: c.start_tick,
                in_frame: c.in_frame,
                out_frame: c.out_frame,
            })
            .collect();
        Ok(Self {
            format: FORMAT.into(),
            version: VERSION,
            bpm: project.bpm.get(),
            master_gain: project.master_gain,
            pattern: std::array::from_fn(|track| {
                std::array::from_fn(|step| project.pattern.active(track, step))
            }),
            tracks,
            sources,
            timeline_sources,
            clips,
            notes: project.notes.notes().to_vec(),
            synth: Some(project.synth),
            effects: Some(project.effects),
            automation: Some(project.automation.lanes().to_vec()),
            workspace: project.workspace,
        })
    }

    fn restore(self) -> Result<Project, String> {
        if self.format != FORMAT {
            return Err("El archivo no es un proyecto Jner DAW".into());
        }
        if !(1..=VERSION).contains(&self.version) {
            return Err(format!(
                "Versión de proyecto no compatible: {}",
                self.version
            ));
        }
        if self.version < 3 && self.synth.is_some() {
            return Err("La versión anterior no admite parámetros de synth".into());
        }
        if self.version >= 3 && self.synth.is_none() {
            return Err("Faltan los parámetros de synth del proyecto".into());
        }
        if self.version < 4 && self.effects.is_some() {
            return Err("La versión anterior no admite efectos".into());
        }
        if self.version >= 4 && self.effects.is_none() {
            return Err("Faltan los parámetros de efectos del proyecto".into());
        }
        if self.version < 5
            && (self.automation.is_some() || self.workspace.view == ProjectView::Automation)
        {
            return Err("La versión anterior no admite automatización".into());
        }
        if self.version == 5 && self.automation.is_none() {
            return Err("Faltan los carriles de automatización".into());
        }
        if self.version == 1
            && (!self.notes.is_empty()
                || self.workspace.piano_playback
                || self.workspace.view == ProjectView::PianoRoll)
        {
            return Err("El formato v1 no admite notas de piano roll".into());
        }
        if self.sources.len() > TRACKS + MAX_ASSETS
            || self.sources.is_empty()
            || self.timeline_sources.len() > MAX_ASSETS
            || self.clips.len() > MAX_CLIPS
            || self.notes.len() > MAX_NOTES
        {
            return Err("El proyecto excede los límites de fuentes o clips".into());
        }
        let bpm = Bpm::new(self.bpm).map_err(str::to_owned)?;
        let mut sources = Vec::with_capacity(self.sources.len());
        for source in self.sources {
            if !(8_000..=192_000).contains(&source.sample_rate)
                || source.frames == 0
                || source.frames > source.sample_rate * 10
                || source.name.len() > 1024
                || source.encoding != "mono_f32le_base64"
            {
                return Err("Metadatos de fuente no válidos".into());
            }
            let bytes = source.frames as usize * 4;
            if source.data.len() != bytes.div_ceil(3) * 4 {
                return Err("La longitud de audio no coincide con la fuente".into());
            }
            let raw = STANDARD
                .decode(source.data)
                .map_err(|e| format!("Audio del proyecto no válido: {e}"))?;
            if raw.len() != bytes {
                return Err("Longitud PCM no válida".into());
            }
            let pcm = raw
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            sources.push(Arc::new(SampleSource::from_pcm(
                source.name,
                source.sample_rate,
                pcm,
            )?));
        }
        let get_source = |index: usize| {
            sources
                .get(index)
                .cloned()
                .ok_or_else(|| "Referencia de fuente no válida".to_owned())
        };
        let pattern_sources = [
            get_source(self.tracks[0].source)?,
            get_source(self.tracks[1].source)?,
            get_source(self.tracks[2].source)?,
            get_source(self.tracks[3].source)?,
        ];
        let timeline_sources = self
            .timeline_sources
            .into_iter()
            .map(get_source)
            .collect::<Result<Vec<_>, _>>()?;
        let clips = self
            .clips
            .into_iter()
            .map(|c| AudioClip {
                id: c.id,
                track: c.track,
                asset: c.source,
                start_tick: c.start_tick,
                in_frame: c.in_frame,
                out_frame: c.out_frame,
            })
            .collect();
        let mut pattern = Pattern::default();
        for (track, steps) in self.pattern.into_iter().enumerate() {
            for (step, active) in steps.into_iter().enumerate() {
                pattern.set(track, step, active);
            }
        }
        let mixer = MixerState {
            tracks: self.tracks.map(|t| TrackMix {
                gain: t.gain,
                pan: t.pan,
                muted: t.muted,
                solo: t.solo,
            }),
        };
        let project = Project {
            bpm,
            master_gain: self.master_gain,
            pattern,
            sources: pattern_sources,
            mixer,
            timeline_sources,
            arrangement: Arrangement::from_clips(clips)?,
            notes: NoteSequence::from_notes(self.notes)?,
            synth: self.synth.unwrap_or_default(),
            effects: self.effects.unwrap_or_default(),
            automation: Automation::from_lanes(self.automation.unwrap_or_default())?,
            workspace: self.workspace,
        };
        project.validate()?;
        Ok(project)
    }
}

pub fn save(path: &Path, project: &Project) -> Result<(), String> {
    // Encode and validate before creating a temporary file; never truncate the destination.
    let document = ProjectDocument::capture(project)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("No se pudo preparar el guardado: {e}"))?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        serde_json::to_writer_pretty(&mut writer, &document)
            .map_err(|e| format!("No se pudo escribir el proyecto: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("No se pudo completar la escritura: {e}"))?;
    }
    let file = temporary.as_file();
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_PROJECT_BYTES {
        return Err("El proyecto supera los 256 MiB".into());
    }
    file.sync_all()
        .map_err(|e| format!("No se pudo sincronizar el proyecto: {e}"))?;
    temporary
        .persist(path)
        .map_err(|e| format!("No se pudo reemplazar el archivo de proyecto: {}", e.error))?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Project, String> {
    let file = File::open(path).map_err(|e| format!("No se pudo abrir el proyecto: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_PROJECT_BYTES {
        return Err("El proyecto supera los 256 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_PROJECT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("No se pudo leer el proyecto: {e}"))?;
    if bytes.len() as u64 > MAX_PROJECT_BYTES {
        return Err("El proyecto supera los 256 MiB".into());
    }
    let document: ProjectDocument =
        serde_json::from_slice(&bytes).map_err(|e| format!("JSON de proyecto no válido: {e}"))?;
    document.restore()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated() -> Project {
        let mut project = Project::empty();
        project.bpm = Bpm::new(137).unwrap();
        project.master_gain = 0.13;
        project.pattern = Pattern::demo();
        project.pattern.set(3, 15, true);
        project.mixer.tracks[0].gain = 0.6;
        project.mixer.tracks[1].pan = -0.7;
        project.mixer.tracks[2].muted = true;
        project.mixer.tracks[3].solo = true;
        project
            .timeline_sources
            .push(Arc::clone(&project.sources[0]));
        let id = project
            .arrangement
            .add(2, 0, 1440, project.sources[0].frame_count())
            .unwrap();
        project
            .arrangement
            .replace(AudioClip {
                in_frame: 100,
                out_frame: 9000,
                ..project.arrangement.get(id).unwrap()
            })
            .unwrap();
        project.workspace = Workspace {
            view: ProjectView::Timeline,
            timeline_playback: true,
            piano_playback: false,
            cursor_tick: 1200,
            insert_track: 2,
            zoom: 112.0,
            snap: false,
        };
        project.notes.add(2, 64, 91, 1200, 480).unwrap();
        project.synth.tracks[2].waveform = crate::synth::Waveform::Triangle;
        project.synth.tracks[2].cutoff_hz = 1800.0;
        project.effects = EffectsState {
            delay_enabled: true,
            delay_ms: 375.0,
            feedback: 0.6,
            distortion_enabled: true,
            drive: 7.0,
            ..Default::default()
        };
        project
            .automation
            .add(crate::automation::Target::TrackGain(2), 0, 0.2)
            .unwrap();
        project
            .automation
            .add(crate::automation::Target::TrackGain(2), 3840, 1.8)
            .unwrap();
        project
    }

    #[test]
    fn complete_round_trip_preserves_sources_sharing_clips_and_parameters() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jner");
        let original = populated();
        save(&path, &original).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.bpm, original.bpm);
        assert_eq!(loaded.master_gain, original.master_gain);
        assert_eq!(loaded.pattern, original.pattern);
        assert_eq!(loaded.mixer, original.mixer);
        assert_eq!(loaded.workspace, original.workspace);
        assert_eq!(loaded.arrangement.clips(), original.arrangement.clips());
        assert_eq!(loaded.notes, original.notes);
        assert_eq!(loaded.synth, original.synth);
        assert_eq!(loaded.effects, original.effects);
        assert_eq!(loaded.automation, original.automation);
        assert!(Arc::ptr_eq(&loaded.sources[0], &loaded.timeline_sources[0]));
        for (a, b) in original.sources.iter().zip(&loaded.sources) {
            assert_eq!(a.sample_rate(), b.sample_rate());
            assert_eq!(a.name, b.name);
            assert!(
                a.pcm()
                    .iter()
                    .zip(b.pcm())
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
        }
    }

    #[test]
    fn file_is_portable_after_the_original_wav_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let wav = directory.path().join("own.wav");
        let mut writer = hound::WavWriter::create(
            &wav,
            hound::WavSpec {
                channels: 1,
                sample_rate: 44_100,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for value in [0.25_f32, -0.5, -0.0, 0.75] {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
        let mut project = Project::empty();
        project.sources[0] = Arc::new(SampleSource::load(&wav).unwrap());
        let path = directory.path().join("portable.jner");
        save(&path, &project).unwrap();
        std::fs::remove_file(&wav).unwrap();
        let other = tempfile::tempdir().unwrap();
        let copied = other.path().join("moved.jner");
        std::fs::copy(&path, &copied).unwrap();
        let loaded = load(&copied).unwrap();
        assert_eq!(loaded.sources[0].sample_rate(), 44_100);
        assert_eq!(
            loaded.sources[0]
                .pcm()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            [0.25_f32, -0.5, -0.0, 0.75].map(f32::to_bits)
        );
    }

    #[test]
    fn corrupt_future_and_invalid_metadata_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.jner");
        let bytes = serde_json::to_vec(&ProjectDocument::capture(&populated()).unwrap()).unwrap();
        let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (pointer, replacement) in [
            ("/version", serde_json::json!(6)),
            ("/bpm", serde_json::json!(0)),
            ("/tracks/0/gain", serde_json::json!(99)),
            ("/tracks/0/source", serde_json::json!(99)),
            ("/clips/0/out_frame", serde_json::json!(99_999_999)),
            ("/workspace/zoom", serde_json::json!(-1)),
            ("/sources/0/data", serde_json::json!("corrupt")),
            ("/sources/0/sample_rate", serde_json::json!(0)),
        ] {
            let mut document = original.clone();
            *document.pointer_mut(pointer).unwrap() = replacement;
            std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
            assert!(load(&path).is_err(), "Accepted {pointer}");
        }
        std::fs::write(&path, b"{truncated").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn invalid_save_and_failed_replacement_leave_the_previous_file_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("safe.jner");
        let mut project = populated();
        save(&path, &project).unwrap();
        let before = std::fs::read(&path).unwrap();
        project.master_gain = f32::NAN;
        assert!(save(&path, &project).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        project.master_gain = 0.2;
        assert!(save(directory.path(), &project).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        save(&path, &project).unwrap();
        assert_eq!(load(&path).unwrap().master_gain, 0.2);
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            1,
            "Temporary files must be cleaned up"
        );
    }

    #[test]
    fn nonfinite_audio_duplicate_clip_ids_and_oversized_files_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.jner");
        let mut document = ProjectDocument::capture(&Project::empty()).unwrap();
        document.sources[0].frames = 1;
        document.sources[0].data = STANDARD.encode(f32::NAN.to_le_bytes());
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(load(&path).is_err());
        let mut duplicate =
            serde_json::to_value(ProjectDocument::capture(&populated()).unwrap()).unwrap();
        let clip = duplicate["clips"][0].clone();
        duplicate["clips"].as_array_mut().unwrap().push(clip);
        std::fs::write(&path, serde_json::to_vec(&duplicate).unwrap()).unwrap();
        assert!(load(&path).is_err());
        File::create(&path)
            .unwrap()
            .set_len(MAX_PROJECT_BYTES + 1)
            .unwrap();
        assert!(matches!(load(&path), Err(error) if error.contains("256")));
    }

    #[test]
    fn version_one_migrates_with_empty_notes_and_new_versions_preserve_piano_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.jner");
        let mut old =
            serde_json::to_value(ProjectDocument::capture(&Project::empty()).unwrap()).unwrap();
        old["version"] = serde_json::json!(1);
        old.as_object_mut().unwrap().remove("notes");
        old.as_object_mut().unwrap().remove("synth");
        old.as_object_mut().unwrap().remove("effects");
        old.as_object_mut().unwrap().remove("automation");
        old["workspace"]
            .as_object_mut()
            .unwrap()
            .remove("piano_playback");
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let migrated = load(&path).unwrap();
        assert!(migrated.notes.notes().is_empty());
        assert!(!migrated.workspace.piano_playback);
        let mut new = populated();
        new.workspace.view = ProjectView::PianoRoll;
        new.workspace.timeline_playback = false;
        new.workspace.piano_playback = true;
        save(&path, &new).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.workspace, new.workspace);
        assert_eq!(loaded.notes, new.notes);
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(document["version"], 5);
        for (pointer, value) in [
            ("/notes/0/pitch", serde_json::json!(128)),
            ("/notes/0/velocity", serde_json::json!(0)),
            ("/notes/0/duration_tick", serde_json::json!(0)),
            ("/version", serde_json::json!(1)),
            ("/workspace/timeline_playback", serde_json::json!(true)),
        ] {
            let mut invalid = document.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(load(&path).is_err(), "Accepted {pointer}");
        }
    }

    #[test]
    fn version_two_preserves_notes_with_default_synth_and_invalid_patches_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.jner");
        let original = populated();
        let mut document =
            serde_json::to_value(ProjectDocument::capture(&original).unwrap()).unwrap();
        document["version"] = serde_json::json!(2);
        document.as_object_mut().unwrap().remove("synth");
        document.as_object_mut().unwrap().remove("effects");
        document.as_object_mut().unwrap().remove("automation");
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let migrated = load(&path).unwrap();
        assert_eq!(migrated.notes, original.notes);
        assert_eq!(migrated.synth, SynthState::default());
        save(&path, &original).unwrap();
        let valid: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for (pointer, value) in [
            ("/synth/tracks/0/cutoff_hz", serde_json::json!(0)),
            ("/synth/tracks/0/attack_ms", serde_json::json!(-1)),
            ("/synth/tracks/0/sustain", serde_json::json!(2)),
            ("/synth/tracks/0/waveform", serde_json::json!("unknown")),
            ("/synth", serde_json::Value::Null),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(load(&path).is_err());
        }
    }

    #[test]
    fn version_three_migrates_with_bypassed_effects_and_v4_rejects_invalid_effects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.jner");
        let original = populated();
        let mut legacy =
            serde_json::to_value(ProjectDocument::capture(&original).unwrap()).unwrap();
        legacy["version"] = serde_json::json!(3);
        legacy.as_object_mut().unwrap().remove("effects");
        legacy.as_object_mut().unwrap().remove("automation");
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.effects, EffectsState::default());
        assert_eq!(loaded.synth, original.synth);
        assert_eq!(loaded.notes, original.notes);
        save(&path, &original).unwrap();
        let valid: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for (pointer, value) in [
            ("/effects/feedback", serde_json::json!(1)),
            ("/effects/drive", serde_json::json!(0)),
            ("/effects/delay_ms", serde_json::json!(0)),
            ("/effects/distortion_mix", serde_json::json!(2)),
            ("/effects", serde_json::Value::Null),
            ("/version", serde_json::json!(3)),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(load(&path).is_err());
        }
    }

    #[test]
    fn version_four_migrates_without_curves_and_invalid_automation_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.jner");
        let original = populated();
        let mut old = serde_json::to_value(ProjectDocument::capture(&original).unwrap()).unwrap();
        old["version"] = serde_json::json!(4);
        old.as_object_mut().unwrap().remove("automation");
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let loaded = load(&path).unwrap();
        assert!(loaded.automation.lanes().is_empty());
        assert_eq!(loaded.effects, original.effects);
        assert_eq!(loaded.notes, original.notes);
        save(&path, &original).unwrap();
        let valid: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for (pointer, value) in [
            ("/automation/0/points/0/value", serde_json::json!(3)),
            ("/automation/0/points/0/tick", serde_json::json!(999999)),
            ("/automation/0/target", serde_json::json!({"track_gain":4})),
            ("/automation", serde_json::Value::Null),
            ("/version", serde_json::json!(4)),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(load(&path).is_err());
        }
    }
}
