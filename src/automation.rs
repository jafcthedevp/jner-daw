//! Bounded musical-time curves; editing/preparation stay outside the audio callback.
use crate::{mixer::MAX_TRACK_GAIN, sequencer::TRACKS, timeline::TICKS_PER_BEAT};
use serde::{Deserialize, Serialize};

pub const MAX_LANES: usize = TRACKS * 2 + 1;
pub const MAX_POINTS: usize = 32;
pub const END_TICK: u64 = 64 * TICKS_PER_BEAT;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    TrackGain(usize),
    TrackPan(usize),
    DelayMix,
}
impl Target {
    pub const ALL: [Self; MAX_LANES] = [
        Self::TrackGain(0),
        Self::TrackGain(1),
        Self::TrackGain(2),
        Self::TrackGain(3),
        Self::TrackPan(0),
        Self::TrackPan(1),
        Self::TrackPan(2),
        Self::TrackPan(3),
        Self::DelayMix,
    ];
    pub fn index(self) -> Option<usize> {
        match self {
            Self::TrackGain(t) if t < TRACKS => Some(t),
            Self::TrackPan(t) if t < TRACKS => Some(TRACKS + t),
            Self::DelayMix => Some(TRACKS * 2),
            _ => None,
        }
    }
    pub fn range(self) -> (f32, f32) {
        match self {
            Self::TrackGain(_) => (0.0, MAX_TRACK_GAIN),
            Self::TrackPan(_) => (-1.0, 1.0),
            Self::DelayMix => (0.0, 1.0),
        }
    }
    pub fn name(self) -> String {
        match self {
            Self::TrackGain(t) => format!("Volumen · Pista {}", t + 1),
            Self::TrackPan(t) => format!("Panorama · Pista {}", t + 1),
            Self::DelayMix => "Mezcla del delay".into(),
        }
    }
    fn valid_value(self, value: f32) -> bool {
        let (lo, hi) = self.range();
        self.index().is_some() && value.is_finite() && (lo..=hi).contains(&value)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub id: u32,
    pub tick: u64,
    pub value: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub target: Target,
    pub enabled: bool,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Automation {
    lanes: Vec<Lane>,
    next_id: u32,
}
impl Automation {
    pub fn from_lanes(lanes: Vec<Lane>) -> Result<Self, String> {
        if lanes.len() > MAX_LANES {
            return Err("Máximo de nueve carriles de automatización".into());
        }
        let mut ids = Vec::new();
        for (i, lane) in lanes.iter().enumerate() {
            if lane.target.index().is_none()
                || lanes[..i].iter().any(|l| l.target == lane.target)
                || lane.points.len() > MAX_POINTS
            {
                return Err("Carril duplicado, destino o número de puntos no válido".into());
            }
            for (j, p) in lane.points.iter().enumerate() {
                if p.id == 0
                    || ids.contains(&p.id)
                    || p.tick > END_TICK
                    || !lane.target.valid_value(p.value)
                    || (j > 0 && lane.points[j - 1].tick >= p.tick)
                {
                    return Err("ID, orden, posición o valor de automatización no válido".into());
                }
                ids.push(p.id);
            }
        }
        let next_id = ids.into_iter().max().unwrap_or(0);
        Ok(Self { lanes, next_id })
    }
    pub fn validate(&self) -> Result<(), String> {
        Self::from_lanes(self.lanes.clone()).map(|_| ())
    }
    pub fn lanes(&self) -> &[Lane] {
        &self.lanes
    }
    pub fn lane(&self, target: Target) -> Option<&Lane> {
        self.lanes.iter().find(|l| l.target == target)
    }
    pub fn get(&self, target: Target, id: u32) -> Option<Point> {
        self.lane(target)?
            .points
            .iter()
            .find(|p| p.id == id)
            .copied()
    }
    fn lane_mut(&mut self, target: Target) -> Result<&mut Lane, String> {
        if target.index().is_none() {
            return Err("Destino de automatización no válido".into());
        }
        let index = match self.lanes.iter().position(|l| l.target == target) {
            Some(i) => i,
            None => {
                self.lanes.push(Lane {
                    target,
                    enabled: true,
                    points: Vec::new(),
                });
                self.lanes.len() - 1
            }
        };
        Ok(&mut self.lanes[index])
    }
    pub fn set_enabled(&mut self, target: Target, enabled: bool) -> Result<(), String> {
        self.lane_mut(target)?.enabled = enabled;
        Ok(())
    }
    pub fn add(&mut self, target: Target, tick: u64, value: f32) -> Result<u32, String> {
        if tick > END_TICK || !target.valid_value(value) {
            return Err("Posición o valor de automatización no válido".into());
        }
        let id = self
            .next_id
            .checked_add(1)
            .ok_or("Se agotaron los IDs de automatización")?;
        let lane = self.lane_mut(target)?;
        if lane.points.len() >= MAX_POINTS {
            return Err("Máximo de 32 puntos por carril".into());
        }
        if lane.points.iter().any(|p| p.tick == tick) {
            return Err("Ya existe un punto en ese tick".into());
        }
        lane.points.push(Point { id, tick, value });
        lane.points.sort_by_key(|p| p.tick);
        self.next_id = id;
        Ok(id)
    }
    pub fn replace(&mut self, target: Target, point: Point) -> Result<(), String> {
        if point.tick > END_TICK || !target.valid_value(point.value) {
            return Err("Posición o valor de automatización no válido".into());
        }
        let lane = self
            .lanes
            .iter_mut()
            .find(|l| l.target == target)
            .ok_or("No existe el carril")?;
        if lane
            .points
            .iter()
            .any(|p| p.id != point.id && p.tick == point.tick)
        {
            return Err("Ya existe un punto en ese tick".into());
        }
        let previous = lane
            .points
            .iter_mut()
            .find(|p| p.id == point.id)
            .ok_or("No existe el punto")?;
        *previous = point;
        lane.points.sort_by_key(|p| p.tick);
        Ok(())
    }
    pub fn remove(&mut self, target: Target, id: u32) {
        if let Some(lane) = self.lanes.iter_mut().find(|l| l.target == target) {
            lane.points.retain(|p| p.id != id);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Values {
    pub gains: [Option<f32>; TRACKS],
    pub pans: [Option<f32>; TRACKS],
    pub delay_mix: Option<f32>,
}
impl Values {
    pub fn get(self, target: Target) -> Option<f32> {
        match target {
            Target::TrackGain(t) => self.gains.get(t).copied().flatten(),
            Target::TrackPan(t) => self.pans.get(t).copied().flatten(),
            Target::DelayMix => self.delay_mix,
        }
    }
    fn set(&mut self, target: Target, value: f32) {
        match target {
            Target::TrackGain(t) => self.gains[t] = Some(value),
            Target::TrackPan(t) => self.pans[t] = Some(value),
            Target::DelayMix => self.delay_mix = Some(value),
        }
    }
    pub fn count(self) -> usize {
        self.gains
            .iter()
            .chain(&self.pans)
            .filter(|v| v.is_some())
            .count()
            + usize::from(self.delay_mix.is_some())
    }
}
struct PreparedLane {
    target: Target,
    points: Vec<Point>,
    cursor: usize,
}
pub struct AutomationPlayer {
    lanes: Vec<PreparedLane>,
}
impl AutomationPlayer {
    pub fn prepare(automation: &Automation) -> Result<Self, String> {
        automation.validate()?;
        let lanes = automation
            .lanes
            .iter()
            .filter(|l| l.enabled && !l.points.is_empty())
            .map(|l| PreparedLane {
                target: l.target,
                points: l.points.clone(),
                cursor: 0,
            })
            .collect();
        Ok(Self { lanes })
    }
    pub fn reset(&mut self) {
        for lane in &mut self.lanes {
            lane.cursor = 0;
        }
    }
    /// Monotonic musical position within a run; reset before restarting or seeking backwards.
    pub fn evaluate(&mut self, tick: f64) -> Values {
        let mut values = Values::default();
        for lane in &mut self.lanes {
            while lane.cursor + 1 < lane.points.len()
                && lane.points[lane.cursor + 1].tick as f64 <= tick
            {
                lane.cursor += 1;
            }
            let a = lane.points[lane.cursor];
            let value = if tick <= a.tick as f64 {
                a.value
            } else if let Some(b) = lane.points.get(lane.cursor + 1) {
                let t = ((tick - a.tick as f64) / (b.tick - a.tick) as f64).clamp(0.0, 1.0) as f32;
                a.value + (b.value - a.value) * t
            } else {
                a.value
            };
            values.set(lane.target, value);
        }
        values
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn points_edit_sort_preserve_ids_and_reject_duplicates_without_mutation() {
        let mut a = Automation::default();
        let target = Target::TrackGain(0);
        let id = a.add(target, 960, 1.0).unwrap();
        let first = a.add(target, 0, 0.0).unwrap();
        assert_eq!(a.lane(target).unwrap().points[0].id, first);
        let before = a.clone();
        assert!(
            a.replace(
                target,
                Point {
                    id,
                    tick: 0,
                    value: 0.5
                }
            )
            .is_err()
        );
        assert_eq!(a, before);
        a.replace(
            target,
            Point {
                id,
                tick: 1440,
                value: 1.5,
            },
        )
        .unwrap();
        assert_eq!(a.get(target, id).unwrap().value, 1.5);
        a.remove(target, first);
        assert_eq!(a.lane(target).unwrap().points.len(), 1);
        assert!(a.add(Target::TrackPan(4), 0, 0.0).is_err());
        assert!(a.add(target, END_TICK + 1, 0.0).is_err());
        assert!(a.add(target, 240, f32::NAN).is_err());
    }
    #[test]
    fn interpolation_holds_endpoints_resets_and_skips_disabled_or_empty_lanes() {
        let mut a = Automation::default();
        let gain = Target::TrackGain(0);
        let pan = Target::TrackPan(1);
        a.add(gain, 240, 0.0).unwrap();
        a.add(gain, 1200, 2.0).unwrap();
        a.add(pan, 0, -1.0).unwrap();
        a.add(pan, 960, 1.0).unwrap();
        a.add(Target::DelayMix, 0, 0.7).unwrap();
        a.set_enabled(Target::DelayMix, false).unwrap();
        a.set_enabled(Target::TrackGain(2), true).unwrap();
        let mut player = AutomationPlayer::prepare(&a).unwrap();
        assert_eq!(player.evaluate(0.0).gains[0], Some(0.0));
        let v = player.evaluate(720.0);
        assert_eq!(v.gains[0], Some(1.0));
        assert_eq!(v.pans[1], Some(0.5));
        assert_eq!(v.count(), 2);
        assert!((player.evaluate(720.5).gains[0].unwrap() - 1.0010417).abs() < 1e-6);
        assert_eq!(player.evaluate(100_000.0).gains[0], Some(2.0));
        player.reset();
        assert_eq!(player.evaluate(0.0).pans[1], Some(-1.0));
    }
    #[test]
    fn invalid_documents_bounds_capacity_and_ids_are_rejected() {
        let mut a = Automation::default();
        for i in 0..MAX_POINTS {
            a.add(Target::DelayMix, i as u64 * 240, 0.5).unwrap();
        }
        assert!(a.add(Target::DelayMix, END_TICK, 0.5).is_err());
        let lane = a.lanes()[0].clone();
        assert!(Automation::from_lanes(vec![lane.clone(), lane.clone()]).is_err());
        let mut invalid = lane.clone();
        invalid.points[1].id = invalid.points[0].id;
        assert!(Automation::from_lanes(vec![invalid]).is_err());
        let mut invalid = lane;
        invalid.points[0].value = 1.01;
        assert!(Automation::from_lanes(vec![invalid]).is_err());
        assert!(a.add(Target::TrackPan(0), 0, -1.1).is_err());
        assert!(a.add(Target::TrackGain(0), 0, 2.1).is_err());
    }

    #[test]
    fn curves_follow_fractional_sample_clock_positions_and_tempo_changes() {
        use crate::{sequencer::Bpm, timeline::MusicalClock};
        use std::num::NonZeroU32;
        let mut a = Automation::default();
        a.add(Target::TrackGain(0), 0, 0.0).unwrap();
        a.add(Target::TrackGain(0), 1920, 2.0).unwrap();
        for rate in [44_100, 48_000, 96_000] {
            for tempo in [30, 137, 300] {
                let mut clock =
                    MusicalClock::new(NonZeroU32::new(rate).unwrap(), Bpm::new(tempo).unwrap());
                let mut p = AutomationPlayer::prepare(&a).unwrap();
                for frame in 0..20000 {
                    let expected = (frame as f64 * f64::from(tempo) * 960.0
                        / (f64::from(rate) * 60.0)
                        / 1920.0
                        * 2.0)
                        .min(2.0) as f32;
                    assert!(
                        (p.evaluate(clock.position()).gains[0].unwrap() - expected).abs() < 1e-6
                    );
                    clock.advance();
                }
                let before = clock.position();
                clock.set_bpm(Bpm::new(240).unwrap());
                clock.advance();
                assert!(
                    (clock.position() - before - 240.0 * 960.0 / (f64::from(rate) * 60.0)).abs()
                        < 1e-9
                );
            }
        }
    }
}
