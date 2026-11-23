use super::*;
use jner_daw::export::{self, ExportControl, ExportOptions, MAX_BEATS, RenderMode};
use std::path::PathBuf;

type ExportResult = Result<(PathBuf, u64, u32), String>;
#[derive(Default)]
pub(super) struct ExportUi {
    options: ExportOptions,
    last_mode: Option<RenderMode>,
    job: Option<mpsc::Receiver<ExportResult>>,
    control: Option<Arc<ExportControl>>,
}
impl ExportUi {
    pub(super) fn is_active(&self) -> bool {
        self.job.is_some()
    }
}
impl App {
    pub(super) fn poll_export(&mut self) {
        let Some(job) = &self.export.job else {
            return;
        };
        let result = match job.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("El worker de exportación se interrumpió".into())
            }
        };
        self.export.job = None;
        self.export.control = None;
        self.open_selected();
        match result {
            Ok((path, frames, rate)) => {
                self.notice = Some(format!(
                    "WAV exportado: {} · {:.3} s · {rate} Hz · estéreo PCM 16 bits",
                    path.display(),
                    frames as f64 / f64::from(rate)
                ));
            }
            Err(error) => {
                self.error = Some(error);
                self.notice = None;
            }
        }
    }
    fn start_export(&mut self) {
        let Some(mut path) = self
            .file_dialog
            .clone()
            .set_title("Exportar WAV estéreo")
            .add_filter("Audio WAV", &["wav"])
            .set_file_name("Jner export.wav")
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("wav");
        }
        self.export_to(path);
    }
    fn export_to(&mut self, path: PathBuf) {
        if self.project_busy() || !self.editor_enabled() {
            return;
        }
        let snapshot = self.capture_project();
        let options = ExportOptions {
            mode: RenderMode::from_project(&snapshot),
            ..self.export.options
        };
        if let Err(e) = snapshot
            .validate()
            .and_then(|()| options.frames(&snapshot).map(|_| ()))
        {
            self.error = Some(e);
            return;
        }
        let control = Arc::new(ExportControl::default());
        let worker_control = Arc::clone(&control);
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("wav-exporter".into())
            .spawn(move || {
                let result = export::export_wav(&path, &snapshot, options, &worker_control)
                    .map(|frames| (path, frames, options.sample_rate));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.output = None;
                self.export.job = Some(receiver);
                self.export.control = Some(control);
                self.error = None;
                self.notice = Some("Exportando WAV offline…".into());
            }
            Err(e) => self.error = Some(format!("No se pudo iniciar la exportación: {e}")),
        }
    }
    pub(super) fn export_controls(&mut self, ui: &mut egui::Ui) {
        if self.export.is_active() {
            ui.group(|ui| {
                if let Some(control) = &self.export.control {
                    let (done, total) = control.progress();
                    let fraction = if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .show_percentage()
                            .text("Exportando WAV…"),
                    );
                    if ui.button("Cancelar exportación").clicked() {
                        control.cancel();
                    }
                }
            });
            return;
        }
        egui::CollapsingHeader::new("Exportar WAV · estéreo PCM 16 bits").id_salt("export-panel").default_open(true).show(ui, |ui| {
            ui.add_enabled_ui(self.editor_enabled() && !self.project_busy(), |ui| {
                let mode = if self.piano_playback { RenderMode::PianoRoll } else if self.timeline_playback { RenderMode::Timeline } else { RenderMode::Pattern };
                if self.export.last_mode != Some(mode) {
                    self.export.options.beats = export::suggested_beats(&self.capture_project(),mode);
                    self.export.last_mode = Some(mode);
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Modo: {}",mode.name()));
                    egui::ComboBox::from_id_salt("export-rate").selected_text(format!("{} Hz",self.export.options.sample_rate)).show_ui(ui,|ui| {
                        for rate in [44_100,48_000,96_000,192_000] { ui.selectable_value(&mut self.export.options.sample_rate,rate,format!("{rate} Hz")); }
                    });
                    ui.label("Duración (beats)");
                    ui.add(egui::DragValue::new(&mut self.export.options.beats).range(1..=MAX_BEATS));
                    if ui.button("Ajustar al contenido").clicked() { self.export.options.beats = export::suggested_beats(&self.capture_project(),mode); }
                    let seconds = f64::from(self.export.options.beats)*60.0/f64::from(self.bpm.get());
                    ui.label(format!("{seconds:.2} s"));
                    if ui.button("Exportar WAV…").clicked() { self.start_export(); }
                });
                ui.label("Desde beat 1, con mixer, master, efectos y automatización. Exporta el modo activo. Aumenta la duración para incluir colas de efectos; el patrón repite durante todo el intervalo.");
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_uses_the_current_mode_without_marking_saved_project_clean() {
        let mut app = super::super::tests::offline_app();
        app.timeline_playback = false;
        app.view = View::Sequencer;
        app.project.dirty = true;
        app.pattern = Pattern::demo();
        app.export.options.beats = 1;
        app.export.options.sample_rate = 8_000;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.wav");
        app.export_to(path.clone());
        assert!(app.export.is_active());
        assert!(app.project_busy());
        assert!(!app.editor_enabled());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.export.is_active() {
            assert!(std::time::Instant::now() < deadline);
            app.poll_export();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.project.dirty);
        assert!(app.project.path.is_none());
        let mut wav = hound::WavReader::open(path).unwrap();
        assert_eq!(wav.duration(), 4000);
        assert!(wav.samples::<i16>().any(|v| v.unwrap() != 0));
    }
}
