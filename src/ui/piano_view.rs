use super::*;
use jner_daw::{
    midi::{END_TICK, MAX_NOTES, Note, NoteSequence},
    timeline::{SNAP_TICKS, TICKS_PER_BEAT},
};

const ROW: f32 = 20.0;
const KEYS: f32 = 64.0;
const RULER: f32 = 28.0;

pub(super) struct PianoEditor {
    pub sequence: NoteSequence,
    pub selected: Option<u32>,
    pub(super) track: usize,
    snap: bool,
    zoom: f32,
    velocity: u8,
    duration: u64,
    center: bool,
    drag: Option<NoteDrag>,
}
impl Default for PianoEditor {
    fn default() -> Self {
        Self {
            sequence: NoteSequence::default(),
            selected: None,
            track: 0,
            snap: true,
            zoom: 64.0,
            velocity: 100,
            duration: TICKS_PER_BEAT,
            center: true,
            drag: None,
        }
    }
}
impl PianoEditor {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn restore(sequence: NoteSequence) -> Self {
        Self {
            sequence,
            ..Self::default()
        }
    }
}
#[derive(Clone, Copy)]
struct NoteDrag {
    original: Note,
    origin: egui::Pos2,
    resize: bool,
}

fn pitch_name(pitch: u8) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        names[usize::from(pitch % 12)],
        i16::from(pitch) / 12 - 1
    )
}

impl App {
    pub(super) fn add_note(&mut self, pitch: u8, start: u64) {
        let duration = if self.piano.snap {
            (self.piano.duration.div_ceil(SNAP_TICKS) * SNAP_TICKS).min(END_TICK)
        } else {
            self.piano.duration
        };
        let duration = duration.min(END_TICK.saturating_sub(start));
        match self.piano.sequence.add(
            self.piano.track,
            pitch,
            self.piano.velocity,
            start,
            duration,
        ) {
            Ok(id) => {
                self.piano.selected = Some(id);
                self.notes_changed();
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn edit_note(&mut self, note: Note) {
        if self.piano.sequence.get(note.id) == Some(note) {
            return;
        }
        match self.piano.sequence.replace(note) {
            Ok(()) => self.notes_changed(),
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn delete_note(&mut self) {
        if let Some(id) = self.piano.selected.take() {
            self.piano.sequence.remove(id);
            self.notes_changed();
        }
    }
    fn notes_changed(&mut self) {
        self.output = None;
        self.mark_dirty();
        self.open_selected();
    }

    pub(super) fn piano_controls(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) {
        super::shell::card().inner_margin(12.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("PIANO ROLL").strong().size(15.0));
                ui.checkbox(&mut self.piano.snap,"Snap 1/16");
                ui.label("Zoom");
                ui.add(egui::Slider::new(&mut self.piano.zoom,32.0..=128.0).show_value(false));
                if ui.button("Centrar C4").clicked() {self.piano.center = true;}
                if ui.add_enabled(self.piano.sequence.notes().is_empty(),egui::Button::new("Crear ejemplo")).clicked() {
                    for (index,pitch) in [60,64,67,72].into_iter().enumerate() {
                        self.add_note(pitch,index as u64*TICKS_PER_BEAT);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("note-track").selected_text(format!("Pista {}",self.piano.track+1)).show_ui(ui, |ui| {
                    for track in 0..TRACKS {ui.selectable_value(&mut self.piano.track,track,format!("Pista {}",track+1));}
                });
                ui.label("Nueva nota: velocity");
                ui.add(egui::DragValue::new(&mut self.piano.velocity).range(1..=127));
                ui.label("duración (beats)");
                let mut duration=self.piano.duration as f64/TICKS_PER_BEAT as f64;
                if ui.add(egui::DragValue::new(&mut duration).range(0.25..=64.0).speed(0.25)).changed() {
                    self.piano.duration=(duration*TICKS_PER_BEAT as f64).round() as u64;
                }
            });
            ui.label(RichText::new(format!("{} / {MAX_NOTES} notas · {} notas activas · {} voces synth · On {} / Off {}",self.piano.sequence.notes().len(),snapshot.active_notes,snapshot.synth_voices,snapshot.note_ons,snapshot.note_offs)).color(ACCENT).size(12.0));
            self.synth_controls(ui);
            self.piano_canvas(ui,snapshot);
            if let Some(mut note)=self.piano.selected.and_then(|id|self.piano.sequence.get(id)) {
                let original=note;
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} · Pista {}",pitch_name(note.pitch),note.track+1));
                    ui.label("Pitch MIDI");
                    ui.add(egui::DragValue::new(&mut note.pitch).range(0..=127));
                    ui.label("Velocity");
                    ui.add(egui::Slider::new(&mut note.velocity,1..=127));
                    ui.label("Pista");
                    let mut track=note.track+1;
                    if ui.add(egui::DragValue::new(&mut track).range(1..=TRACKS)).changed() {note.track=track-1;}
                });
                ui.horizontal_wrapped(|ui| {
                    let mut beat=note.start_tick as f64/TICKS_PER_BEAT as f64+1.0;
                    let mut duration=note.duration_tick as f64/TICKS_PER_BEAT as f64;
                    ui.label("Inicio (beat)");
                    if ui.add(egui::DragValue::new(&mut beat).range(1.0..=64.999).speed(0.25)).changed() {
                        note=note.moved((beat-1.0)*TICKS_PER_BEAT as f64-note.start_tick as f64,0,self.piano.snap);
                    }
                    ui.label("Duración (beats)");
                    if ui.add(egui::DragValue::new(&mut duration).range(1.0/TICKS_PER_BEAT as f64..=64.0).speed(0.25)).changed() {
                        note=note.resized(duration*TICKS_PER_BEAT as f64-note.duration_tick as f64,self.piano.snap);
                    }
                    if ui.button("Borrar nota").clicked() {self.delete_note();}
                });
                if note != original && self.piano.selected.is_some() {self.edit_note(note);}
            }
            ui.label(RichText::new("Doble clic en la rejilla: crear. Arrastra el cuerpo: mover; borde derecho: duración. Supr: borrar. Editar detiene el transport.").size(11.0).color(MUTED));
        });
    }

    fn piano_canvas(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) -> egui::Rect {
        let mut scroll = egui::ScrollArea::both()
            .id_salt("piano-scroll")
            .max_height(350.0)
            .auto_shrink([false, false])
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: egui::scroll_area::DragScroll::Never,
                ..Default::default()
            });
        if self.piano.center {
            scroll = scroll.vertical_scroll_offset((127.0 - 72.0) * ROW);
            self.piano.center = false;
        }
        scroll
            .show(ui, |ui| {
                let zoom = self.piano.zoom;
                let (canvas, response) = ui.allocate_exact_size(
                    egui::vec2(KEYS + 64.0 * zoom, RULER + 128.0 * ROW),
                    egui::Sense::click(),
                );
                let grid_left = canvas.left() + KEYS;
                for pitch in 0..=127_u8 {
                    let y = canvas.top() + RULER + (127 - pitch) as f32 * ROW;
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(canvas.left(), y),
                        egui::vec2(canvas.width(), ROW),
                    );
                    let black = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        if black {
                            Color32::from_rgb(19, 19, 21)
                        } else {
                            Color32::from_rgb(27, 27, 30)
                        },
                    );
                    ui.painter().text(
                        egui::pos2(canvas.left() + 5.0, y + ROW / 2.0),
                        egui::Align2::LEFT_CENTER,
                        pitch_name(pitch),
                        egui::FontId::monospace(10.0),
                        MUTED,
                    );
                    ui.painter().hline(
                        canvas.x_range(),
                        y,
                        egui::Stroke::new(1.0, Color32::from_rgb(42, 42, 46)),
                    );
                }
                for division in 0..=256 {
                    let x = grid_left + division as f32 * zoom / 4.0;
                    ui.painter().vline(
                        x,
                        canvas.y_range(),
                        egui::Stroke::new(
                            1.0,
                            if division % 16 == 0 {
                                Color32::from_rgb(81, 80, 88)
                            } else {
                                Color32::from_rgb(42, 42, 46)
                            },
                        ),
                    );
                }
                let notes = self.piano.sequence.notes().to_vec();
                let note_rect = |n: Note| {
                    egui::Rect::from_min_size(
                        egui::pos2(
                            grid_left + n.start_tick as f32 / TICKS_PER_BEAT as f32 * zoom,
                            canvas.top() + RULER + (127 - n.pitch) as f32 * ROW + 1.0,
                        ),
                        egui::vec2(
                            (n.duration_tick as f32 / TICKS_PER_BEAT as f32 * zoom).max(6.0),
                            ROW - 2.0,
                        ),
                    )
                };
                if response.double_clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                    && pos.x >= grid_left
                    && pos.y >= canvas.top() + RULER
                    && pos.y >= ui.clip_rect().top() + RULER
                    && !notes.iter().any(|n| note_rect(*n).contains(pos))
                {
                    let row = ((pos.y - canvas.top() - RULER) / ROW)
                        .floor()
                        .clamp(0.0, 127.0) as u8;
                    let step = if self.piano.snap { SNAP_TICKS } else { 1 };
                    let tick = (((pos.x - grid_left) / zoom) as f64 * TICKS_PER_BEAT as f64
                        / step as f64)
                        .floor() as u64
                        * step;
                    self.add_note(127 - row, tick.min(END_TICK - step));
                }
                for note in notes {
                    let rect = note_rect(note);
                    let response = ui.interact(
                        rect,
                        egui::Id::unique(("midi-note", note.id)),
                        egui::Sense::click_and_drag(),
                    );
                    let color = TRACK_COLORS[note.track];
                    ui.painter().rect_filled(
                        rect,
                        2.0,
                        color.gamma_multiply(0.35 + f32::from(note.velocity) / 127.0 * 0.6),
                    );
                    ui.painter().rect_stroke(
                        rect,
                        2.0,
                        egui::Stroke::new(
                            if self.piano.selected == Some(note.id) {
                                2.0
                            } else {
                                1.0
                            },
                            if self.piano.selected == Some(note.id) {
                                Color32::WHITE
                            } else {
                                color
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if rect.width() > 36.0 {
                        ui.painter().with_clip_rect(rect.shrink(2.0)).text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            pitch_name(note.pitch),
                            egui::FontId::monospace(10.0),
                            Color32::WHITE,
                        );
                    }
                    let edge = 6.0_f32.min(rect.width() / 3.0);
                    ui.painter().vline(
                        rect.right() - 2.0,
                        rect.y_range().shrink(3.0),
                        egui::Stroke::new(1.0, Color32::WHITE),
                    );
                    if response.clicked() {
                        self.piano.selected = Some(note.id);
                    }
                    if response.drag_started()
                        && let Some(pos) = response.interact_pointer_pos()
                    {
                        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                        self.piano.drag = Some(NoteDrag {
                            original: note,
                            origin,
                            resize: origin.x > rect.right() - edge,
                        });
                        self.piano.selected = Some(note.id);
                        self.output = None;
                    }
                    if response.dragged()
                        && let Some(drag) = self.piano.drag
                        && drag.original.id == note.id
                        && let Some(pos) = response.interact_pointer_pos()
                    {
                        let delta = pos - drag.origin;
                        let ticks = f64::from(delta.x / zoom) * TICKS_PER_BEAT as f64;
                        let changed = if drag.resize {
                            drag.original.resized(ticks, self.piano.snap)
                        } else {
                            drag.original.moved(
                                ticks,
                                (-delta.y / ROW).round().clamp(-127.0, 127.0) as i16,
                                self.piano.snap,
                            )
                        };
                        if let Err(error) = self.piano.sequence.replace(changed) {
                            self.error = Some(error);
                        }
                    }
                }
                if let Some(drag) = self.piano.drag
                    && !ui.input(|i| i.pointer.primary_down())
                {
                    self.piano.drag = None;
                    if self.piano.sequence.get(drag.original.id) != Some(drag.original) {
                        self.mark_dirty();
                    }
                    self.open_selected();
                }
                let x = grid_left + snapshot.musical_ticks as f32 / TICKS_PER_BEAT as f32 * zoom;
                ui.painter()
                    .vline(x, canvas.y_range(), egui::Stroke::new(2.0, ACCENT));
                let top = canvas.top().max(ui.clip_rect().top());
                let header = egui::Rect::from_min_size(
                    egui::pos2(canvas.left(), top),
                    egui::vec2(canvas.width(), RULER),
                );
                ui.painter()
                    .rect_filled(header, 0.0, Color32::from_rgb(23, 23, 26));
                ui.interact(
                    header,
                    egui::Id::unique("piano-ruler"),
                    egui::Sense::click(),
                );
                for bar in 0..16 {
                    ui.painter().text(
                        egui::pos2(grid_left + bar as f32 * 4.0 * zoom + 4.0, top + RULER / 2.0),
                        egui::Align2::LEFT_CENTER,
                        (bar + 1).to_string(),
                        egui::FontId::monospace(11.0),
                        MUTED,
                    );
                }
                if !ui.ctx().text_edit_focused()
                    && ui.input(|i| i.modifiers.is_none() && i.key_pressed(egui::Key::Delete))
                {
                    self.delete_note();
                }
                canvas
            })
            .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn note_edits_mark_the_project_and_leave_samples_and_audio_clips_intact() {
        let mut app = super::super::tests::offline_app();
        let source = Arc::clone(&app.sources[0]);
        app.insert_timeline_clip(0, Arc::clone(&source), 0);
        app.switch_view(View::PianoRoll);
        app.project.dirty = false;
        app.add_note(60, 240);
        assert!(app.project.dirty);
        let id = app.piano.selected.unwrap();
        app.edit_note(Note {
            velocity: 77,
            ..app
                .piano
                .sequence
                .get(id)
                .unwrap()
                .moved(240.0, 7, true)
                .resized(240.0, true)
        });
        let note = app.piano.sequence.get(id).unwrap();
        assert_eq!(
            (
                note.start_tick,
                note.pitch,
                note.velocity,
                note.duration_tick
            ),
            (480, 67, 77, 1200)
        );
        assert_eq!(app.timeline.arrangement.clips().len(), 1);
        assert!(Arc::ptr_eq(&app.sources[0], &source));
        app.delete_note();
        assert!(app.piano.sequence.notes().is_empty());
        assert!(app.piano_playback && !app.timeline_playback);
    }

    #[test]
    fn pointer_events_create_move_and_resize_a_note_on_the_actual_canvas() {
        fn frame(
            app: &mut App,
            ctx: &egui::Context,
            time: f64,
            events: Vec<egui::Event>,
        ) -> egui::Rect {
            let mut canvas = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 600.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        canvas = app.piano_canvas(ui, Snapshot::default());
                    });
                },
            );
            output.textures_delta.clear();
            canvas
        }
        fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }
        }
        let ctx = egui::Context::default();
        let mut app = super::super::tests::offline_app();
        app.switch_view(View::PianoRoll);
        app.piano.center = false;
        let canvas = frame(&mut app, &ctx, 0.0, vec![]);
        let point = canvas.min + egui::vec2(KEYS + 32.0, RULER + 7.0 * ROW + ROW / 2.0);
        for (time, pressed) in [(0.1, true), (0.15, false), (0.2, true), (0.25, false)] {
            frame(
                &mut app,
                &ctx,
                time,
                vec![egui::Event::PointerMoved(point), button(point, pressed)],
            );
        }
        assert_eq!(
            app.piano.sequence.notes().len(),
            1,
            "Double click must create a note"
        );
        let original = app.piano.sequence.notes()[0];
        assert_eq!(
            (original.pitch, original.start_tick, original.duration_tick),
            (120, 480, 960)
        );
        let body = point + egui::vec2(20.0, 0.0);
        frame(&mut app, &ctx, 0.7, vec![egui::Event::PointerMoved(body)]);
        frame(&mut app, &ctx, 0.75, vec![button(body, true)]);
        let moved = body + egui::vec2(32.0, -ROW * 2.0);
        frame(&mut app, &ctx, 0.8, vec![egui::Event::PointerMoved(moved)]);
        frame(&mut app, &ctx, 0.85, vec![button(moved, false)]);
        let note = app.piano.sequence.get(original.id).unwrap();
        assert_eq!((note.start_tick, note.pitch), (960, 122));
        let edge = canvas.min + egui::vec2(KEYS + 128.0 - 2.0, RULER + 5.0 * ROW + ROW / 2.0);
        frame(&mut app, &ctx, 1.2, vec![egui::Event::PointerMoved(edge)]);
        frame(&mut app, &ctx, 1.25, vec![button(edge, true)]);
        let resized = edge + egui::vec2(32.0, 0.0);
        frame(
            &mut app,
            &ctx,
            1.3,
            vec![egui::Event::PointerMoved(resized)],
        );
        frame(&mut app, &ctx, 1.35, vec![button(resized, false)]);
        assert_eq!(
            app.piano.sequence.get(original.id).unwrap().duration_tick,
            1440
        );
        assert!(!app.piano.is_dragging());
        // Clickable notes may own keyboard focus. Delete still applies unless a text field is being edited.
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique(("midi-note", original.id))));
        frame(
            &mut app,
            &ctx,
            1.6,
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.piano.sequence.notes().is_empty());
    }
}
