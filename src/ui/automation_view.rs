use super::*;
use jner_daw::{
    automation::{Automation, END_TICK, MAX_POINTS, Point, Target},
    timeline::{TICKS_PER_BEAT, quantize},
};
const LEFT: f32 = 58.0;
const RULER: f32 = 30.0;
const HEIGHT: f32 = 230.0;

pub(super) struct AutomationEditor {
    pub sequence: Automation,
    target: Target,
    selected: Option<u32>,
    snap: bool,
    zoom: f32,
    drag: Option<PointDrag>,
}
impl Default for AutomationEditor {
    fn default() -> Self {
        Self {
            sequence: Automation::default(),
            target: Target::TrackGain(0),
            selected: None,
            snap: true,
            zoom: 64.0,
            drag: None,
        }
    }
}
impl AutomationEditor {
    pub fn restore(sequence: Automation) -> Self {
        Self {
            sequence,
            ..Self::default()
        }
    }
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
}
#[derive(Clone, Copy)]
struct PointDrag {
    original: Point,
    origin: egui::Pos2,
}

impl App {
    fn automation_changed(&mut self) {
        self.output = None;
        self.mark_dirty();
        self.open_selected();
    }
    fn automation_base(&self, target: Target) -> f32 {
        match target {
            Target::TrackGain(t) => self.mixer.tracks[t].gain,
            Target::TrackPan(t) => self.mixer.tracks[t].pan,
            Target::DelayMix => self.effects.delay_mix,
        }
    }
    fn add_automation_point(&mut self, tick: u64, value: f32) {
        match self
            .automation
            .sequence
            .add(self.automation.target, tick, value)
        {
            Ok(id) => {
                self.automation.selected = Some(id);
                self.automation_changed();
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn edit_automation_point(&mut self, point: Point) {
        if self
            .automation
            .sequence
            .get(self.automation.target, point.id)
            == Some(point)
        {
            return;
        }
        match self
            .automation
            .sequence
            .replace(self.automation.target, point)
        {
            Ok(()) => self.automation_changed(),
            Err(error) => self.error = Some(error),
        }
    }
    fn remove_automation_point(&mut self) {
        if let Some(id) = self.automation.selected.take() {
            self.automation.sequence.remove(self.automation.target, id);
            self.automation_changed();
        }
    }
    fn automation_example(&mut self) {
        if self
            .automation
            .sequence
            .lane(self.automation.target)
            .is_some_and(|l| !l.points.is_empty())
        {
            return;
        }
        let (lo, hi) = self.automation.target.range();
        let mut sequence = self.automation.sequence.clone();
        if let Err(error) = sequence.set_enabled(self.automation.target, true) {
            self.error = Some(error);
            return;
        }
        let mut selected = None;
        for (tick, value) in [(0, lo), (3840, hi), (7680, lo + (hi - lo) * 0.25)] {
            match sequence.add(self.automation.target, tick, value) {
                Ok(id) => selected = Some(id),
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
        }
        self.automation.sequence = sequence;
        self.automation.selected = selected;
        self.automation_changed();
    }
    pub(super) fn automation_controls(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) {
        super::shell::card().inner_margin(12.0).show(ui,|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("AUTOMATIZACIÓN").strong().size(15.0));
                let previous=self.automation.target;
                egui::ComboBox::from_id_salt("automation-target").selected_text(previous.name()).show_ui(ui,|ui| {
                    for target in Target::ALL {ui.selectable_value(&mut self.automation.target,target,target.name());}
                });
                if previous!=self.automation.target {self.automation.selected=None;}
                let mut enabled=self.automation.sequence.lane(self.automation.target).is_some_and(|l|l.enabled);
                if ui.checkbox(&mut enabled,"Leer curva").changed() {
                    match self.automation.sequence.set_enabled(self.automation.target,enabled) {Ok(())=>self.automation_changed(),Err(error)=>self.error=Some(error)}
                }
                ui.checkbox(&mut self.automation.snap,"Snap 1/16");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Zoom");ui.add(egui::Slider::new(&mut self.automation.zoom,32.0..=128.0).show_value(false));
                let empty=self.automation.sequence.lane(self.automation.target).is_none_or(|l|l.points.is_empty());
                if ui.add_enabled(empty,egui::Button::new("Crear ejemplo")).clicked(){self.automation_example();}
                if ui.button("Añadir en beat 1").clicked(){self.add_automation_point(0,self.automation_base(self.automation.target));}
            });
            let count=self.automation.sequence.lane(self.automation.target).map_or(0,|l|l.points.len());
            let value=self.automation.target.index().and_then(|i|snapshot.automation_values[i]);
            ui.label(RichText::new(format!("{count} / {MAX_POINTS} puntos · {} carriles activos · {}",snapshot.automation_values.iter().flatten().count(),value.map_or_else(||"Control manual".into(),|v|format!("Valor de curva {v:.3}")))).color(ACCENT).size(12.0));
            self.automation_canvas(ui,snapshot);
            if let Some(mut point)=self.automation.selected.and_then(|id|self.automation.sequence.get(self.automation.target,id)) {
                let original=point;let (lo,hi)=self.automation.target.range();
                ui.horizontal_wrapped(|ui| {
                    ui.label("Beat");let mut beat=point.tick as f64/TICKS_PER_BEAT as f64+1.0;
                    if ui.add(egui::DragValue::new(&mut beat).range(1.0..=65.0).speed(0.25)).changed(){point.tick=quantize((beat-1.0)*TICKS_PER_BEAT as f64,self.automation.snap).min(END_TICK);}
                    ui.label("Valor");ui.add(egui::Slider::new(&mut point.value,lo..=hi));
                    if ui.button("Borrar punto").clicked(){self.remove_automation_point();}
                });
                if point!=original && self.automation.selected.is_some(){self.edit_automation_point(point);}
            }
            ui.label(RichText::new("Doble clic: crear. Arrastra un punto: mover tiempo/valor. Supr: borrar. Editar detiene el transport; Leer curva sustituye valores manuales durante Play.").size(11.0).color(MUTED));
            ui.label(RichText::new(match self.automation.target {Target::TrackGain(_)=>"Volumen: 0 = silencio, 1 = 100%, 2 = 200%.",Target::TrackPan(_)=>"Panorama: -1 = izquierda, 0 = centro, 1 = derecha.",Target::DelayMix=>"Mezcla: 0 = señal seca, 1 = solo eco."}).size(11.0).color(MUTED));
            if self.automation.target==Target::DelayMix && !self.effects.delay_enabled {ui.label("Activa Delay en Efectos master para escuchar su mezcla automatizada.");}
        });
    }
    fn automation_canvas(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) -> egui::Rect {
        egui::ScrollArea::horizontal()
            .id_salt("automation-scroll")
            .show(ui, |ui| {
                let zoom = self.automation.zoom;
                let (lo, hi) = self.automation.target.range();
                let (canvas, response) = ui.allocate_exact_size(
                    egui::vec2(LEFT + 64.0 * zoom + 12.0, RULER + HEIGHT),
                    egui::Sense::click(),
                );
                let grid = egui::Rect::from_min_max(
                    canvas.min + egui::vec2(LEFT, RULER),
                    egui::pos2(canvas.left() + LEFT + 64.0 * zoom, canvas.bottom()),
                );
                ui.painter()
                    .rect_filled(canvas, 3.0, Color32::from_rgb(19, 19, 21));
                for division in 0..=256 {
                    let x = grid.left() + division as f32 * zoom / 4.0;
                    ui.painter().vline(
                        x,
                        canvas.y_range(),
                        egui::Stroke::new(
                            1.0,
                            if division % 16 == 0 {
                                Color32::from_rgb(74, 73, 81)
                            } else {
                                Color32::from_rgb(36, 36, 40)
                            },
                        ),
                    );
                    if division % 16 == 0 {
                        ui.painter().text(
                            egui::pos2(x + 3.0, canvas.top() + RULER / 2.0),
                            egui::Align2::LEFT_CENTER,
                            (division / 16 + 1).to_string(),
                            egui::FontId::monospace(11.0),
                            MUTED,
                        );
                    }
                }
                for step in 0..=4 {
                    let y = grid.bottom() - step as f32 / 4.0 * HEIGHT;
                    ui.painter().hline(
                        grid.x_range(),
                        y,
                        egui::Stroke::new(1.0, Color32::from_rgb(47, 47, 52)),
                    );
                    ui.painter().text(
                        egui::pos2(canvas.left() + 4.0, y),
                        egui::Align2::LEFT_CENTER,
                        format!("{:.2}", lo + (hi - lo) * step as f32 / 4.0),
                        egui::FontId::monospace(10.0),
                        MUTED,
                    );
                }
                let position = |p: Point| {
                    egui::pos2(
                        grid.left() + p.tick as f32 / TICKS_PER_BEAT as f32 * zoom,
                        grid.bottom() - (p.value - lo) / (hi - lo) * HEIGHT,
                    )
                };
                let points = self
                    .automation
                    .sequence
                    .lane(self.automation.target)
                    .map_or_else(Vec::new, |l| l.points.clone());
                if let Some(first) = points.first() {
                    let mut previous = egui::pos2(grid.left(), position(*first).y);
                    for point in &points {
                        let pos = position(*point);
                        ui.painter()
                            .line_segment([previous, pos], egui::Stroke::new(2.0, ACCENT));
                        previous = pos;
                    }
                    ui.painter().line_segment(
                        [previous, egui::pos2(grid.right(), previous.y)],
                        egui::Stroke::new(2.0, ACCENT),
                    );
                }
                if response.double_clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                    && grid.contains(pos)
                    && !points.iter().any(|p| position(*p).distance(pos) < 9.0)
                {
                    let tick = quantize(
                        f64::from((pos.x - grid.left()) / zoom) * TICKS_PER_BEAT as f64,
                        self.automation.snap,
                    )
                    .min(END_TICK);
                    let value = (lo + (grid.bottom() - pos.y) / HEIGHT * (hi - lo)).clamp(lo, hi);
                    self.add_automation_point(tick, value);
                }
                for point in points {
                    let pos = position(point);
                    let hit = egui::Rect::from_center_size(pos, egui::vec2(16.0, 16.0));
                    let response = ui.interact(
                        hit,
                        egui::Id::unique(("automation-point", point.id)),
                        egui::Sense::click_and_drag(),
                    );
                    ui.painter().circle_filled(
                        pos,
                        5.0,
                        if self.automation.selected == Some(point.id) {
                            Color32::WHITE
                        } else {
                            ACCENT
                        },
                    );
                    if response.clicked() {
                        self.automation.selected = Some(point.id);
                    }
                    if response.drag_started()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        self.automation.drag = Some(PointDrag {
                            original: point,
                            origin: ui.input(|i| i.pointer.press_origin()).unwrap_or(pointer),
                        });
                        self.automation.selected = Some(point.id);
                        self.output = None;
                    }
                    if response.dragged()
                        && let Some(drag) = self.automation.drag
                        && drag.original.id == point.id
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        let delta = pointer - drag.origin;
                        let updated = Point {
                            id: point.id,
                            tick: quantize(
                                drag.original.tick as f64
                                    + f64::from(delta.x / zoom) * TICKS_PER_BEAT as f64,
                                self.automation.snap,
                            )
                            .min(END_TICK),
                            value: (drag.original.value - delta.y / HEIGHT * (hi - lo))
                                .clamp(lo, hi),
                        };
                        if let Err(error) = self
                            .automation
                            .sequence
                            .replace(self.automation.target, updated)
                        {
                            self.error = Some(error);
                        }
                    }
                }
                if let Some(drag) = self.automation.drag
                    && !ui.input(|i| i.pointer.primary_down())
                {
                    self.automation.drag = None;
                    if self
                        .automation
                        .sequence
                        .get(self.automation.target, drag.original.id)
                        != Some(drag.original)
                    {
                        self.mark_dirty();
                    }
                    self.open_selected();
                }
                let x = grid.left() + snapshot.musical_ticks as f32 / TICKS_PER_BEAT as f32 * zoom;
                if x <= grid.right() {
                    ui.painter().vline(
                        x,
                        canvas.y_range(),
                        egui::Stroke::new(2.0, Color32::from_rgb(238, 192, 103)),
                    );
                }
                if !ui.ctx().text_edit_focused()
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
                {
                    self.remove_automation_point();
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
    fn editing_automation_marks_dirty_preserves_modes_and_restores_with_project() {
        let mut app = super::super::tests::offline_app();
        app.switch_view(View::Automation);
        assert!(app.timeline_playback);
        app.add_automation_point(0, 0.3);
        let id = app.automation.selected.unwrap();
        app.edit_automation_point(Point {
            id,
            tick: 480,
            value: 1.5,
        });
        assert!(app.project.dirty);
        let captured = app.capture_project();
        app.apply_project(jner_daw::project::Project::empty())
            .unwrap();
        assert!(app.automation.sequence.lanes().is_empty());
        app.apply_project(captured).unwrap();
        assert_eq!(app.view, View::Automation);
        assert!(app.timeline_playback);
        assert_eq!(
            app.automation
                .sequence
                .get(Target::TrackGain(0), id)
                .unwrap()
                .value,
            1.5
        );
    }

    #[test]
    fn pointer_events_create_move_and_delete_points_on_the_actual_canvas() {
        fn frame(
            app: &mut App,
            ctx: &egui::Context,
            time: f64,
            events: Vec<egui::Event>,
        ) -> egui::Rect {
            let mut canvas = egui::Rect::NOTHING;
            let mut out = ctx.run_ui(
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
                        canvas = app.automation_canvas(ui, Snapshot::default());
                    });
                },
            );
            out.textures_delta.clear();
            canvas
        }
        fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }
        }
        let ctx = egui::Context::default();
        let mut app = super::super::tests::offline_app();
        let canvas = frame(&mut app, &ctx, 0.0, vec![]);
        let p = canvas.min + egui::vec2(LEFT + 32.0, RULER + HEIGHT * 0.75);
        for (t, down) in [(0.1, true), (0.15, false), (0.2, true), (0.25, false)] {
            frame(
                &mut app,
                &ctx,
                t,
                vec![egui::Event::PointerMoved(p), button(p, down)],
            );
        }
        let id = app
            .automation
            .selected
            .expect("Double click must create a point");
        let point = app
            .automation
            .sequence
            .get(Target::TrackGain(0), id)
            .unwrap();
        assert_eq!(point.tick, 480);
        assert!((point.value - 0.5).abs() < 1e-5);
        frame(&mut app, &ctx, 0.7, vec![egui::Event::PointerMoved(p)]);
        frame(&mut app, &ctx, 0.75, vec![button(p, true)]);
        let moved = p + egui::vec2(32.0, -HEIGHT * 0.25);
        frame(&mut app, &ctx, 0.8, vec![egui::Event::PointerMoved(moved)]);
        frame(&mut app, &ctx, 0.85, vec![button(moved, false)]);
        let point = app
            .automation
            .sequence
            .get(Target::TrackGain(0), id)
            .unwrap();
        assert_eq!(point.tick, 960);
        assert!((point.value - 1.0).abs() < 1e-5);
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique(("automation-point", id))));
        frame(
            &mut app,
            &ctx,
            1.2,
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            app.automation
                .sequence
                .lane(Target::TrackGain(0))
                .unwrap()
                .points
                .is_empty()
        );
    }
}
