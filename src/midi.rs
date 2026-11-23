//! Internal MIDI notes and a bounded event scheduler; no instrument or external MIDI I/O.
use serde::{Deserialize, Serialize};

use crate::{
    sequencer::TRACKS,
    timeline::{SNAP_TICKS, TICKS_PER_BEAT},
};

pub const MAX_NOTES: usize = 256;
pub const END_TICK: u64 = 64 * TICKS_PER_BEAT;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub id: u32,
    pub track: usize,
    pub pitch: u8,
    pub velocity: u8,
    pub start_tick: u64,
    pub duration_tick: u64,
}

fn snapped(tick: f64, snap: bool) -> u64 {
    if !tick.is_finite() {
        return 0;
    }
    let step = if snap { SNAP_TICKS as f64 } else { 1.0 };
    ((tick / step).round() * step).clamp(0.0, END_TICK as f64) as u64
}

impl Note {
    fn valid(self) -> bool {
        self.id > 0
            && self.track < TRACKS
            && self.pitch <= 127
            && (1..=127).contains(&self.velocity)
            && self.duration_tick > 0
            && self.start_tick < END_TICK
            && self.duration_tick <= END_TICK - self.start_tick
    }

    pub fn moved(self, delta_ticks: f64, semitones: i16, snap: bool) -> Self {
        if !self.valid() || !delta_ticks.is_finite() {
            return self;
        }
        Self {
            start_tick: snapped(self.start_tick as f64 + delta_ticks, snap)
                .min(END_TICK - self.duration_tick),
            pitch: i16::from(self.pitch)
                .saturating_add(semitones)
                .clamp(0, 127) as u8,
            ..self
        }
    }

    pub fn resized(self, delta_ticks: f64, snap: bool) -> Self {
        if !self.valid() || !delta_ticks.is_finite() {
            return self;
        }
        let end = snapped(
            (self.start_tick + self.duration_tick) as f64 + delta_ticks,
            snap,
        );
        let minimum = if snap { SNAP_TICKS } else { 1 }.min(END_TICK - self.start_tick);
        Self {
            duration_tick: end.saturating_sub(self.start_tick).max(minimum),
            ..self
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteSequence {
    notes: Vec<Note>,
    next_id: u32,
}

impl NoteSequence {
    pub fn from_notes(notes: Vec<Note>) -> Result<Self, String> {
        if notes.len() > MAX_NOTES {
            return Err("Máximo de 256 notas por proyecto".into());
        }
        for (index, note) in notes.iter().enumerate() {
            if !note.valid() || notes[..index].iter().any(|n| n.id == note.id) {
                return Err("ID, pista, pitch, velocity o duración de nota no válidos".into());
            }
        }
        let next_id = notes.iter().map(|n| n.id).max().unwrap_or(0);
        Ok(Self { notes, next_id })
    }
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }
    pub fn get(&self, id: u32) -> Option<Note> {
        self.notes.iter().find(|n| n.id == id).copied()
    }
    pub fn add(
        &mut self,
        track: usize,
        pitch: u8,
        velocity: u8,
        start_tick: u64,
        duration_tick: u64,
    ) -> Result<u32, String> {
        if self.notes.len() >= MAX_NOTES {
            return Err("Máximo de 256 notas por proyecto".into());
        }
        let id = self
            .next_id
            .checked_add(1)
            .ok_or("Se agotaron los IDs de nota")?;
        let note = Note {
            id,
            track,
            pitch,
            velocity,
            start_tick,
            duration_tick,
        };
        if !note.valid() {
            return Err("Parámetros de nota no válidos".into());
        }
        self.notes.push(note);
        self.next_id = id;
        Ok(id)
    }
    pub fn replace(&mut self, note: Note) -> Result<(), String> {
        if !note.valid() {
            return Err("Parámetros de nota no válidos".into());
        }
        let previous = self
            .notes
            .iter_mut()
            .find(|n| n.id == note.id)
            .ok_or("No existe la nota")?;
        *previous = note;
        Ok(())
    }
    pub fn remove(&mut self, id: u32) {
        self.notes.retain(|n| n.id != id);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteEventKind {
    Off,
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteEvent {
    pub tick: u64,
    pub id: u32,
    pub track: usize,
    pub pitch: u8,
    pub velocity: u8,
    pub kind: NoteEventKind,
}

pub struct MidiPlayer {
    // Prepared and sorted before opening the stream. Never resized in the callback.
    events: Vec<NoteEvent>,
    cursor: usize,
    note_ons: u64,
    note_offs: u64,
    active: u32,
}

impl MidiPlayer {
    pub fn prepare(sequence: &NoteSequence) -> Self {
        let mut events = Vec::with_capacity(sequence.notes.len() * 2);
        for n in &sequence.notes {
            events.push(NoteEvent {
                tick: n.start_tick,
                id: n.id,
                track: n.track,
                pitch: n.pitch,
                velocity: n.velocity,
                kind: NoteEventKind::On,
            });
            events.push(NoteEvent {
                tick: n.start_tick + n.duration_tick,
                id: n.id,
                track: n.track,
                pitch: n.pitch,
                velocity: 0,
                kind: NoteEventKind::Off,
            });
        }
        events.sort_by_key(|e| (e.tick, matches!(e.kind, NoteEventKind::On), e.id));
        Self {
            events,
            cursor: 0,
            note_ons: 0,
            note_offs: 0,
            active: 0,
        }
    }
    pub fn reset(&mut self) {
        self.cursor = 0;
        self.note_ons = 0;
        self.note_offs = 0;
        self.active = 0;
    }
    pub fn counts(&self) -> (u64, u64, u32) {
        (self.note_ons, self.note_offs, self.active)
    }
    pub fn advance(&mut self, tick: u64, mut emit: impl FnMut(NoteEvent)) {
        // At most MAX_NOTES * 2 events per call, even after a large clock advance.
        while let Some(event) = self.events.get(self.cursor).copied() {
            if event.tick > tick {
                break;
            }
            match event.kind {
                NoteEventKind::On => {
                    self.note_ons += 1;
                    self.active += 1;
                }
                NoteEventKind::Off => {
                    self.note_offs += 1;
                    self.active = self.active.saturating_sub(1);
                }
            }
            emit(event);
            self.cursor += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sequencer::Bpm, timeline::MusicalClock};
    use std::num::NonZeroU32;

    #[test]
    fn editing_preserves_identity_velocity_and_respects_pitch_and_time_bounds() {
        let mut sequence = NoteSequence::default();
        let id = sequence.add(2, 60, 91, 960, 480).unwrap();
        let n = sequence.get(id).unwrap();
        let moved = n.moved(260.0, 3, true);
        assert_eq!(
            (moved.start_tick, moved.pitch, moved.velocity, moved.id),
            (1200, 63, 91, id)
        );
        assert_eq!(n.moved(-9999.0, -128, true).start_tick, 0);
        assert_eq!(n.moved(0.0, -128, true).pitch, 0);
        assert_eq!(n.moved(1e9, 128, true).start_tick, END_TICK - 480);
        assert_eq!(n.moved(0.0, 128, true).pitch, 127);
        assert_eq!(n.resized(-9999.0, true).duration_tick, 240);
        assert_eq!(n.resized(-9999.0, false).duration_tick, 1);
        assert_eq!(n.resized(1e9, true).duration_tick, END_TICK - 960);
        sequence.replace(moved).unwrap();
        assert_eq!(sequence.get(id), Some(moved));
        sequence.remove(id);
        assert!(sequence.notes().is_empty());
        assert!(sequence.add(0, 60, 80, 0, 240).unwrap() > id);
    }
    #[test]
    fn invalid_notes_ids_capacity_and_overflow_are_rejected_without_mutation() {
        let mut s = NoteSequence::default();
        for (track, pitch, velocity, start, duration) in [
            (4, 60, 80, 0, 1),
            (0, 128, 80, 0, 1),
            (0, 60, 0, 0, 1),
            (0, 60, 128, 0, 1),
            (0, 60, 80, END_TICK, 1),
            (0, 60, 80, 0, 0),
            (0, 60, 80, 1, u64::MAX),
        ] {
            assert!(s.add(track, pitch, velocity, start, duration).is_err());
        }
        assert!(s.notes().is_empty());
        for _ in 0..MAX_NOTES {
            s.add(0, 60, 80, 0, 240).unwrap();
        }
        assert!(s.add(0, 60, 80, 0, 240).is_err());
        let n = s.notes()[0];
        assert!(NoteSequence::from_notes(vec![n, n]).is_err());
        assert!(
            s.replace(Note {
                duration_tick: u64::MAX,
                ..n
            })
            .is_err()
        );
        assert_eq!(s.get(n.id), Some(n));
        let mut exhausted = NoteSequence::from_notes(vec![Note { id: u32::MAX, ..n }]).unwrap();
        assert!(exhausted.add(0, 60, 80, 0, 240).is_err());
    }
    #[test]
    fn note_on_off_timing_matches_sample_clock_at_different_rates_and_tempos() {
        for rate in [44_100, 48_000, 96_000] {
            for tempo in [30, 137, 300] {
                let mut s = NoteSequence::default();
                s.add(1, 60, 80, 0, 240).unwrap();
                s.add(1, 60, 100, 240, 240).unwrap();
                s.add(2, 67, 127, 240, 480).unwrap();
                let mut player = MidiPlayer::prepare(&s);
                let mut clock =
                    MusicalClock::new(NonZeroU32::new(rate).unwrap(), Bpm::new(tempo).unwrap());
                let mut actual = Vec::new();
                let last =
                    (720_u64 * u64::from(rate) * 60).div_ceil(u64::from(tempo) * TICKS_PER_BEAT);
                for frame in 0..=last {
                    player.advance(clock.ticks(), |e| actual.push((frame, e)));
                    clock.advance();
                }
                assert_eq!(actual.len(), 6);
                for (frame, e) in &actual {
                    assert_eq!(
                        *frame,
                        (e.tick * u64::from(rate) * 60).div_ceil(u64::from(tempo) * TICKS_PER_BEAT)
                    );
                }
                assert_eq!(actual[1].1.kind, NoteEventKind::Off);
                assert_eq!(actual[2].1.kind, NoteEventKind::On);
                assert_eq!(player.counts(), (3, 3, 0));
                player.advance(END_TICK, |_| panic!("Events repeated"));
                player.reset();
                player.advance(0, |_| {});
                assert_eq!(player.counts(), (1, 0, 1));
            }
        }
    }
}
