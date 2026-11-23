use super::*;
use jner_daw::synth::{MAX_VOICES, Waveform};
use std::num::NonZeroU32;

impl App {
    pub(super) fn set_synth(&mut self, state: SynthState) {
        if state == self.synth {
            return;
        }
        let rate = self.output.as_ref().map_or(48_000, |o| o.info.sample_rate);
        let prepared = NonZeroU32::new(rate)
            .ok_or_else(|| "Sample rate no válido".to_owned())
            .and_then(|r| state.prepare(r));
        match prepared {
            Ok(prepared) if self.send(Command::SetSynth(prepared)) => {
                self.synth = state;
                self.mark_dirty();
            }
            Ok(_) => {}
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn synth_controls(&mut self, ui: &mut egui::Ui) {
        let track = self.piano.track;
        egui::CollapsingHeader::new(format!("Sintetizador · Pista {} · {}",track+1,self.synth.tracks[track].waveform.name()))
            .id_salt("synth-panel").show(ui,|ui| {
                let mut state=self.synth;
                let patch=&mut state.tracks[track];
                ui.horizontal_wrapped(|ui| {
                    ui.label("Onda");
                    egui::ComboBox::from_id_salt("synth-wave").selected_text(patch.waveform.name()).show_ui(ui,|ui| {
                        for wave in Waveform::ALL {ui.selectable_value(&mut patch.waveform,wave,wave.name());}
                    });
                    ui.label("Pasa bajos");
                    ui.add(egui::Slider::new(&mut patch.cutoff_hz,20.0..=20_000.0).logarithmic(true).suffix(" Hz"));
                    if ui.button("Restablecer sonido").clicked() {*patch=Default::default();}
                });
                egui::Grid::new("adsr-controls").num_columns(2).show(ui,|ui| {
                    ui.horizontal(|ui| {ui.label("Attack");ui.add(egui::Slider::new(&mut patch.attack_ms,0.0..=5000.0).logarithmic(true).smallest_positive(1.0).suffix(" ms"));});
                    ui.horizontal(|ui| {ui.label("Decay");ui.add(egui::Slider::new(&mut patch.decay_ms,0.0..=5000.0).logarithmic(true).smallest_positive(1.0).suffix(" ms"));});
                    ui.end_row();
                    ui.horizontal(|ui| {ui.label("Sustain");ui.add(egui::Slider::new(&mut patch.sustain,0.0..=1.0));});
                    ui.horizontal(|ui| {ui.label("Release");ui.add(egui::Slider::new(&mut patch.release_ms,0.0..=5000.0).logarithmic(true).smallest_positive(1.0).suffix(" ms"));});
                    ui.end_row();
                });
                if state!=self.synth {self.set_synth(state);}
                ui.label(RichText::new(format!("{MAX_VOICES} voces compartidas entre pistas. Los cambios de sonido se aplican a las notas que empiecen después.")).size(11.0).color(MUTED));
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_edits_are_validated_saved_and_preserved_across_view_changes() {
        let mut app = super::super::tests::offline_app();
        let mut state = app.synth;
        state.tracks[2].waveform = Waveform::Saw;
        state.tracks[2].attack_ms = 300.0;
        app.set_synth(state);
        assert!(app.project.dirty);
        app.switch_view(View::PianoRoll);
        app.switch_view(View::Mixer);
        assert_eq!(app.synth, state);
        let captured = app.capture_project();
        app.apply_project(jner_daw::project::Project::empty())
            .unwrap();
        assert_ne!(app.synth, state);
        app.apply_project(captured).unwrap();
        assert_eq!(app.synth, state);
        let mut invalid = state;
        invalid.tracks[2].sustain = f32::NAN;
        app.set_synth(invalid);
        assert_eq!(app.synth, state);
        assert!(app.error.is_some());
    }
}
