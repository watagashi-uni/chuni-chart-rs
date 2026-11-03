// SPDX-License-Identifier: AGPL-3.0-only
use crate::{
    MAX_EVENTS, Result,
    chart::{Chart, geometry, num, tick},
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct ScrollSegment {
    pub tick: f64,
    pub end_tick: f64,
    pub time: f64,
    pub end_time: f64,
    pub velocity: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScrollProfile {
    pub id: u32,
    pub segments: Vec<ScrollSegment>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScrollArea {
    pub tick: f64,
    pub end_tick: f64,
    pub time: f64,
    pub end_time: f64,
    pub lane: f64,
    pub width: f64,
    pub profile_id: u32,
}

/// Static preview annotation, not a replacement for note times or positions.
/// Regional annotations show the selected program's playback timeline; `area`
/// identifies its separate note-selection scope in Chart::scroll_areas.
#[derive(Clone, Debug, Serialize)]
pub struct Scroll {
    pub time: f64,
    pub end_time: f64,
    pub lane: f64,
    pub width: f64,
    pub velocity: f64,
    pub global: bool,
    pub profile_id: u32,
    pub area: Option<usize>,
}

impl Chart {
    /// 0x011A1FD0: complete horizontal containment, half-open time scope,
    /// +1/192-beat position tolerance, and the greatest matching positive id.
    /// Default/global program 0 is used when no area matches. This selects a
    /// program for one note/node; it does not simulate the moving playfield.
    pub fn scroll_profile_at(&self, tick: f64, lane: f64, width: f64) -> u32 {
        if !tick.is_finite() || !lane.is_finite() || !width.is_finite() || width <= 0.0 {
            return 0;
        }
        let position = tick + self.resolution / 768.0;
        self.scroll_areas
            .iter()
            .filter(|a| {
                position >= a.tick
                    && position < a.end_tick
                    && lane >= a.lane
                    && lane + width <= a.lane + a.width
            })
            .map(|a| a.profile_id)
            .max()
            .unwrap_or(0)
    }
}

fn profile_id(parts: &[&str], field: usize) -> Result<u32> {
    let value = num(parts, field)?;
    if !(0.0..=i32::MAX as f64).contains(&value) || value.fract() != 0.0 {
        return Err("Invalid scroll profile id".into());
    }
    Ok(value as u32)
}

fn end_tick(c: &Chart, start: f64, duration: f64) -> Result<f64> {
    let end = start + duration;
    if !(0.0..=c.resolution * 100_000.0).contains(&duration)
        || !end.is_finite()
        || c.seconds_at(end) > 600.0
    {
        return Err("Invalid scroll duration".into());
    }
    Ok(end)
}

fn push(c: &mut Chart, scroll: Scroll) -> Result<()> {
    if c.notes.len() + c.longs.len() + c.scrolls.len() >= MAX_EVENTS {
        return Err("Too many chart events".into());
    }
    c.scrolls.push(scroll);
    Ok(())
}

pub(crate) fn parse(c: &mut Chart, lines: &[Vec<&str>]) -> Result<f64> {
    let mut profiles = BTreeMap::<u32, Vec<ScrollSegment>>::new();
    let mut records = 0;
    for p in lines {
        if !matches!(p[0], "STP" | "SFL" | "SLP" | "SLA" | "DCM") {
            continue;
        }
        records += 1;
        if records > MAX_EVENTS {
            return Err("Too many scroll events".into());
        }
        let start = tick(p, c.resolution)?;
        if p[0] == "SLA" {
            let lane = num(p, 3)?;
            let width = num(p, 4)?;
            geometry(lane, width)?;
            let end = end_tick(c, start, num(p, 5)?)?;
            c.scroll_areas.push(ScrollArea {
                tick: start,
                end_tick: end,
                time: c.seconds_at(start),
                end_time: c.seconds_at(end),
                lane,
                width,
                profile_id: profile_id(p, 6)?,
            });
        } else {
            // 0x011A8B60 passes SFL and SLP into the same map insertion:
            // SFL hardcodes id 0; SLP reads it from field 5.
            let id = if p[0] == "SLP" { profile_id(p, 5)? } else { 0 };
            let end = end_tick(c, start, num(p, 3)?)?;
            // STP is the legacy zero-speed program. DCM has the same four
            // numeric fields as SFL but is stored independently by the game.
            // These values are annotations; their magnitude does not expand
            // the timeline or allocate pixels. num() rejects NaN/infinity.
            let velocity = if p[0] == "STP" { 0.0 } else { num(p, 4)? };
            let segment = ScrollSegment {
                tick: start,
                end_tick: end,
                time: c.seconds_at(start),
                end_time: c.seconds_at(end),
                velocity,
            };
            if p[0] == "DCM" {
                c.distance_scrolls.push(segment);
            } else {
                profiles.entry(id).or_default().push(segment);
            }
        }
    }
    for segments in profiles.values_mut() {
        segments.sort_by(|a, b| a.tick.total_cmp(&b.tick));
    }
    let mut global_end: f64 = 0.0;
    c.distance_scrolls.sort_by(|a, b| a.tick.total_cmp(&b.tick));
    for s in &c.distance_scrolls {
        global_end = global_end.max(s.end_tick);
    }
    if let Some(global) = profiles.get(&0) {
        for s in global {
            global_end = global_end.max(s.end_tick);
            let mut at = s.tick;
            while at < s.end_tick {
                let stop = (((at / c.resolution).floor() + 1.0) * c.resolution).min(s.end_tick);
                push(
                    c,
                    Scroll {
                        time: c.seconds_at(at),
                        end_time: c.seconds_at(stop),
                        lane: 0.0,
                        width: 16.0,
                        velocity: s.velocity,
                        global: true,
                        profile_id: 0,
                        area: None,
                    },
                )?;
                at = stop;
            }
        }
    }
    for i in 0..c.scroll_areas.len() {
        let area = c.scroll_areas[i].clone();
        // Area id 0 does not override the default program, which is drawn once.
        if area.profile_id == 0 {
            continue;
        }
        if let Some(segments) = profiles.get(&area.profile_id) {
            let mut last: f64 = 0.0;
            for s in segments {
                last = last.max(s.end_tick);
                let end = s.end_tick.min(area.end_tick);
                if end > s.tick {
                    push(
                        c,
                        Scroll {
                            time: s.time,
                            end_time: c.seconds_at(end),
                            lane: area.lane,
                            width: area.width,
                            velocity: s.velocity,
                            global: false,
                            profile_id: area.profile_id,
                            area: Some(i),
                        },
                    )?;
                }
            }
            if last < area.end_tick {
                push(
                    c,
                    Scroll {
                        time: c.seconds_at(last),
                        end_time: area.end_time,
                        lane: area.lane,
                        width: area.width,
                        velocity: 1.0,
                        global: false,
                        profile_id: area.profile_id,
                        area: Some(i),
                    },
                )?;
            }
        }
    }
    c.scroll_profiles = profiles
        .into_iter()
        .map(|(id, segments)| ScrollProfile { id, segments })
        .collect();
    Ok(global_end)
}
