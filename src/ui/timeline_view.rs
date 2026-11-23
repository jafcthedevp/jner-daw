use super::*;
use jner_daw::timeline::{
    Arrangement, AudioClip, MAX_ASSETS, MAX_CLIPS, MAX_START_TICK, TICKS_PER_BEAT, quantize,
};

const RULER: f32 = 34.0;
const LANE: f32 = 76.0;

pub(super) struct TimelineEditor {
    pub assets: Vec<Arc<SampleSource>>,
    pub arrangement: Arrangement,
    selected: Option<u32>,
    cursor: u64,
    insert_track: usize,
    zoom: f32,
    snap: bool,
    drag: Option<ClipDrag>,
}

impl Default for TimelineEditor {
    fn default() -> Self {
        Self {
            assets: Vec::new(),
            arrangement: Arrangement::default(),
            selected: None,
            cursor: 0,
            insert_track: 0,
            zoom: 64.0,
            snap: true,
            drag: None,
        }
    }
}

impl TimelineEditor {
    pub(super) fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub(super) fn workspace(
        &self,
        view: jner_daw::project::ProjectView,
        timeline_playback: bool,
        piano_playback: bool,
    ) -> jner_daw::project::Workspace {
        jner_daw::project::Workspace {
            view,
            timeline_playback,
            piano_playback,
            cursor_tick: self.cursor,
            insert_track: self.insert_track,
            zoom: self.zoom,
            snap: self.snap,
        }
    }
    pub(super) fn restore(
        arrangement: Arrangement,
        assets: Vec<Arc<SampleSource>>,
        workspace: jner_daw::project::Workspace,
    ) -> Self {
        Self {
            arrangement,
            assets,
            cursor: workspace.cursor_tick,
            insert_track: workspace.insert_track,
            zoom: workspace.zoom,
            snap: workspace.snap,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy)]
enum EditKind {
    Move,
    Left,
    Right,
}

#[derive(Clone, Copy)]
struct ClipDrag {
    original: AudioClip,
    origin: egui::Pos2,
    kind: EditKind,
}

impl App {
    pub(super) fn insert_timeline_clip(
        &mut self,
        track: usize,
        source: Arc<SampleSource>,
        tick: u64,
    ) {
        self.output = None;
        self.timeline
            .arrangement
            .prune_assets(&mut self.timeline.assets);
        let existing = self
            .timeline
            .assets
            .iter()
            .position(|s| Arc::ptr_eq(s, &source));
        let result = if self.timeline.arrangement.clips().len() >= MAX_CLIPS {
            Err("Máximo de 32 clips por sesión")
        } else if existing.is_none() && self.timeline.assets.len() >= MAX_ASSETS {
            Err("Máximo de 16 fuentes. Quita clips que ya no necesites para liberar sus fuentes")
        } else {
            let asset = existing.unwrap_or_else(|| {
                self.timeline.assets.push(Arc::clone(&source));
                self.timeline.assets.len() - 1
            });
            self.timeline
                .arrangement
                .add(track, asset, tick, source.frame_count())
        };
        self.open_selected();
        match result {
            Ok(id) => {
                self.mark_dirty();
                self.timeline.selected = Some(id);
                self.notice = Some("Clip añadido. Pulsa Play para escuchar el timeline.".into());
            }
            Err(error) => self.error = Some(error.into()),
        }
    }

    fn timeline_example(&mut self) {
        if !self.timeline.arrangement.clips().is_empty() {
            return;
        }
        self.output = None;
        self.timeline.assets = self.sources.to_vec();
        for (track, beats) in [
            (0, vec![0.0, 1.0, 2.0, 3.0]),
            (1, vec![1.0, 3.0]),
            (2, vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]),
            (3, vec![3.0]),
        ] {
            for beat in beats {
                if let Err(error) = self.timeline.arrangement.add(
                    track,
                    track,
                    (beat * TICKS_PER_BEAT as f64) as u64,
                    self.sources[track].frame_count(),
                ) {
                    self.error = Some(error.into());
                    return;
                }
            }
        }
        self.open_selected();
        self.mark_dirty();
        self.notice = Some(
            "Ejemplo de clips listo. El timeline se reproduce una vez; no repite el patrón.".into(),
        );
    }

    pub(super) fn timeline_controls(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) {
        super::shell::card().inner_margin(16.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("TIMELINE").strong().size(15.0));
                ui.checkbox(&mut self.timeline.snap, "Snap 1/16");
                ui.label("Zoom");
                let mut zoom = self.timeline.zoom / 64.0 * 100.0;
                ui.add(egui::Slider::new(&mut zoom, 37.5..=250.0).suffix(" %"));
                self.timeline.zoom = zoom / 100.0 * 64.0;
                if ui.add_enabled(self.timeline.arrangement.clips().is_empty(), egui::Button::new("Crear ejemplo")).clicked() { self.timeline_example(); }
            });
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("clip-track").width(85.0).selected_text(TRACK_NAMES[self.timeline.insert_track]).show_ui(ui, |ui| {
                    for (track, name) in TRACK_NAMES.iter().enumerate() { ui.selectable_value(&mut self.timeline.insert_track, track, *name); }
                });
                ui.label("Insertar en beat");
                let mut beat = self.timeline.cursor as f64 / TICKS_PER_BEAT as f64 + 1.0;
                if ui.add(egui::DragValue::new(&mut beat).range(1.0..=257.0).speed(0.25)).changed() {
                    self.timeline.cursor = quantize((beat - 1.0) * TICKS_PER_BEAT as f64, self.timeline.snap);
                }
                if ui.button("Sonido de pista").clicked() {
                    let track = self.timeline.insert_track;
                    self.insert_timeline_clip(track, Arc::clone(&self.sources[track]), self.timeline.cursor);
                }
                if ui.add_enabled(self.loading.is_none(), egui::Button::new("Importar WAV")).clicked() {
                    self.start_load_for(LoadDestination::TimelineClip { track: self.timeline.insert_track, tick: self.timeline.cursor });
                }
                if ui.add_enabled(self.timeline.selected.is_some(), egui::Button::new("Quitar clip")).clicked()
                    && let Some(id) = self.timeline.selected.take() {
                    self.output = None;
                    self.timeline.arrangement.remove(id);
                    self.timeline.arrangement.prune_assets(&mut self.timeline.assets);
                    self.open_selected();
                    self.mark_dirty();
                }
            });
            let beat = snapshot.musical_ticks / TICKS_PER_BEAT;
            ui.label(RichText::new(format!("Playhead  {}.{}   ·   {} clips", beat / 4 + 1, beat % 4 + 1, self.timeline.arrangement.clips().len())).color(ACCENT).size(12.0));
            self.timeline_canvas(ui, snapshot);
            if let Some(clip) = self.timeline.selected.and_then(|id| self.timeline.arrangement.get(id))
                && let Some(source) = self.timeline.assets.get(clip.asset) {
                ui.label(RichText::new(format!("{} · inicio beat {:.2} · fuente {:.3}–{:.3} s", source.name,
                    clip.start_tick as f64 / TICKS_PER_BEAT as f64 + 1.0,
                    f64::from(clip.in_frame) / f64::from(source.sample_rate()), f64::from(clip.out_frame) / f64::from(source.sample_rate()))).size(11.0).color(MUTED));
            }
            ui.label(RichText::new("Arrastra el cuerpo para mover y los bordes para recortar. Editar clips detiene la reproducción.").size(11.0).color(MUTED));
        });
    }

    fn timeline_canvas(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) {
        let height = RULER + LANE * TRACKS as f32;
        ui.horizontal_top(|ui| {
            let (labels, _) = ui.allocate_exact_size(egui::vec2(76.0, height), egui::Sense::hover());
            for (track, name) in TRACK_NAMES.iter().enumerate() {
                ui.painter().text(egui::pos2(labels.left(), labels.top() + RULER + LANE * (track as f32 + 0.5)), egui::Align2::LEFT_CENTER,
                    *name, egui::FontId::proportional(13.0), TRACK_COLORS[track]);
            }
            egui::ScrollArea::horizontal().id_salt("timeline-scroll").show(ui, |ui| {
                let tail = self.timeline.arrangement.clips().iter().filter_map(|c| self.timeline.assets.get(c.asset).map(|s| c.duration_ticks(s.sample_rate(), self.bpm) / TICKS_PER_BEAT as f64)).fold(4.0_f64, f64::max);
                let beats = (MAX_START_TICK / TICKS_PER_BEAT) as f32 + tail as f32 + 4.0;
                let (canvas, response) = ui.allocate_exact_size(egui::vec2(beats * self.timeline.zoom, height), egui::Sense::click());
                let painter = ui.painter();
                painter.rect_filled(canvas, 3.0, Color32::from_rgb(19, 19, 21));
                for division in 0..=(beats as usize * 4) {
                    let x = canvas.left() + division as f32 * self.timeline.zoom / 4.0;
                    let color = if division % 16 == 0 { Color32::from_rgb(74, 73, 81) } else if division % 4 == 0 { Color32::from_rgb(47, 47, 52) } else { Color32::from_rgb(30, 30, 34) };
                    painter.line_segment([egui::pos2(x, canvas.top() + RULER), egui::pos2(x, canvas.bottom())], egui::Stroke::new(1.0, color));
                    if division % 16 == 0 {
                        painter.text(egui::pos2(x + 4.0, canvas.top() + 12.0), egui::Align2::LEFT_CENTER, (division / 16 + 1).to_string(), egui::FontId::monospace(11.0), MUTED);
                    }
                }
                for track in 0..=TRACKS {
                    let y = canvas.top() + RULER + track as f32 * LANE;
                    painter.line_segment([egui::pos2(canvas.left(), y), egui::pos2(canvas.right(), y)], egui::Stroke::new(1.0, Color32::from_rgb(47, 47, 52)));
                }
                if response.clicked() && let Some(pointer) = response.interact_pointer_pos()
                    && pointer.y < canvas.top() + RULER {
                    self.timeline.cursor = quantize(f64::from((pointer.x - canvas.left()) / self.timeline.zoom) * TICKS_PER_BEAT as f64, self.timeline.snap);
                }
                let cursor_x = canvas.left() + self.timeline.cursor as f32 / TICKS_PER_BEAT as f32 * self.timeline.zoom;
                ui.painter().line_segment([egui::pos2(cursor_x, canvas.top()), egui::pos2(cursor_x, canvas.bottom())], egui::Stroke::new(1.0, Color32::from_rgb(238, 192, 103)));
                let clips = self.timeline.arrangement.clips().to_vec();
                for clip in clips {
                    let Some(source) = self.timeline.assets.get(clip.asset) else { continue; };
                    let duration = clip.duration_ticks(source.sample_rate(), self.bpm) / TICKS_PER_BEAT as f64;
                    let x = canvas.left() + clip.start_tick as f32 / TICKS_PER_BEAT as f32 * self.timeline.zoom;
                    let y = canvas.top() + RULER + clip.track as f32 * LANE + 7.0;
                    let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2((duration as f32 * self.timeline.zoom).max(12.0), LANE - 14.0));
                    let response = ui.interact(rect, egui::Id::unique(("audio-clip", clip.id)), egui::Sense::click_and_drag());
                    let color = TRACK_COLORS[clip.track];
                    ui.painter().rect_filled(rect, 4.0, color.gamma_multiply(0.65));
                    ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(if self.timeline.selected == Some(clip.id) { 2.0 } else { 1.0 }, if self.timeline.selected == Some(clip.id) { Color32::WHITE } else { color }), egui::StrokeKind::Inside);
                    let edge = 8.0_f32.min(rect.width() / 4.0);
                    for handle in [rect.left() + edge * 0.5, rect.right() - edge * 0.5] {
                        ui.painter().line_segment([egui::pos2(handle, rect.top() + 12.0), egui::pos2(handle, rect.bottom() - 12.0)], egui::Stroke::new(2.0, color));
                    }
                    if rect.width() > 65.0 {
                        ui.painter().with_clip_rect(rect.shrink(6.0)).text(rect.left_top() + egui::vec2(9.0, 13.0), egui::Align2::LEFT_TOP, &source.name, egui::FontId::proportional(11.0), Color32::WHITE);
                    }
                    if response.clicked() { self.timeline.selected = Some(clip.id); }
                    if response.drag_started() && let Some(pointer) = response.interact_pointer_pos() {
                        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pointer);
                        let kind = if origin.x < rect.left() + edge { EditKind::Left }
                            else if origin.x > rect.right() - edge { EditKind::Right } else { EditKind::Move };
                        self.timeline.drag = Some(ClipDrag { original: clip, origin, kind });
                        self.timeline.selected = Some(clip.id);
                        self.output = None;
                    }
                    if response.dragged() && let Some(drag) = self.timeline.drag
                        && drag.original.id == clip.id && let Some(pointer) = response.interact_pointer_pos() {
                        let delta = pointer - drag.origin;
                        let ticks = f64::from(delta.x / self.timeline.zoom) * TICKS_PER_BEAT as f64;
                        let source = &self.timeline.assets[clip.asset];
                        let changed = match drag.kind {
                            EditKind::Move => drag.original.moved(ticks, (drag.original.track as f32 + (delta.y / LANE).round()).clamp(0.0, (TRACKS - 1) as f32) as usize, self.timeline.snap),
                            EditKind::Left => drag.original.trim_left(ticks, source.sample_rate(), self.bpm, self.timeline.snap),
                            EditKind::Right => drag.original.trim_right(ticks, source.sample_rate(), source.frame_count(), self.bpm, self.timeline.snap),
                        };
                        if let Err(error) = self.timeline.arrangement.replace(changed) { self.error = Some(error.into()); }
                    }
                    if response.drag_stopped() && self.timeline.drag.is_some_and(|d| d.original.id == clip.id) {
                        self.timeline.drag = None;
                        self.open_selected();
                        self.mark_dirty();
                        self.notice = Some("Clip actualizado. El audio original se conserva; pulsa Play para escucharlo.".into());
                    }
                }
                let playhead = canvas.left() + snapshot.musical_ticks as f32 / TICKS_PER_BEAT as f32 * self.timeline.zoom;
                ui.painter().line_segment([egui::pos2(playhead, canvas.top()), egui::pos2(playhead, canvas.bottom())], egui::Stroke::new(2.0, ACCENT));
            });
        });
    }
}
