use super::*;
use jner_daw::project::{self, Project, ProjectView};
use std::path::PathBuf;

#[derive(Clone, Copy)]
enum Action {
    New,
    Open,
    Close,
}

enum Completed {
    Saved(PathBuf),
    Opened(PathBuf, Box<Project>),
}
type ProjectResult = Result<Completed, String>;

#[derive(Default)]
pub(super) struct ProjectUi {
    pub path: Option<PathBuf>,
    pub dirty: bool,
    job: Option<mpsc::Receiver<ProjectResult>>,
    pending: Option<Action>,
    confirm: bool,
    allow_close: bool,
    title: String,
}

impl App {
    pub(super) fn mark_dirty(&mut self) {
        self.project.dirty = true;
    }

    pub(super) fn project_busy(&self) -> bool {
        self.project.job.is_some()
            || self.loading.is_some()
            || self.timeline.is_dragging()
            || self.piano.is_dragging()
            || self.automation.is_dragging()
            || self.recording.is_active()
            || self.export.is_active()
    }

    pub(super) fn editor_enabled(&self) -> bool {
        self.project.job.is_none()
            && !self.project.confirm
            && !self.recording.is_active()
            && !self.export.is_active()
    }

    pub(super) fn capture_project(&self) -> Project {
        Project {
            bpm: self.bpm,
            master_gain: self.gain,
            pattern: self.pattern,
            sources: self.sources.clone(),
            mixer: self.mixer,
            timeline_sources: self.timeline.assets.clone(),
            arrangement: self.timeline.arrangement.clone(),
            notes: self.piano.sequence.clone(),
            synth: self.synth,
            effects: self.effects,
            automation: self.automation.sequence.clone(),
            workspace: self.timeline.workspace(
                match self.view {
                    View::Sequencer => ProjectView::Sequencer,
                    View::Mixer => ProjectView::Mixer,
                    View::Timeline => ProjectView::Timeline,
                    View::PianoRoll => ProjectView::PianoRoll,
                    View::Automation => ProjectView::Automation,
                },
                self.timeline_playback,
                self.piano_playback,
            ),
        }
    }

    pub(super) fn apply_project(&mut self, project: Project) -> Result<(), String> {
        project.validate()?; // Do not change the current session if validation fails.
        self.output = None;
        self.bpm = project.bpm;
        self.gain = project.master_gain;
        self.pattern = project.pattern;
        self.sources = project.sources;
        self.mixer = project.mixer;
        self.view = match project.workspace.view {
            ProjectView::Sequencer => View::Sequencer,
            ProjectView::Mixer => View::Mixer,
            ProjectView::Timeline => View::Timeline,
            ProjectView::PianoRoll => View::PianoRoll,
            ProjectView::Automation => View::Automation,
        };
        self.timeline_playback = project.workspace.timeline_playback;
        self.piano_playback = project.workspace.piano_playback;
        self.piano = PianoEditor::restore(project.notes);
        self.synth = project.synth;
        self.effects = project.effects;
        self.automation = AutomationEditor::restore(project.automation);
        self.timeline = TimelineEditor::restore(
            project.arrangement,
            project.timeline_sources,
            project.workspace,
        );
        self.project.dirty = false;
        self.open_selected();
        Ok(())
    }

    fn request_action(&mut self, action: Action, ctx: &egui::Context) {
        if self.project_busy() {
            self.notice = Some("Espera a que termine la operación actual.".into());
            return;
        }
        if self.project.dirty {
            self.project.pending = Some(action);
            self.project.confirm = true;
        } else {
            self.execute_action(action, ctx);
        }
    }

    fn execute_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::New => {
                if let Err(error) = self.apply_project(Project::empty()) {
                    self.error = Some(error);
                    return;
                }
                self.project.path = None;
                self.notice = Some("Proyecto nuevo. Añade pasos o clips para empezar.".into());
            }
            Action::Open => {
                let Some(path) = self
                    .file_dialog
                    .clone()
                    .set_title("Abrir proyecto Jner DAW")
                    .add_filter("Proyecto Jner DAW", &["jner"])
                    .pick_file()
                else {
                    return;
                };
                self.start_open(path);
            }
            Action::Close => {
                self.project.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub(super) fn start_open(&mut self, path: PathBuf) {
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("project-reader".into())
            .spawn(move || {
                let result =
                    project::load(&path).map(|project| Completed::Opened(path, Box::new(project)));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.project.job = Some(receiver);
                self.notice = Some("Abriendo proyecto…".into());
            }
            Err(error) => self.error = Some(format!("No se pudo iniciar la apertura: {error}")),
        }
    }

    fn start_save(&mut self, save_as: bool) {
        let path = if save_as || self.project.path.is_none() {
            let suggested = self
                .project
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or_else(
                    || "Proyecto.jner".into(),
                    |s| s.to_string_lossy().into_owned(),
                );
            self.file_dialog
                .clone()
                .set_title("Guardar proyecto Jner DAW")
                .set_file_name(suggested)
                .add_filter("Proyecto Jner DAW", &["jner"])
                .save_file()
        } else {
            self.project.path.clone()
        };
        let Some(mut path) = path else {
            self.project.pending = None;
            return;
        };
        if path.extension().is_none() {
            path.set_extension("jner");
        }
        self.save_to(path);
    }

    pub(super) fn save_to(&mut self, path: PathBuf) {
        let snapshot = self.capture_project();
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("project-writer".into())
            .spawn(move || {
                let result = project::save(&path, &snapshot).map(|()| Completed::Saved(path));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.project.job = Some(receiver);
                self.notice = Some("Guardando proyecto…".into());
            }
            Err(error) => {
                self.error = Some(format!("No se pudo iniciar el guardado: {error}"));
                self.project.confirm = self.project.pending.is_some();
            }
        }
    }

    fn poll_project(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.project.job else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("La operación de proyecto se interrumpió".into())
            }
        };
        self.project.job = None;
        match result {
            Ok(Completed::Saved(path)) => {
                self.project.path = Some(path);
                self.project.dirty = false;
                self.error = None;
                self.notice = Some("Proyecto guardado con sus fuentes de audio.".into());
                if let Some(action) = self.project.pending.take() {
                    self.execute_action(action, ctx);
                }
            }
            Ok(Completed::Opened(path, project)) => match self.apply_project(*project) {
                Ok(()) => {
                    self.project.path = Some(path);
                    self.notice =
                        Some("Proyecto abierto. La reproducción comienza detenida.".into());
                }
                Err(error) => self.error = Some(error),
            },
            Err(error) => {
                self.error = Some(error);
                self.notice = None;
                self.project.confirm = self.project.pending.is_some();
            }
        }
    }

    pub(super) fn project_events(&mut self, ctx: &egui::Context) {
        self.poll_project(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && !self.project.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_action(Action::Close, ctx);
        }
        if !self.project_busy() && !self.project.confirm {
            let (new, open, save, save_as) = ctx.input(|i| {
                (
                    i.modifiers.ctrl && i.key_pressed(egui::Key::N),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::O),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::S),
                    i.modifiers.shift,
                )
            });
            if new {
                self.request_action(Action::New, ctx);
            } else if open {
                self.request_action(Action::Open, ctx);
            } else if save {
                self.start_save(save_as);
            }
        }
        let name = self
            .project
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "Sin título".into(), |n| n.to_string_lossy().into_owned());
        let title = format!(
            "Jner DAW · {name}{}",
            if self.project.dirty { " *" } else { "" }
        );
        if self.project.title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.project.title = title;
        }
    }

    pub(super) fn project_toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!self.project_busy() && !self.project.confirm, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Nuevo").on_hover_text("Ctrl+N").clicked() {
                        self.request_action(Action::New, &ctx);
                    }
                    if ui.button("Abrir").on_hover_text("Ctrl+O").clicked() {
                        self.request_action(Action::Open, &ctx);
                    }
                    if ui.button("Guardar").on_hover_text("Ctrl+S").clicked() {
                        self.start_save(false);
                    }
                    if ui
                        .button("Guardar como")
                        .on_hover_text("Ctrl+Shift+S")
                        .clicked()
                    {
                        self.start_save(true);
                    }
                });
            });
            let name = self
                .project
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or_else(|| "Sin título".into(), |s| s.to_string_lossy().into_owned());
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{name}{}",
                        if self.project.dirty { " *" } else { "" }
                    ))
                    .color(MUTED)
                    .size(12.0),
                )
                .truncate(),
            );
            if self.project.job.is_some() {
                ui.spinner();
            }
        });
    }

    pub(super) fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        if !self.project.confirm {
            return;
        }
        let mut choice = 0;
        let response = egui::Modal::new(egui::Id::unique("unsaved-project")).show(ctx, |ui| {
            ui.heading("Cambios sin guardar");
            ui.label("Guarda el proyecto antes de continuar para conservar tus cambios.");
            if let Some(error) = &self.error {
                ui.colored_label(Color32::from_rgb(255, 158, 143), error);
            }
            ui.horizontal(|ui| {
                if ui.button("Guardar y continuar").clicked() {
                    choice = 1;
                }
                if ui.button("Descartar").clicked() {
                    choice = 2;
                }
                if ui.button("Cancelar").clicked() {
                    choice = 3;
                }
            });
        });
        if choice == 0 && response.should_close() {
            choice = 3;
        }
        match choice {
            1 => {
                self.project.confirm = false;
                self.start_save(false);
            }
            2 => {
                self.project.confirm = false;
                if let Some(action) = self.project.pending.take() {
                    self.execute_action(action, ctx);
                }
            }
            3 => {
                self.project.confirm = false;
                self.project.pending = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        super::super::tests::offline_app()
    }
    fn finish(app: &mut App, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while app.project.job.is_some() {
            app.poll_project(ctx);
            assert!(
                std::time::Instant::now() < deadline,
                "Project worker timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn unsaved_new_is_guarded_and_failed_save_never_replaces_the_session() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.bpm = Bpm::new(137).unwrap();
        app.mark_dirty();
        app.request_action(Action::New, &ctx);
        assert!(app.project.confirm);
        assert_eq!(app.bpm.get(), 137);
        let directory = tempfile::tempdir().unwrap();
        app.project.confirm = false;
        app.save_to(directory.path().join("missing").join("project.jner"));
        finish(&mut app, &ctx);
        assert!(app.project.confirm);
        assert!(app.project.dirty);
        assert_eq!(app.bpm.get(), 137);
        assert!(app.project.path.is_none());
        assert!(app.error.is_some());
    }

    #[test]
    fn save_and_continue_creates_a_clean_new_session_only_after_success() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.bpm = Bpm::new(137).unwrap();
        app.mark_dirty();
        app.request_action(Action::New, &ctx);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.jner");
        app.project.confirm = false;
        app.save_to(path.clone());
        finish(&mut app, &ctx);
        assert_eq!(project::load(&path).unwrap().bpm.get(), 137);
        assert_eq!(app.bpm.get(), 120);
        assert_eq!(app.pattern, Pattern::default());
        assert!(!app.project.dirty);
        assert!(app.project.path.is_none());
    }

    #[test]
    fn failed_open_preserves_the_old_project_path_and_dirty_state() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.bpm = Bpm::new(137).unwrap();
        app.mark_dirty();
        let old_path = PathBuf::from("previous.jner");
        app.project.path = Some(old_path.clone());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.jner");
        std::fs::write(&path, "{invalid").unwrap();
        app.start_open(path);
        finish(&mut app, &ctx);
        assert_eq!(app.bpm.get(), 137);
        assert!(app.project.dirty);
        assert_eq!(app.project.path, Some(old_path));
        assert!(app.error.is_some());
    }

    #[test]
    fn loaded_project_is_applied_as_a_whole_and_invalid_data_is_not_applied() {
        let mut app = app();
        let ctx = egui::Context::default();
        let source = Arc::clone(&app.sources[0]);
        app.insert_timeline_clip(2, source, 1440);
        app.bpm = Bpm::new(137).unwrap();
        app.mixer.tracks[2].solo = true;
        app.piano.sequence.add(1, 64, 77, 480, 960).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jner");
        project::save(&path, &app.capture_project()).unwrap();
        app.apply_project(Project::empty()).unwrap();
        app.start_open(path.clone());
        finish(&mut app, &ctx);
        assert_eq!(app.bpm.get(), 137);
        assert_eq!(app.timeline.arrangement.clips()[0].track, 2);
        assert!(app.mixer.tracks[2].solo);
        assert_eq!(app.piano.sequence.notes()[0].velocity, 77);
        assert!(app.timeline_playback);
        assert!(app.output.is_none());
        assert_eq!(app.project.path, Some(path));
        assert!(!app.project.dirty);
        let mut invalid = app.capture_project();
        invalid.master_gain = f32::NAN;
        assert!(app.apply_project(invalid).is_err());
        assert_eq!(app.bpm.get(), 137);
    }
}
