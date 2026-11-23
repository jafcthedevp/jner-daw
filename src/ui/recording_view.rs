use super::*;
use jner_daw::{
    recording::{self, InputDevice, RecordingSession},
    timeline::{MAX_ASSETS, MAX_CLIPS},
};
use std::time::Instant;

#[derive(Default)]
pub(super) struct RecordingUi {
    inputs: Vec<InputDevice>,
    selected: Option<usize>,
    track: usize,
    initialized: bool,
    number: u32,
    active: Option<ActiveTake>,
}
struct ActiveTake {
    session: RecordingSession,
    track: usize,
    began: Instant,
    stopping: Option<Instant>,
}
impl RecordingUi {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }
}

impl App {
    fn refresh_inputs(&mut self) {
        match recording::input_devices() {
            Ok(inputs) => {
                self.recording.selected = inputs
                    .iter()
                    .position(|d| d.is_default)
                    .or(if inputs.is_empty() { None } else { Some(0) });
                self.recording.inputs = inputs;
            }
            Err(error) => self.error = Some(error),
        }
        self.recording.initialized = true;
    }
    fn start_recording(&mut self) {
        if self.project_busy() || !self.editor_enabled() {
            return;
        }
        if self.timeline.arrangement.clips().len() >= MAX_CLIPS
            || self.timeline.assets.len() >= MAX_ASSETS
            || self.recording.track >= TRACKS
        {
            self.error = Some("Libera un clip/fuente del timeline antes de grabar".into());
            return;
        }
        if self.selected.and_then(|i| self.devices.get(i)).is_none() {
            self.error = Some("Selecciona una salida de audio".into());
            return;
        }
        let Some(input) = self
            .recording
            .selected
            .and_then(|i| self.recording.inputs.get(i))
        else {
            self.error = Some("Selecciona una entrada de audio".into());
            return;
        };
        let name = format!("Toma {}", self.recording.number.saturating_add(1));
        let session = match RecordingSession::open(&input.device, name) {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.output = None;
        let result = if let Some(device) = self.selected.and_then(|i| self.devices.get(i)) {
            let settings = PlaybackSettings {
                bpm: self.bpm,
                mixer: self.mixer,
                synth: self.synth,
                effects: self.effects,
                automation: &self.automation.sequence,
                record_clock: Some(Arc::clone(&session.clock)),
            };
            AudioOutput::open_timeline_with_settings(
                &device.device,
                &self.timeline.assets,
                &self.timeline.arrangement,
                &settings,
            )
        } else {
            Err("Selecciona una salida de audio".into())
        };
        let mut output = match result {
            Ok(o) => o,
            Err(e) => {
                drop(session);
                self.open_selected();
                self.error = Some(e);
                return;
            }
        };
        if let Err(e) = output
            .send(Command::SetGain(self.gain))
            .and_then(|()| output.send(Command::Play))
        {
            drop(output);
            drop(session);
            self.open_selected();
            self.error = Some(e.into());
            return;
        }
        if !self.timeline_playback {
            self.mark_dirty();
        }
        self.timeline_playback = true;
        self.piano_playback = false;
        self.view = View::Timeline;
        self.output = Some(output);
        self.error = None;
        self.notice = Some(
            "Grabando desde el inicio del timeline. Pulsa Detener toma para crear el clip.".into(),
        );
        self.recording.active = Some(ActiveTake {
            session,
            track: self.recording.track,
            began: Instant::now(),
            stopping: None,
        });
    }
    pub(super) fn stop_recording(&mut self) {
        if self
            .recording
            .active
            .as_ref()
            .is_none_or(|t| t.stopping.is_some())
        {
            return;
        }
        if self.output.is_none() {
            self.cancel_recording();
            self.error = Some("Se interrumpió la salida durante la grabación".into());
            return;
        }
        if self.send(Command::Stop)
            && let Some(take) = &mut self.recording.active
        {
            take.stopping = Some(Instant::now());
        }
    }
    fn cancel_recording(&mut self) {
        self.output = None;
        self.recording.active.take();
        self.open_selected();
        self.notice = Some("Toma cancelada. El arreglo anterior se conserva.".into());
    }
    pub(super) fn poll_recording(&mut self) {
        let Some(active) = &self.recording.active else {
            return;
        };
        let result = active.session.poll();
        let timeout = active
            .stopping
            .is_some_and(|t| t.elapsed() > Duration::from_secs(3))
            || (!active.session.clock.started() && active.began.elapsed() > Duration::from_secs(3))
            || active.began.elapsed() > Duration::from_secs(15);
        let output_failed = self.output.as_ref().is_none_or(|o| o.snapshot().errors > 0);
        let result = match result {
            _ if output_failed => Err("Se interrumpió la salida; la toma no se añadió".into()),
            Some(r) => r,
            None if timeout => Err(
                "La captura no recibió callbacks válidos o perdió la salida; la toma no se añadió"
                    .into(),
            ),
            None => return,
        };
        let Some(active) = self.recording.active.take() else {
            return;
        };
        let track = active.track;
        let estimated = active.session.clock.uses_estimated_timing();
        self.output = None;
        drop(active);
        match result {
            Ok(source) => {
                self.apply_recorded_take(track, source);
                if estimated && let Some(notice) = &mut self.notice {
                    notice.push_str(
                        " Alineación estimada: el dispositivo no ofreció timestamps utilizables.",
                    );
                }
            }
            Err(error) => {
                self.open_selected();
                self.error = Some(error);
                self.notice = None;
            }
        }
    }
    fn apply_recorded_take(&mut self, track: usize, source: SampleSource) {
        let seconds = source.frame_count() as f64 / f64::from(source.sample_rate());
        self.timeline_playback = true;
        self.piano_playback = false;
        self.view = View::Timeline;
        let previous_count = self.timeline.arrangement.clips().len();
        self.insert_timeline_clip(track, Arc::new(source), 0);
        if self.timeline.arrangement.clips().len() == previous_count + 1 {
            self.section = Section::Editor;
            self.recording.number = self.recording.number.saturating_add(1);
            self.notice = Some(format!(
                "Toma de {seconds:.3} s añadida al inicio de Pista {}. Pulsa Play para escucharla.",
                track + 1
            ));
        }
    }
    pub(super) fn recording_controls(&mut self, ui: &mut egui::Ui) {
        if let Some(take) = &self.recording.active {
            let seconds = take.session.clock.elapsed_seconds(take.session.sample_rate);
            let peak = take.session.clock.peak();
            let rate = take.session.sample_rate;
            let channels = take.session.channels;
            let can_stop = take.stopping.is_none();
            let status = if take.stopping.is_some() {
                "FINALIZANDO"
            } else {
                "GRABANDO"
            };
            ui.group(|ui| {
                let text = format!("{status} · {seconds:.2} / 10 s · {rate} Hz · {channels} canales → mono · pico {peak:.3}");
                ui.label(RichText::new(text).color(ACCENT));
                ui.horizontal(|ui| {
                    if ui.add_enabled(can_stop, egui::Button::new("Detener toma")).clicked() {
                        self.stop_recording();
                    }
                    if ui.button("Cancelar toma").clicked() {
                        self.cancel_recording();
                    }
                });
            });
            return;
        }
        egui::CollapsingHeader::new("Grabación de entrada · toma mono de hasta 10 s")
            .id_salt("recording-panel")
            .default_open(true)
            .show(ui, |ui| {
                if !self.recording.initialized {
                    self.refresh_inputs();
                }
                ui.add_enabled_ui(self.editor_enabled() && !self.project_busy(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let selected = self.recording.selected
                            .and_then(|i| self.recording.inputs.get(i))
                            .map_or("Sin entrada", |d| d.name.as_str());
                        egui::ComboBox::from_id_salt("recording-input")
                            .width(310.0)
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                for (i, input) in self.recording.inputs.iter().enumerate() {
                                    let suffix = if input.is_default { " · predeterminada" } else { "" };
                                    ui.selectable_value(&mut self.recording.selected, Some(i), format!("{}{suffix}", input.name));
                                }
                            });
                        if ui.button("Actualizar entradas").clicked() {
                            self.refresh_inputs();
                        }
                        egui::ComboBox::from_id_salt("recording-track")
                            .selected_text(format!("Pista {}", self.recording.track + 1))
                            .show_ui(ui, |ui| {
                                for track in 0..TRACKS {
                                    ui.selectable_value(&mut self.recording.track, track, format!("Pista {}", track + 1));
                                }
                            });
                        let ready = self.recording.selected.is_some() && self.selected.is_some();
                        if ui.add_enabled(ready, egui::Button::new("Grabar desde inicio")).clicked() {
                            self.start_recording();
                        }
                    });
                    let text = "La captura empieza al pulsar Grabar. Reproduce el timeline desde beat 1, conserva sus clips y añade una toma seca. Sin monitorización de entrada.";
                    ui.label(RichText::new(text).size(11.0).color(MUTED));
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_take_creates_an_independent_clip_at_transport_origin_and_survives_save() {
        let mut app = super::super::tests::offline_app();
        let source = Arc::clone(&app.sources[0]);
        app.insert_timeline_clip(1, Arc::clone(&source), 960);
        let previous = app.timeline.arrangement.clips()[0];
        app.section = Section::Audio;
        app.apply_recorded_take(2, recording::synthetic_take().unwrap());
        assert!(app.section == Section::Editor);
        assert_eq!(app.timeline.arrangement.clips().len(), 2);
        assert_eq!(app.timeline.arrangement.clips()[0], previous);
        let take = app.timeline.arrangement.clips()[1];
        assert_eq!((take.track, take.start_tick), (2, 0));
        assert!(Arc::ptr_eq(&app.sources[0], &source));
        assert!(app.project.dirty);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("take.jner");
        jner_daw::project::save(&path, &app.capture_project()).unwrap();
        let loaded = jner_daw::project::load(&path).unwrap();
        assert_eq!(loaded.arrangement.clips(), app.timeline.arrangement.clips());
        assert_eq!(loaded.timeline_sources[take.asset].frame_count(), 11_025);
    }
}
