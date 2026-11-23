use super::*;
use std::num::NonZeroU32;

impl App {
    pub(super) fn set_effects(&mut self, state: EffectsState) {
        if state == self.effects {
            return;
        }
        let rate = self.output.as_ref().map_or(48_000, |o| o.info.sample_rate);
        let prepared = NonZeroU32::new(rate)
            .ok_or_else(|| "Sample rate no válido".to_owned())
            .and_then(|rate| state.prepare(rate));
        match prepared {
            Ok(prepared) if self.send(Command::SetEffects(prepared)) => {
                self.effects = state;
                self.mark_dirty();
            }
            Ok(_) => {}
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn effects_controls(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Efectos master · Distorsión → Delay").id_salt("effects-panel").show(ui,|ui| {
            let mut state=self.effects;
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut state.distortion_enabled,"Distorsión");
                ui.label("Drive");ui.add(egui::Slider::new(&mut state.drive,1.0..=20.0).logarithmic(true).suffix(" ×"));
                ui.label("Mezcla");ui.add(egui::Slider::new(&mut state.distortion_mix,0.0..=1.0));
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut state.delay_enabled,"Delay");
                ui.label("Tiempo");ui.add(egui::Slider::new(&mut state.delay_ms,10.0..=1000.0).logarithmic(true).suffix(" ms"));
                ui.label("Feedback");ui.add(egui::Slider::new(&mut state.feedback,0.0..=0.85));
                ui.label("Mezcla");ui.add(egui::Slider::new(&mut state.delay_mix,0.0..=1.0));
            });
            if ui.button("Restablecer efectos").clicked() {state=EffectsState::default();}
            if state!=self.effects {self.set_effects(state);}
            ui.label(RichText::new("Actúan sobre la mezcla estéreo antes del master. Puedes ajustar mientras suena; Stop limpia el eco.").size(11.0).color(MUTED));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effect_edits_are_validated_persisted_and_preserved_when_changing_modes() {
        let mut app = super::super::tests::offline_app();
        let state = EffectsState {
            delay_enabled: true,
            delay_ms: 375.0,
            feedback: 0.6,
            distortion_enabled: true,
            drive: 8.0,
            ..Default::default()
        };
        app.set_effects(state);
        assert!(app.project.dirty);
        assert_eq!(app.effects, state);
        app.switch_view(View::Sequencer);
        app.switch_view(View::PianoRoll);
        assert_eq!(app.effects, state);
        let captured = app.capture_project();
        app.apply_project(jner_daw::project::Project::empty())
            .unwrap();
        assert_eq!(app.effects, EffectsState::default());
        app.apply_project(captured).unwrap();
        assert_eq!(app.effects, state);
        app.set_effects(EffectsState {
            feedback: f32::NAN,
            ..state
        });
        assert_eq!(app.effects, state);
        assert!(app.error.is_some());
    }
}
