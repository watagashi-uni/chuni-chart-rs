// SPDX-License-Identifier: AGPL-3.0-only
use crate::{MAX_EVENTS, MAX_INPUT_BYTES, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    #[serde(rename = "type")]
    pub kind: String,
    pub tick: f64,
    pub time: f64,
    pub lane: f64,
    pub width: f64,
}
impl Note {
    pub fn ground(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "TAP" | "CHR" | "FLK" | "HLD_H" | "SLD_H"
        )
    }
    pub fn critical(&self) -> bool {
        self.kind == "CHR"
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct LongNote {
    pub head: Note,
    pub end_tick: f64,
    pub end_time: f64,
    pub end_lane: f64,
    pub end_width: f64,
    pub color: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Tempo {
    pub tick: f64,
    pub bpm: f64,
    pub time: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Beat {
    pub tick: f64,
    pub time: f64,
    pub measure: usize,
    pub major: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Measure {
    pub tick: f64,
    pub end_tick: f64,
    pub time: f64,
    pub end_time: f64,
    pub numerator: usize,
    pub id: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct Scroll {
    pub time: f64,
    pub end_time: f64,
    pub lane: f64,
    pub width: f64,
    pub velocity: f64,
    pub global: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Chart {
    pub resolution: f64,
    pub tempos: Vec<Tempo>,
    pub notes: Vec<Note>,
    pub longs: Vec<LongNote>,
    pub beats: Vec<Beat>,
    pub measures: Vec<Measure>,
    pub scrolls: Vec<Scroll>,
    pub duration: f64,
}
fn num(parts: &[&str], i: usize) -> Result<f64> {
    let n: f64 = parts
        .get(i)
        .ok_or("Missing numeric field")?
        .parse()
        .map_err(|_| "Invalid numeric field")?;
    if !n.is_finite() {
        return Err("Non-finite numeric field".into());
    }
    Ok(n)
}
fn tick(p: &[&str], resolution: f64) -> Result<f64> {
    let t = num(p, 1)? * resolution + num(p, 2)?;
    if !(0.0..=10_000_000.0).contains(&t) {
        return Err("Tick outside supported range".into());
    }
    Ok(t)
}
fn geometry(lane: f64, width: f64) -> Result<()> {
    if !lane.is_finite()
        || !width.is_finite()
        || lane < 0.0
        || width <= 0.0
        || lane + width > 16.000001
    {
        return Err("Invalid note geometry".into());
    }
    Ok(())
}
fn base(p: &[&str], resolution: f64) -> Result<Note> {
    let n = Note {
        kind: p[0].into(),
        tick: tick(p, resolution)?,
        time: 0.0,
        lane: num(p, 3)?,
        width: num(p, 4)?,
    };
    geometry(n.lane, n.width)?;
    Ok(n)
}
fn key(t: f64, x: f64, w: f64) -> (u64, u64, u64) {
    (t.to_bits(), x.to_bits(), w.to_bits())
}
impl Chart {
    pub fn seconds_at(&self, tick: f64) -> f64 {
        let i = self
            .tempos
            .partition_point(|b| b.tick <= tick)
            .saturating_sub(1);
        let b = &self.tempos[i];
        b.time + (tick - b.tick) * 240.0 / (self.resolution * b.bpm)
    }
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_INPUT_BYTES {
            return Err("Chart exceeds 2 MiB input limit".into());
        }
        let lines: Vec<Vec<&str>> = text
            .trim_start_matches('\u{feff}')
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim().starts_with("//"))
            .map(|l| l.split_whitespace().collect())
            .collect();
        if lines.len() > 50_000 {
            return Err("Too many input lines".into());
        }
        let resolution = match lines.iter().find(|p| p[0] == "RESOLUTION") {
            Some(p) => num(p, 1)?,
            None => 384.0,
        };
        if !(1.0..=1_000_000.0).contains(&resolution) {
            return Err("Invalid resolution".into());
        }
        // Fixed input ticks need not be integral; retain all decimal BPM precision.
        let mut tempo_rows = Vec::new();
        if let Some(p) = lines.iter().find(|p| p[0] == "BPM_DEF") {
            tempo_rows.push((0.0, num(p, 1)?));
        }
        for p in &lines {
            if p[0] == "BPM" {
                tempo_rows.push((tick(p, resolution)?, num(p, 3)?));
            }
        }
        if tempo_rows.iter().any(|(_, b)| *b <= 0.0 || *b > 10_000.0) {
            return Err("BPM outside supported range".into());
        }
        tempo_rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut tempos: Vec<Tempo> = Vec::new();
        for (t, b) in tempo_rows {
            if let Some(last) = tempos.last_mut()
                && last.tick == t
            {
                last.bpm = b;
                continue;
            }
            tempos.push(Tempo {
                tick: t,
                bpm: b,
                time: 0.0,
            });
        }
        if tempos.first().is_none_or(|t| t.tick != 0.0) {
            return Err("Missing initial BPM".into());
        }
        for i in 1..tempos.len() {
            tempos[i].time = tempos[i - 1].time
                + (tempos[i].tick - tempos[i - 1].tick) * 240.0 / (resolution * tempos[i - 1].bpm);
        }
        let mut c = Self {
            resolution,
            tempos,
            notes: Vec::new(),
            longs: Vec::new(),
            beats: Vec::new(),
            measures: Vec::new(),
            scrolls: Vec::new(),
            duration: 0.0,
        };
        for (line, p) in lines.iter().enumerate() {
            let parse_line = |c: &mut Self| -> Result<()> {
                match p[0] {
                    "TAP" | "CHR" | "FLK" | "MNE" | "AIR" | "AUR" | "AUL" | "ADW" | "ADR"
                    | "ADL" => c.notes.push(base(p, resolution)?),
                    "HLD" | "HXD" | "AHD" | "AHX" | "SLD" | "SLC" | "SXD" | "SXC" | "ASC"
                    | "ASD" | "ALD" => {
                        let head = base(p, resolution)?;
                        let k = p[0];
                        let hold = matches!(k, "HLD" | "HXD" | "AHD" | "AHX");
                        let air_hold = matches!(k, "AHD" | "AHX");
                        let air_path = matches!(k, "ASC" | "ASD" | "ALD");
                        let duration = num(
                            p,
                            if air_path {
                                7
                            } else if air_hold {
                                6
                            } else {
                                5
                            },
                        )?;
                        if duration < 0.0 {
                            return Err("Negative duration".into());
                        }
                        let end_tick = head.tick + duration;
                        let end_lane = if hold {
                            head.lane
                        } else {
                            num(p, if air_path { 8 } else { 6 })?
                        };
                        let end_width = if hold {
                            head.width
                        } else if air_path {
                            num(p, 9)?
                        } else if p.len() > 7 {
                            num(p, 7)?
                        } else {
                            head.width
                        };
                        geometry(end_lane, end_width)?;
                        let color = if air_path {
                            p.get(11).unwrap_or(&"DEF").to_string()
                        } else {
                            String::new()
                        };
                        let mut tail = head.clone();
                        tail.tick = end_tick;
                        tail.lane = end_lane;
                        tail.width = end_width;
                        if hold {
                            let mut h = head.clone();
                            h.kind = if air_hold {
                                "AHD_H"
                            } else if k == "HXD" {
                                "CHR"
                            } else {
                                "HLD_H"
                            }
                            .into();
                            if !air_hold || !matches!(p.get(5), Some(&"AHD" | &"AHX")) {
                                c.notes.push(h);
                            }
                            tail.kind = if air_hold { "AHD_T" } else { "HLD_T" }.into();
                            c.notes.push(tail);
                        } else if !air_path && k.ends_with('D') {
                            tail.kind = "SLD_T".into();
                            c.notes.push(tail);
                        } else if air_path {
                            if k == "ALD" {
                                let step = num(p, 5)?;
                                if step < 0.0 {
                                    return Err("Negative AIR interval".into());
                                }
                                if step > 0.0 && duration > 0.0 {
                                    let count = (duration / step).ceil();
                                    if count > MAX_EVENTS as f64 {
                                        return Err("Too many AIR nodes".into());
                                    }
                                    for i in 0..count as usize {
                                        let delta = i as f64 * step;
                                        let r = delta / duration;
                                        c.notes.push(Note {
                                            kind: "ALD_T".into(),
                                            tick: head.tick + delta,
                                            time: 0.0,
                                            lane: head.lane + (end_lane - head.lane) * r,
                                            width: head.width + (end_width - head.width) * r,
                                        });
                                    }
                                }
                            } else {
                                if !matches!(p.get(5), Some(&"ASC" | &"ASD" | &"AHD")) {
                                    let mut h = head.clone();
                                    h.kind = format!("{k}_H");
                                    c.notes.push(h);
                                }
                                if k == "ASD" {
                                    tail.kind = "ASD_T".into();
                                    c.notes.push(tail);
                                }
                            }
                        }
                        c.longs.push(LongNote {
                            head,
                            end_tick,
                            end_time: 0.0,
                            end_lane,
                            end_width,
                            color,
                        });
                    }
                    "VERSION" | "MUSIC" | "SEQUENCEID" | "DIFFICULT" | "LEVEL" | "CREATOR"
                    | "BPM" | "BPM_DEF" | "MET" | "MET_DEF" | "RESOLUTION" | "CLK_DEF"
                    | "PROGJUDGE_BPM" | "PROGJUDGE_AER" | "TUTORIAL" | "SFL" | "SLP" | "SLA" => {}
                    k if k.starts_with("T_") => {}
                    _ => return Err("Unsupported C2S record".into()),
                }
                if c.notes.len() + c.longs.len() > MAX_EVENTS {
                    return Err("Too many chart events".into());
                }
                Ok(())
            };
            parse_line(&mut c).map_err(|e| format!("Line {}: {e}", line + 1))?;
        }
        let slides: Vec<&LongNote> = c
            .longs
            .iter()
            .filter(|n| matches!(n.head.kind.as_str(), "SLD" | "SLC" | "SXD" | "SXC"))
            .collect();
        let ends: HashSet<_> = slides
            .iter()
            .map(|s| key(s.end_tick, s.end_lane, s.end_width))
            .collect();
        for s in slides {
            if !ends.contains(&key(s.head.tick, s.head.lane, s.head.width)) {
                let mut h = s.head.clone();
                h.kind = if h.kind.starts_with("SX") {
                    "CHR"
                } else {
                    "SLD_H"
                }
                .into();
                c.notes.push(h);
            }
        }
        if c.notes.len() + c.longs.len() > MAX_EVENTS {
            return Err("Too many chart events".into());
        }
        let mut seen = HashSet::new();
        c.notes
            .retain(|n| seen.insert((n.kind.clone(), key(n.tick, n.lane, n.width))));
        c.notes.sort_by(|a, b| a.tick.total_cmp(&b.tick));
        let times: Vec<_> = c.notes.iter().map(|n| c.seconds_at(n.tick)).collect();
        for (n, t) in c.notes.iter_mut().zip(times) {
            if n.ground() && (n.lane.fract() != 0.0 || n.width.fract() != 0.0) {
                return Err("Fractional ground-note geometry".into());
            }
            n.time = t;
        }
        let times: Vec<_> = c
            .longs
            .iter()
            .map(|n| (c.seconds_at(n.head.tick), c.seconds_at(n.end_tick)))
            .collect();
        for (n, (start, end)) in c.longs.iter_mut().zip(times) {
            n.head.time = start;
            n.end_time = end;
        }
        let mut scroll_end: f64 = 0.0;
        let mut lists: std::collections::HashMap<u64, Vec<(f64, f64, f64)>> =
            std::collections::HashMap::new();
        for p in &lines {
            if matches!(p[0], "SFL" | "SLP") {
                let start = tick(p, resolution)?;
                let duration = num(p, 3)?;
                let velocity = num(p, 4)?;
                if duration < 0.0 || duration > resolution * 100_000.0 || velocity.abs() > 100_000.0
                {
                    return Err("Invalid scroll event".into());
                }
                let end = start + duration;
                if p[0] == "SFL" {
                    scroll_end = scroll_end.max(end);
                    let mut at = start;
                    while at < end {
                        let stop = ((at / resolution).floor() + 1.0) * resolution;
                        let stop = stop.min(end);
                        c.scrolls.push(Scroll {
                            time: c.seconds_at(at),
                            end_time: c.seconds_at(stop),
                            lane: 0.0,
                            width: 16.0,
                            velocity,
                            global: true,
                        });
                        if c.scrolls.len() > MAX_EVENTS {
                            return Err("Too many scroll events".into());
                        }
                        at = stop;
                    }
                } else {
                    lists
                        .entry(num(p, 5)?.to_bits())
                        .or_default()
                        .push((start, end, velocity));
                }
            }
        }
        for p in &lines {
            if p[0] == "SLA" {
                let n = base(p, resolution)?;
                let duration = num(p, 5)?;
                if duration < 0.0 {
                    return Err("Negative scroll area duration".into());
                }
                let end = n.tick + duration;
                if let Some(list) = lists.get(&num(p, 6)?.to_bits()) {
                    let mut max: f64 = 0.0;
                    for &(a, b, v) in list {
                        max = max.max(b);
                        let b = b.min(end);
                        if b > a {
                            c.scrolls.push(Scroll {
                                time: c.seconds_at(a),
                                end_time: c.seconds_at(b),
                                lane: n.lane,
                                width: n.width,
                                velocity: v,
                                global: false,
                            });
                        }
                        if c.scrolls.len() > MAX_EVENTS {
                            return Err("Too many scroll events".into());
                        }
                    }
                    if max < end {
                        c.scrolls.push(Scroll {
                            time: c.seconds_at(max),
                            end_time: c.seconds_at(end),
                            lane: n.lane,
                            width: n.width,
                            velocity: 1.0,
                            global: false,
                        });
                    }
                }
            }
        }
        if c.notes.len() + c.longs.len() + c.scrolls.len() > MAX_EVENTS {
            return Err("Too many chart events".into());
        }
        if c.scrolls
            .iter()
            .any(|s| !s.end_time.is_finite() || s.end_time > 600.0)
        {
            return Err("Scroll duration exceeds 600 seconds".into());
        }
        let mut end_tick = c
            .notes
            .iter()
            .map(|n| n.tick)
            .chain(c.longs.iter().map(|n| n.end_tick))
            .fold(0.0, f64::max);
        end_tick = end_tick.max(scroll_end);
        for p in &lines {
            if matches!(p[0], "MET" | "BPM") {
                end_tick = end_tick.max(tick(p, resolution)?);
            }
        }
        c.duration = c.seconds_at(end_tick);
        if c.notes.is_empty() || !c.duration.is_finite() || c.duration > 600.0 {
            return Err("Empty chart or duration exceeds 600 seconds".into());
        }
        let def = lines.iter().find(|p| p[0] == "MET_DEF");
        let (a, b) = if let Some(p) = def {
            (num(p, 1)?, num(p, 2)?)
        } else {
            (4.0, 4.0)
        };
        let mut meters = BTreeMap::new();
        meters.insert(0_u64, (0.0, a, b));
        for p in &lines {
            if p[0] == "MET" {
                let numerator = num(p, 4)?;
                let denominator = num(p, 3)?;
                if numerator == 0.0 || denominator == 0.0 {
                    continue;
                }
                let t = tick(p, resolution)?;
                meters.insert(t.to_bits(), (t, numerator, denominator));
            }
        }
        let meters: Vec<_> = meters.values().copied().collect();
        let mut measure = 0;
        for (i, (t, n, d)) in meters.iter().copied().enumerate() {
            if !(1.0..=64.0).contains(&n)
                || !(1.0..=64.0).contains(&d)
                || n.fract() != 0.0
                || d.fract() != 0.0
            {
                return Err("Invalid time signature".into());
            }
            let stop = meters.get(i + 1).map_or(end_tick, |m| m.0).min(end_tick);
            let step = resolution / d;
            let mut start = t;
            while start < stop {
                let end = start + step * n;
                c.measures.push(Measure {
                    tick: start,
                    end_tick: end,
                    time: c.seconds_at(start),
                    end_time: c.seconds_at(end),
                    numerator: n as usize,
                    id: measure,
                });
                for b in 0..n as usize {
                    let tick = start + b as f64 * step;
                    if tick >= stop {
                        break;
                    }
                    c.beats.push(Beat {
                        tick,
                        time: c.seconds_at(tick),
                        measure,
                        major: b == 0,
                    });
                    if c.beats.len() > 50_000 {
                        return Err("Too many beat lines".into());
                    }
                }
                start += step * n;
                measure += 1;
            }
        }
        c.duration = c
            .measures
            .iter()
            .map(|m| m.end_time)
            .fold(c.duration, f64::max);
        if c.duration > 600.0 {
            return Err("Duration exceeds 600 seconds".into());
        }
        Ok(c)
    }
}
