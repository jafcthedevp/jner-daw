mod automation_view;
mod effects_view;
mod export_view;
mod piano_view;
mod project_actions;
mod recording_view;
mod shell;
mod synth_view;
mod timeline_view;

use automation_view::AutomationEditor;
use export_view::ExportUi;
use piano_view::PianoEditor;
use project_actions::ProjectUi;
use recording_view::RecordingUi;
use shell::Section;
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use timeline_view::TimelineEditor;

use eframe::egui::{self, Color32, RichText};
use jner_daw::{
    audio::{AudioOutput, OutputDevice, PlaybackSettings, Snapshot, output_devices},
    effects::EffectsState,
    engine::{Command, DEFAULT_GAIN, MAX_GAIN},
    mixer::{MAX_TRACK_GAIN, MixerState},
    samples::{SampleSource, starter_kit},
    sequencer::{Bpm, Pattern, STEPS, TRACK_NAMES, TRACKS},
    synth::SynthState,
};

const ACCENT: Color32 = Color32::from_rgb(255, 78, 117);
const MUTED: Color32 = Color32::from_rgb(165, 164, 171);
const TRACK_COLORS: [Color32; TRACKS] = [
    Color32::from_rgb(226, 135, 156),
    Color32::from_rgb(211, 171, 110),
    Color32::from_rgb(141, 185, 174),
    Color32::from_rgb(167, 157, 212),
];

pub fn run() -> eframe::Result {
    run_with_project(None)
}

pub fn run_with_project(
    opened: Option<(std::path::PathBuf, jner_daw::project::Project)>,
) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Jner DAW · Project")
            .with_inner_size([1320.0, 860.0])
            .with_min_inner_size([940.0, 620.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Jner DAW",
        options,
        Box::new(move |cc| {
            let mut app = App::new(cc);
            if let Some((path, project)) = opened {
                app.apply_project(project).map_err(std::io::Error::other)?;
                app.project.path = Some(path);
            }
            Ok(Box::new(app))
        }),
    )
}

#[derive(Clone, Copy)]
enum LoadDestination {
    PatternTrack(usize),
    TimelineClip { track: usize, tick: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum View {
    Sequencer,
    Mixer,
    Timeline,
    PianoRoll,
    Automation,
}

type LoadResult = Result<(LoadDestination, SampleSource), String>;

struct App {
    devices: Vec<OutputDevice>,
    selected: Option<usize>,
    output: Option<AudioOutput>,
    error: Option<String>,
    notice: Option<String>,
    gain: f32,
    bpm: Bpm,
    pattern: Pattern,
    sources: [Arc<SampleSource>; TRACKS],
    loading: Option<mpsc::Receiver<LoadResult>>,
    mixer: MixerState,
    view: View,
    timeline_playback: bool,
    piano_playback: bool,
    piano: PianoEditor,
    synth: SynthState,
    effects: EffectsState,
    automation: AutomationEditor,
    timeline: TimelineEditor,
    project: ProjectUi,
    file_dialog: rfd::FileDialog,
    recording: RecordingUi,
    export: ExportUi,
    section: Section,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        shell::apply_theme(&cc.egui_ctx);
        let mut app = Self {
            devices: Vec::new(),
            selected: None,
            output: None,
            error: None,
            notice: Some(
                "Kit interno listo. Pulsa Play para escuchar el patrón de ejemplo.".into(),
            ),
            gain: DEFAULT_GAIN,
            bpm: Bpm::default(),
            pattern: Pattern::demo(),
            sources: starter_kit().map(Arc::new),
            loading: None,
            mixer: MixerState::default(),
            view: View::Sequencer,
            timeline_playback: false,
            piano_playback: false,
            piano: PianoEditor::default(),
            synth: SynthState::default(),
            effects: EffectsState::default(),
            automation: AutomationEditor::default(),
            timeline: TimelineEditor::default(),
            project: ProjectUi::default(),
            file_dialog: rfd::FileDialog::new().set_parent(cc),
            recording: RecordingUi::default(),
            export: ExportUi::default(),
            section: Section::default(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.output = None;
        self.error = None;
        self.selected = None;
        match output_devices() {
            Ok(devices) => {
                self.devices = devices;
                self.selected = self
                    .devices
                    .iter()
                    .position(|d| d.is_default)
                    .or_else(|| (!self.devices.is_empty()).then_some(0));
                self.open_selected();
            }
            Err(error) => {
                self.devices.clear();
                self.error = Some(error);
            }
        }
    }

    fn open_selected(&mut self) {
        self.output = None;
        self.error = None;
        let Some(device) = self.selected.and_then(|i| self.devices.get(i)) else {
            self.error =
                Some("No hay salida de audio. Conecta un dispositivo y pulsa Actualizar.".into());
            return;
        };
        let settings = PlaybackSettings {
            bpm: self.bpm,
            mixer: self.mixer,
            synth: self.synth,
            effects: self.effects,
            automation: &self.automation.sequence,
            record_clock: None,
        };
        let opened = if self.piano_playback {
            AudioOutput::open_synth_with_settings(&device.device, &self.piano.sequence, &settings)
        } else if self.timeline_playback {
            AudioOutput::open_timeline_with_settings(
                &device.device,
                &self.timeline.assets,
                &self.timeline.arrangement,
                &settings,
            )
        } else {
            AudioOutput::open_drums_with_settings(
                &device.device,
                &self.sources,
                self.pattern,
                &settings,
            )
        };
        match opened {
            Ok(mut output) => match output.send(Command::SetGain(self.gain)) {
                Ok(()) => self.output = Some(output),
                Err(error) => self.error = Some(error.into()),
            },
            Err(error) => self.error = Some(error),
        }
    }

    fn switch_view(&mut self, view: View) {
        if view != View::Mixer && view != View::Automation {
            let timeline = view == View::Timeline;
            let piano = view == View::PianoRoll;
            if self.timeline_playback != timeline || self.piano_playback != piano {
                self.timeline_playback = timeline;
                self.piano_playback = piano;
                self.mark_dirty();
                self.open_selected();
                self.notice = Some(if piano {
                    "Piano roll: añade notas y pulsa Play para escuchar el sintetizador. Cada pista tiene su propio sonido.".into()
                } else if timeline {
                    "Timeline: añade un sonido de pista, importa un WAV o crea el ejemplo. Play comienza en el principio.".into()
                } else {
                    "Patrón de 16 pasos listo. Pulsa Play para escuchar el loop.".into()
                });
            }
        }
        self.view = view;
    }

    /// Local state is changed only after the command is accepted, or when offline.
    fn send(&mut self, command: Command) -> bool {
        match self
            .output
            .as_mut()
            .map_or(Ok(()), |output| output.send(command))
        {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error.into());
                false
            }
        }
    }

    fn set_pattern(&mut self, pattern: Pattern) {
        if pattern != self.pattern && self.send(Command::SetPattern(pattern)) {
            self.pattern = pattern;
            self.mark_dirty();
        }
    }

    fn set_mixer(&mut self, mixer: MixerState) {
        if mixer != self.mixer && self.send(Command::SetMixer(mixer)) {
            self.mixer = mixer;
            self.mark_dirty();
        }
    }

    fn start_load(&mut self, track: usize) {
        self.start_load_for(LoadDestination::PatternTrack(track));
    }

    fn start_load_for(&mut self, destination: LoadDestination) {
        let track = match destination {
            LoadDestination::PatternTrack(track) | LoadDestination::TimelineClip { track, .. } => {
                track
            }
        };
        let Some(path) = self
            .file_dialog
            .clone()
            .set_title(format!("Cargar WAV · {}", TRACK_NAMES[track]))
            .add_filter("Audio WAV", &["wav"])
            .pick_file()
        else {
            return;
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("wav-loader".into())
            .spawn(move || {
                let result = SampleSource::load(&path).map(|source| (destination, source));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.loading = Some(receiver);
                self.notice = Some("Cargando sample…".into());
            }
            Err(error) => self.error = Some(format!("No se pudo iniciar la carga: {error}")),
        }
    }

    fn finish_load(&mut self) {
        let Some(receiver) = &self.loading else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("La carga del WAV se interrumpió".into()),
        };
        self.loading = None;
        match result {
            Ok((LoadDestination::PatternTrack(track), source)) => {
                // Close the stream before replacing buffers. Never retire sample data in a callback.
                self.output = None;
                self.sources[track] = Arc::new(source);
                self.mark_dirty();
                self.open_selected();
                self.notice = Some(format!(
                    "{} cargado. Pulsa Play para escuchar el patrón.",
                    TRACK_NAMES[track]
                ));
            }
            Ok((LoadDestination::TimelineClip { track, tick }, source)) => {
                self.insert_timeline_clip(track, Arc::new(source), tick);
            }
            Err(error) => {
                self.error = Some(error);
                self.notice = None;
            }
        }
    }

    fn output_controls(&mut self, ui: &mut egui::Ui) {
        shell::card().inner_margin(18.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("SALIDA DE AUDIO").size(11.0).color(MUTED));
            ui.horizontal(|ui| {
                let previous = self.selected;
                egui::ComboBox::from_id_salt("output-device")
                    .width((ui.available_width() - 260.0).max(280.0))
                    .selected_text(self.selected.and_then(|i| self.devices.get(i)).map_or_else(
                        || "Sin dispositivo".into(),
                        |d| {
                            format!(
                                "{}{}",
                                d.name,
                                if d.is_default {
                                    " · predeterminado"
                                } else {
                                    ""
                                }
                            )
                        },
                    ))
                    .show_ui(ui, |ui| {
                        for (index, device) in self.devices.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.selected,
                                Some(index),
                                format!(
                                    "{}{}",
                                    device.name,
                                    if device.is_default {
                                        " · predeterminado"
                                    } else {
                                        ""
                                    }
                                ),
                            );
                        }
                    });
                if previous != self.selected {
                    self.open_selected();
                }
                if ui.button("Actualizar").clicked() {
                    self.refresh();
                }
                if ui.button("Reconectar").clicked() {
                    self.open_selected();
                }
            });
            if let Some(output) = &self.output {
                ui.label(
                    RichText::new(format!(
                        "{} Hz   /   {} canales   /   {}",
                        output.info.sample_rate, output.info.channels, output.info.sample_format
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
            }
        });
    }

    fn mixer_controls(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) {
        shell::card().inner_margin(18.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("MIXER").strong().size(15.0));
                ui.label(
                    RichText::new("4 pistas · salida estéreo")
                        .size(12.0)
                        .color(MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Reiniciar mixer").clicked() {
                        self.set_mixer(MixerState::default());
                    }
                });
            });
            ui.add_space(8.0);
            ui.columns(TRACKS, |columns| {
                for (track, column) in columns.iter_mut().enumerate() {
                    shell::card()
                        .fill(Color32::from_gray(30))
                        .inner_margin(14.0)
                        .show(column, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().slider_width = (ui.available_width() - 90.0).max(35.0);
                            ui.label(
                                RichText::new(if self.piano_playback {
                                    format!("Pista {}", track + 1)
                                } else {
                                    TRACK_NAMES[track].to_owned()
                                })
                                .strong()
                                .size(19.0)
                                .color(TRACK_COLORS[track]),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if self.piano_playback {
                                        self.synth.tracks[track].waveform.name()
                                    } else {
                                        &self.sources[track].name
                                    })
                                    .size(11.0)
                                    .color(MUTED),
                                )
                                .truncate(),
                            );
                            ui.add_space(6.0);
                            let mut next = self.mixer;
                            let channel = &mut next.tracks[track];
                            let mut changed = false;
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(channel.muted, "Mute")
                                    .on_hover_text("Silencia esta pista, incluso si está en Solo")
                                    .clicked()
                                {
                                    channel.muted = !channel.muted;
                                    changed = true;
                                }
                                if ui
                                    .selectable_label(channel.solo, "Solo")
                                    .on_hover_text("Puedes activar Solo en varias pistas")
                                    .clicked()
                                {
                                    channel.solo = !channel.solo;
                                    changed = true;
                                }
                            });
                            ui.add_space(6.0);
                            ui.label(RichText::new("VOLUMEN").size(10.0).color(MUTED));
                            let mut percent = channel.gain * 100.0;
                            if ui
                                .add(
                                    egui::Slider::new(&mut percent, 0.0..=MAX_TRACK_GAIN * 100.0)
                                        .suffix(" %"),
                                )
                                .changed()
                            {
                                channel.gain = percent / 100.0;
                                changed = true;
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new("PANORAMA").size(10.0).color(MUTED));
                            let mut pan = channel.pan * 100.0;
                            if ui
                                .add(egui::Slider::new(&mut pan, -100.0..=100.0).show_value(false))
                                .changed()
                            {
                                channel.pan = pan / 100.0;
                                changed = true;
                            }
                            let pan_label = if pan.abs() < 0.5 {
                                "Centro".into()
                            } else if pan < 0.0 {
                                format!("L {:.0}%", -pan)
                            } else {
                                format!("R {pan:.0}%")
                            };
                            ui.label(RichText::new(pan_label).size(12.0).color(MUTED));
                            if changed {
                                self.set_mixer(next);
                            }
                            ui.add_space(8.0);
                            ui.label(RichText::new("PICO").size(10.0).color(MUTED));
                            ui.add(
                                egui::ProgressBar::new(snapshot.track_peaks[track].min(1.0))
                                    .desired_width(ui.available_width())
                                    .desired_height(8.0)
                                    .corner_radius(3)
                                    .fill(TRACK_COLORS[track]),
                            )
                            .on_hover_text(
                                "Nivel después del volumen y panorama, antes del master",
                            );
                            let channel = self.mixer.tracks[track];
                            let status = if channel.muted {
                                "Silenciada"
                            } else if !self.mixer.audible(track) {
                                "Excluida por Solo"
                            } else if channel.solo {
                                "En Solo"
                            } else {
                                "Activa"
                            };
                            ui.label(RichText::new(status).size(11.0).color(
                                if self.mixer.audible(track) {
                                    TRACK_COLORS[track]
                                } else {
                                    MUTED
                                },
                            ));
                        });
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("MASTER").size(11.0).color(MUTED));
                ui.label("L");
                ui.add(
                    egui::ProgressBar::new(snapshot.stereo_peaks[0])
                        .desired_width(150.0)
                        .desired_height(8.0)
                        .corner_radius(3)
                        .fill(ACCENT),
                );
                ui.label("R");
                ui.add(
                    egui::ProgressBar::new(snapshot.stereo_peaks[1])
                        .desired_width(150.0)
                        .desired_height(8.0)
                        .corner_radius(3)
                        .fill(ACCENT),
                );
            });
            ui.label(
                RichText::new(
                    "Mute tiene prioridad sobre Solo. Las pistas en Solo se escuchan juntas.",
                )
                .size(11.0)
                .color(MUTED),
            );
        });
    }

    fn sequencer(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) {
        shell::card().inner_margin(18.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("PATRÓN 01").strong().size(15.0));
                ui.label(
                    RichText::new("4/4  ·  16 pasos  ·  semicorcheas")
                        .size(12.0)
                        .color(MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Limpiar").clicked() {
                        self.set_pattern(Pattern::default());
                    }
                    if ui.button("Ejemplo").clicked() {
                        self.set_pattern(Pattern::demo());
                    }
                });
            });
            ui.add_space(8.0);
            let cell_width = ((ui.available_width() - 160.0 - 16.0 * 4.0) / 16.0).max(22.0);
            egui::Grid::new("step-sequencer")
                .spacing([4.0, 10.0])
                .min_col_width(0.0)
                .show(ui, |ui| {
                    ui.allocate_ui(egui::vec2(160.0, 18.0), |ui| {
                        ui.label(RichText::new("PISTA / SAMPLE").size(10.0).color(MUTED));
                    });
                    for step in 0..STEPS {
                        let current = snapshot.step == Some(step as u8);
                        ui.add_sized(
                            [cell_width, 18.0],
                            egui::Label::new(
                                RichText::new(format!("{:02}", step + 1))
                                    .size(11.0)
                                    .color(if current { ACCENT } else { MUTED }),
                            ),
                        );
                    }
                    ui.end_row();
                    for (track, &color) in TRACK_COLORS.iter().enumerate() {
                        ui.allocate_ui_with_layout(
                            egui::vec2(160.0, 52.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(TRACK_NAMES[track]).strong().color(color),
                                    );
                                    if ui
                                        .add_enabled(
                                            self.loading.is_none(),
                                            egui::Button::new(
                                                RichText::new("Cargar WAV").size(10.0),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.start_load(track);
                                    }
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&self.sources[track].name)
                                            .size(11.0)
                                            .color(MUTED),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&self.sources[track].name);
                            },
                        );
                        for step in 0..STEPS {
                            let active = self.pattern.active(track, step);
                            let current = snapshot.step == Some(step as u8);
                            let fill = if active {
                                color
                            } else if step % 4 == 0 {
                                Color32::from_gray(45)
                            } else {
                                Color32::from_gray(34)
                            };
                            let stroke = if current {
                                egui::Stroke::new(2.0, Color32::WHITE)
                            } else {
                                egui::Stroke::NONE
                            };
                            if ui
                                .add_sized(
                                    [cell_width, 48.0],
                                    egui::Button::new("").fill(fill).stroke(stroke),
                                )
                                .on_hover_text(format!(
                                    "{} · paso {} · {}",
                                    TRACK_NAMES[track],
                                    step + 1,
                                    if active { "Activado" } else { "Desactivado" }
                                ))
                                .clicked()
                            {
                                let mut pattern = self.pattern;
                                pattern.set(track, step, !active);
                                self.set_pattern(pattern);
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        root.ctx().request_repaint_after(Duration::from_millis(33));
        self.finish_load();
        self.poll_recording();
        self.poll_export();
        let ctx = root.ctx().clone();
        self.project_events(&ctx);
        if self
            .output
            .as_ref()
            .is_some_and(|o| o.snapshot().errors > 0)
        {
            self.output = None;
            self.error = Some(
                "Se interrumpió la salida de audio. Pulsa Reconectar o selecciona otra salida."
                    .into(),
            );
        }
        let snapshot = self
            .output
            .as_ref()
            .map(AudioOutput::snapshot)
            .unwrap_or_default();
        if self.recording.is_active() && !root.ctx().text_edit_focused() {
            if root.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Space)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
            }) {
                self.stop_recording();
            }
        } else if self.editor_enabled() && !root.ctx().text_edit_focused() {
            if root.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                self.send(if snapshot.playing {
                    Command::Stop
                } else {
                    Command::Play
                });
            }
            if root.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.send(Command::Stop);
            }
        }
        self.workspace_ui(root, snapshot);
        self.unsaved_dialog(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn offline_app() -> App {
        App {
            devices: Vec::new(),
            selected: None,
            output: None,
            error: None,
            notice: None,
            gain: DEFAULT_GAIN,
            bpm: Bpm::default(),
            pattern: Pattern::demo(),
            sources: starter_kit().map(Arc::new),
            loading: None,
            mixer: MixerState::default(),
            view: View::Timeline,
            timeline_playback: true,
            piano_playback: false,
            piano: PianoEditor::default(),
            synth: SynthState::default(),
            effects: EffectsState::default(),
            automation: AutomationEditor::default(),
            timeline: TimelineEditor::default(),
            project: ProjectUi::default(),
            file_dialog: rfd::FileDialog::new(),
            recording: RecordingUi::default(),
            export: ExportUi::default(),
            section: Section::default(),
        }
    }

    fn complete_load(app: &mut App, destination: LoadDestination, source: SampleSource) {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.send(Ok((destination, source))).ok().unwrap();
        app.loading = Some(receiver);
        app.finish_load();
    }

    #[test]
    fn importing_a_clip_keeps_pattern_sources_and_uses_the_requested_track_and_tick() {
        let mut app = offline_app();
        let original = Arc::clone(&app.sources[0]);
        let source = starter_kit().into_iter().nth(1).unwrap();
        complete_load(
            &mut app,
            LoadDestination::TimelineClip {
                track: 2,
                tick: 1440,
            },
            source,
        );
        assert!(Arc::ptr_eq(&app.sources[0], &original));
        let clip = app.timeline.arrangement.clips()[0];
        assert_eq!((clip.track, clip.start_tick), (2, 1440));
        assert_eq!(app.timeline.assets[clip.asset].name, "Snare interno");
        assert!(app.loading.is_none());
    }

    #[test]
    fn replacing_a_pattern_sound_does_not_replace_an_existing_clip_source() {
        let mut app = offline_app();
        let original = Arc::clone(&app.sources[0]);
        app.insert_timeline_clip(0, Arc::clone(&original), 0);
        let source = starter_kit().into_iter().nth(1).unwrap();
        complete_load(&mut app, LoadDestination::PatternTrack(0), source);
        assert!(!Arc::ptr_eq(&app.sources[0], &original));
        let clip = app.timeline.arrangement.clips()[0];
        assert!(Arc::ptr_eq(&app.timeline.assets[clip.asset], &original));
        assert_eq!(app.timeline.assets[clip.asset].name, "Kick interno");
        assert_eq!(app.sources[0].name, "Snare interno");
    }
}
