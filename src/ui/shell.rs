use super::*;

pub(super) const BACKGROUND: Color32 = Color32::from_rgb(11, 11, 12);
pub(super) const SURFACE: Color32 = Color32::from_rgb(25, 25, 27);
pub(super) const BORDER: Color32 = Color32::from_rgb(49, 49, 52);
pub(super) const TEXT: Color32 = Color32::from_rgb(245, 244, 246);

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum Section {
    #[default]
    Editor,
    Audio,
    Export,
}

pub(super) fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .corner_radius(16)
        .inner_margin(18)
}

pub(super) fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = egui::Style::default();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.slider_width = 120.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    style.visuals.panel_fill = BACKGROUND;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = Color32::from_gray(16);
    style.visuals.faint_bg_color = Color32::from_gray(29);
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.weak_text_color = Some(MUTED);
    style.visuals.selection.bg_fill = Color32::from_rgb(88, 34, 49);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, TEXT);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.window_corner_radius = egui::CornerRadius::same(16);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(12);
    style.visuals.window_stroke = egui::Stroke::new(1.0, BORDER);
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    for (widget, fill) in [
        (&mut style.visuals.widgets.inactive, 40),
        (&mut style.visuals.widgets.hovered, 57),
        (&mut style.visuals.widgets.active, 72),
    ] {
        widget.bg_fill = Color32::from_gray(fill);
        widget.weak_bg_fill = Color32::from_gray(fill);
        widget.bg_stroke = egui::Stroke::NONE;
        widget.fg_stroke = egui::Stroke::new(1.0, TEXT);
        widget.corner_radius = egui::CornerRadius::same(10);
    }
    ctx.set_global_style(style);
}

fn nav_button(ui: &mut egui::Ui, label: &str, selected: bool, icon: usize) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 43.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Tab, ui.is_enabled(), selected, label)
    });
    let fill = if selected {
        Color32::from_gray(39)
    } else if response.hovered() {
        Color32::from_gray(26)
    } else {
        Color32::TRANSPARENT
    };
    let color = if selected { TEXT } else { MUTED };
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, fill);
    let c = rect.left_center() + egui::vec2(22.0, 0.0);
    let stroke = egui::Stroke::new(1.7, color);
    match icon {
        0 => {
            for y in [-5.0, 3.0] {
                for x in [-7.0, 1.0] {
                    painter.rect_stroke(
                        egui::Rect::from_min_size(c + egui::vec2(x, y), egui::vec2(5.0, 5.0)),
                        1.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        1 => {
            for (y, w) in [(-5.0, 14.0), (0.0, 9.0), (5.0, 12.0)] {
                painter.line_segment(
                    [c + egui::vec2(-7.0, y), c + egui::vec2(w - 7.0, y)],
                    stroke,
                );
            }
        }
        2 => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(16.0, 16.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            for x in [-3.0, 3.0] {
                painter.line_segment([c + egui::vec2(x, -6.0), c + egui::vec2(x, 6.0)], stroke);
            }
        }
        3 => {
            for (x, y) in [(-6.0, -3.0), (0.0, 4.0), (6.0, -1.0)] {
                painter.line_segment([c + egui::vec2(x, -8.0), c + egui::vec2(x, 8.0)], stroke);
                painter.circle_filled(c + egui::vec2(x, y), 2.5, color);
            }
        }
        4 => {
            painter.add(egui::Shape::line(
                vec![
                    c + egui::vec2(-8.0, 5.0),
                    c + egui::vec2(-2.0, -4.0),
                    c + egui::vec2(4.0, 3.0),
                    c + egui::vec2(9.0, -6.0),
                ],
                stroke,
            ));
        }
        5 => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(8.0, 13.0)),
                4.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [c + egui::vec2(-7.0, 1.0), c + egui::vec2(-7.0, 8.0)],
                stroke,
            );
            painter.line_segment([c + egui::vec2(7.0, 1.0), c + egui::vec2(7.0, 8.0)], stroke);
            painter.line_segment(
                [c + egui::vec2(-7.0, 8.0), c + egui::vec2(7.0, 8.0)],
                stroke,
            );
        }
        _ => {
            painter.line_segment(
                [c + egui::vec2(0.0, -8.0), c + egui::vec2(0.0, 5.0)],
                stroke,
            );
            painter.add(egui::Shape::line(
                vec![
                    c + egui::vec2(-5.0, 0.0),
                    c + egui::vec2(0.0, 5.0),
                    c + egui::vec2(5.0, 0.0),
                ],
                stroke,
            ));
            painter.line_segment(
                [c + egui::vec2(-7.0, 9.0), c + egui::vec2(7.0, 9.0)],
                stroke,
            );
        }
    }
    painter.text(
        rect.left_center() + egui::vec2(44.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.0),
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn round_transport(ui: &mut egui::Ui, play: bool, enabled: bool, label: &str) -> bool {
    ui.add_enabled_ui(enabled, |ui| {
        let size = if play { 48.0 } else { 36.0 };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
        let fill = if play {
            if response.hovered() {
                Color32::WHITE
            } else {
                TEXT
            }
        } else {
            Color32::from_gray(43)
        };
        let fill = if enabled {
            fill
        } else {
            fill.gamma_multiply(0.4)
        };
        ui.painter().circle_filled(rect.center(), size / 2.0, fill);
        let center = rect.center();
        let ink = if play { BACKGROUND } else { TEXT };
        if play {
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    center + egui::vec2(-4.0, -8.0),
                    center + egui::vec2(-4.0, 8.0),
                    center + egui::vec2(9.0, 0.0),
                ],
                ink,
                egui::Stroke::NONE,
            ));
        } else {
            ui.painter().rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(10.0, 10.0)),
                2.0,
                ink,
            );
        }
        response.on_hover_text(label).clicked()
    })
    .inner
}

impl App {
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 15.0, ACCENT);
            for (x, h) in [(-6.0, 5.0), (0.0, 9.0), (6.0, 3.0)] {
                ui.painter().line_segment(
                    [
                        r.center() + egui::vec2(x, -h),
                        r.center() + egui::vec2(x, h),
                    ],
                    egui::Stroke::new(2.5, TEXT),
                );
            }
            ui.label(RichText::new("Jner").size(25.0).strong());
        });
        ui.add_space(30.0);
        ui.label(RichText::new("ESTUDIO").size(10.0).color(MUTED));
        ui.add_space(4.0);
        ui.add_enabled_ui(self.editor_enabled() && !self.project_busy(), |ui| {
            for (view, label, icon) in [
                (View::Sequencer, "Secuenciador", 0),
                (View::Timeline, "Timeline", 1),
                (View::PianoRoll, "Piano roll", 2),
                (View::Mixer, "Mixer", 3),
                (View::Automation, "Automatización", 4),
            ] {
                if nav_button(
                    ui,
                    label,
                    self.section == Section::Editor && self.view == view,
                    icon,
                )
                .clicked()
                {
                    self.section = Section::Editor;
                    if self.view != view {
                        self.switch_view(view);
                    }
                }
            }
            ui.add_space(20.0);
            ui.separator();
            ui.add_space(12.0);
            if nav_button(ui, "Audio y captura", self.section == Section::Audio, 5).clicked() {
                self.section = Section::Audio;
            }
            if nav_button(ui, "Exportar audio", self.section == Section::Export, 6).clicked() {
                self.section = Section::Export;
            }
        });
        ui.add_space((ui.available_height() - 100.0).max(20.0));
        ui.label(RichText::new("TU SALIDA").size(10.0).color(MUTED));
        let name = self
            .selected
            .and_then(|i| self.devices.get(i))
            .map_or("Sin dispositivo", |d| d.name.as_str());
        ui.add(egui::Label::new(RichText::new(name).size(12.0)).truncate())
            .on_hover_text(name);
        if let Some(output) = &self.output {
            ui.label(
                RichText::new(format!(
                    "{} kHz · {} canales",
                    output.info.sample_rate as f32 / 1000.0,
                    output.info.channels
                ))
                .size(11.0)
                .color(MUTED),
            );
        }
    }

    fn player(&mut self, ui: &mut egui::Ui, snapshot: Snapshot) {
        ui.columns(3, |columns| {
            columns[0].add_space(5.0);
            let name = self
                .project
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or_else(|| "Sin t?tulo".into(), |n| n.to_string_lossy().into_owned());
            columns[0].add(egui::Label::new(RichText::new(name).strong()).truncate());
            let mode = if self.piano_playback {
                "Piano roll"
            } else if self.timeline_playback {
                "Timeline"
            } else {
                "Patr?n de 16 pasos"
            };
            columns[0].label(
                RichText::new(if snapshot.playing {
                    format!("Reproduciendo ? {mode}")
                } else {
                    mode.into()
                })
                .size(11.0)
                .color(if snapshot.playing { ACCENT } else { MUTED }),
            );
            columns[1].add_enabled_ui(self.editor_enabled(), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(((ui.available_width() - 220.0) / 2.0).max(0.0));
                    if round_transport(ui, false, self.output.is_some(), "Detener ? Esc") {
                        self.send(Command::Stop);
                    }
                    if round_transport(
                        ui,
                        true,
                        self.output.is_some() && !snapshot.playing,
                        "Reproducir ? Espacio",
                    ) {
                        self.send(Command::Play);
                    }
                    let seconds = self.output.as_ref().map_or(0.0, |o| {
                        snapshot.position as f64 / f64::from(o.info.sample_rate)
                    });
                    ui.label(
                        RichText::new(format!(
                            "{:02}:{:05.2}",
                            (seconds / 60.0) as u64,
                            seconds % 60.0
                        ))
                        .monospace()
                        .size(13.0),
                    );
                });
            });
            columns[2].add_enabled_ui(self.editor_enabled(), |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().slider_width =
                        (ui.available_width() - 220.0).clamp(60.0, 120.0);
                    let mut gain = self.gain * 100.0;
                    if ui
                        .add(egui::Slider::new(&mut gain, 0.0..=MAX_GAIN * 100.0).suffix(" %"))
                        .changed()
                        && self.send(Command::SetGain(gain / 100.0))
                    {
                        self.gain = gain / 100.0;
                        self.mark_dirty();
                    }
                    ui.label(RichText::new("Master").size(11.0).color(MUTED));
                    let mut bpm = self.bpm.get();
                    if ui
                        .add(
                            egui::DragValue::new(&mut bpm)
                                .range(Bpm::MIN..=Bpm::MAX)
                                .speed(1),
                        )
                        .changed()
                        && let Ok(tempo) = Bpm::new(bpm)
                        && self.send(Command::SetBpm(tempo))
                    {
                        self.bpm = tempo;
                        self.mark_dirty();
                    }
                    ui.label(RichText::new("BPM").size(11.0).color(MUTED));
                });
            });
        });
    }

    pub(super) fn workspace_ui(&mut self, root: &mut egui::Ui, snapshot: Snapshot) {
        egui::Panel::bottom("player")
            .resizable(false)
            .exact_size(86.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(31, 31, 33))
                    .inner_margin(egui::Margin::symmetric(24, 14))
                    .stroke(egui::Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| self.player(ui, snapshot));
        egui::Panel::left("navigation")
            .resizable(false)
            .exact_size(198.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(7, 7, 8))
                    .inner_margin(14),
            )
            .show(root, |ui| self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(24))
            .show(root, |ui| {
                let rect = ui.max_rect();
                let mut gradient = egui::Mesh::default();
                gradient.colored_vertex(rect.left_top(), Color32::from_rgb(29, 32, 28));
                gradient.colored_vertex(rect.right_top(), Color32::from_rgb(25, 25, 31));
                gradient.colored_vertex(egui::pos2(rect.right(), rect.top() + 240.0), BACKGROUND);
                gradient.colored_vertex(egui::pos2(rect.left(), rect.top() + 240.0), BACKGROUND);
                gradient.add_triangle(0, 1, 2);
                gradient.add_triangle(0, 2, 3);
                ui.painter().add(egui::Shape::mesh(gradient));
                self.project_toolbar(ui);
                ui.add_space(22.0);
                let (title, description) = match self.section {
                    Section::Audio => (
                        "Audio y captura",
                        "Elige tu salida. Captura tu próxima idea.",
                    ),
                    Section::Export => (
                        "Exportar audio",
                        "Tu sesión, lista para escuchar y compartir.",
                    ),
                    Section::Editor => match self.view {
                        View::Sequencer => ("Secuenciador", "Dale forma a tu próximo ritmo."),
                        View::Timeline => ("Timeline", "Cada sonido, en su lugar."),
                        View::PianoRoll => ("Piano roll", "De una nota a tu próxima melodía."),
                        View::Mixer => ("Mixer", "Encuentra el equilibrio de tu sesión."),
                        View::Automation => ("Automatización", "Pon tu sonido en movimiento."),
                    },
                };
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(20, 0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(title).size(30.0).strong());
                        ui.label(RichText::new(description).size(14.0).color(MUTED));
                    });
                ui.add_space(18.0);
                egui::ScrollArea::vertical()
                    .id_salt("workspace-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.recording.is_active() {
                            self.recording_controls(ui);
                        }
                        if self.export.is_active() {
                            self.export_controls(ui);
                        }
                        match self.section {
                            Section::Audio => {
                                ui.add_enabled_ui(self.editor_enabled(), |ui| {
                                    self.output_controls(ui)
                                });
                                ui.add_space(16.0);
                                if !self.recording.is_active() {
                                    card().show(ui, |ui| self.recording_controls(ui));
                                }
                            }
                            Section::Export => {
                                if !self.export.is_active() {
                                    card().show(ui, |ui| self.export_controls(ui));
                                }
                            }
                            Section::Editor => {
                                ui.add_enabled_ui(self.editor_enabled(), |ui| {
                                    match self.view {
                                        View::Sequencer => self.sequencer(ui, snapshot),
                                        View::Timeline => self.timeline_controls(ui, snapshot),
                                        View::PianoRoll => self.piano_controls(ui, snapshot),
                                        View::Mixer => self.mixer_controls(ui, snapshot),
                                        View::Automation => self.automation_controls(ui, snapshot),
                                    }
                                    ui.add_space(18.0);
                                    card().show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        self.effects_controls(ui);
                                    });
                                });
                            }
                        }
                        ui.add_space(16.0);
                        if let Some(error) = &self.error {
                            ui.colored_label(Color32::from_rgb(255, 153, 155), error);
                        }
                        if let Some(notice) = &self.notice {
                            ui.label(RichText::new(notice).size(12.0).color(MUTED));
                        }
                        ui.add_space(12.0);
                        ui.collapsing("Diagnóstico de audio", |ui| {
                            ui.monospace(format!(
                                "{} frames por buffer · {} errores CPAL · pico {:.3}",
                                snapshot.buffer_frames, snapshot.errors, snapshot.peak
                            ));
                        });
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_pointer_navigation_preserves_playback_mode_at_both_window_sizes() {
        fn frame(
            app: &mut App,
            ctx: &egui::Context,
            size: egui::Vec2,
            time: f64,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |root| app.workspace_ui(root, Snapshot::default()),
            );
            output.textures_delta.clear();
            output
        }
        fn center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
            output
                .shapes
                .iter()
                .find_map(|clipped| {
                    if let egui::Shape::Text(t) = &clipped.shape
                        && t.galley.job.text == label
                    {
                        Some(t.pos + t.galley.size() / 2.0)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("No se dibujó {label}"))
        }
        fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }
        }
        for size in [egui::vec2(940.0, 620.0), egui::vec2(1320.0, 860.0)] {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = super::super::tests::offline_app();
            let _ = frame(&mut app, &ctx, size, 0.0, vec![]);
            let initial = frame(&mut app, &ctx, size, 0.01, vec![]);
            let pos = center(&initial, "Mixer");
            let _ = frame(
                &mut app,
                &ctx,
                size,
                0.1,
                vec![egui::Event::PointerMoved(pos), button(pos, true)],
            );
            let mixer = frame(&mut app, &ctx, size, 0.2, vec![button(pos, false)]);
            assert_eq!(app.view, View::Mixer);
            assert!(app.timeline_playback && !app.piano_playback);
            assert!(!app.project.dirty);
            let export_pos = center(&mixer, "Exportar audio");
            let _ = frame(
                &mut app,
                &ctx,
                size,
                0.3,
                vec![
                    egui::Event::PointerMoved(export_pos),
                    button(export_pos, true),
                ],
            );
            let output = frame(&mut app, &ctx, size, 0.4, vec![button(export_pos, false)]);
            assert!(app.section == Section::Export);
            assert_eq!(app.view, View::Mixer);
            assert!(app.timeline_playback && !app.piano_playback);
            assert!(!app.project.dirty);
            let bpm = center(&output, "BPM");
            assert!(bpm.x > 0.0 && bpm.x < size.x && bpm.y > size.y - 86.0 && bpm.y < size.y);
        }
    }
}
