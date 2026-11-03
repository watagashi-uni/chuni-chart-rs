// SPDX-License-Identifier: AGPL-3.0-only
// Static ground-note protection reconstructed from chusanApp.exe; see NOTICE.
// These are internal windows, not input ownership in a particular playthrough.
use crate::chart::{Note, mark_critical_notes};
use serde::Serialize;

const FIX: f64 = 2.0 / 60.0;
const SAME_BEAT: f64 = 1.0 / 192.0;

#[derive(Clone, Debug)]
pub struct Window {
    pub note: Note,
    /// Positive distance before the note, in seconds.
    pub early: [f64; 16],
    /// Negative distance after the note, retained for API compatibility.
    pub late: [f64; 16],
    /// Negative distance to the latest covered lane's miss boundary.
    pub miss: f64,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Band {
    pub grade: &'static str,
    pub from: f64,
    pub to: f64,
}

fn outer(note: &Note, easy: bool) -> f64 {
    // FLICK.TAP has a fixed +/-5-frame profile, including on EASY charts.
    (if easy && note.kind != "FLK" { 6.0 } else { 5.0 }) / 60.0
}

pub fn supported(input: &[Note]) -> bool {
    input.iter().filter(|n| n.ground()).all(|n| {
        n.time.is_finite()
            && n.tick.is_finite()
            && n.tick >= 0.0
            && n.lane >= 0.0
            && n.lane.fract() == 0.0
            && n.width > 0.0
            && n.width.fract() == 0.0
            && n.lane + n.width <= 16.0
    })
}

/// Internal windows for notes expressed at the usual C2S resolution of 384.
/// `tick` and `time` must describe the same chart position. For other resolutions,
/// use `protect_with_resolution`: the EXE's same-position tolerance is in beats.
pub fn protect(input: &[Note], easy: bool) -> Vec<Window> {
    protect_with_resolution(input, easy, 384.0)
}

/// Apply the default TAP/EXT/FLICK.TAP profiles and pair protection.
/// AIR, MINE, sustained judgements and FLICK.SLASH are outside this model.
pub fn protect_with_resolution(input: &[Note], easy: bool, resolution: f64) -> Vec<Window> {
    if !supported(input) || !resolution.is_finite() || resolution <= 0.0 {
        return Vec::new();
    }
    let mut notes = input.to_vec();
    mark_critical_notes(&mut notes);
    let mut ws: Vec<_> = notes
        .into_iter()
        .filter(Note::ground)
        .map(|note| {
            let max = outer(&note, easy);
            Window {
                note,
                early: [max; 16],
                late: [-max; 16],
                miss: -max,
            }
        })
        .collect();
    ws.sort_by(|a, b| a.note.time.total_cmp(&b.note.time));

    // 0x0119DBA0: candidates depend on ORIGINAL timestamps, never on another
    // note's shortened window. Previously tightened distances only shrink.
    let max_pair_gap = (if easy { 12.0 } else { 10.0 }) / 60.0;
    for j in 0..ws.len() {
        let (left, right) = ws.split_at_mut(j);
        let b = &mut right[0];
        for a in left.iter_mut().rev() {
            let dt = b.note.time - a.note.time;
            if dt >= max_pair_gap {
                break;
            }
            let beat_gap = (b.note.tick - a.note.tick) * 4.0 / resolution;
            if dt <= 0.0
                || beat_gap <= SAME_BEAT
                || dt >= outer(&a.note, easy) + outer(&b.note, easy)
            {
                continue;
            }
            let first = a.note.lane.max(b.note.lane) as usize;
            let last = (a.note.lane + a.note.width).min(b.note.lane + b.note.width) as usize;
            let distance = (dt / 2.0).max(FIX);
            for lane in first..last {
                a.late[lane] = a.late[lane].max(-distance);
                b.early[lane] = b.early[lane].min(distance);
            }
        }
    }
    // 0x0119DF70: MINIMALIZE runs after all pairs. TAP and FLICK.TAP propagate
    // the tightest side across the note; EXT retains per-lane notches.
    for w in &mut ws {
        let first = w.note.lane as usize;
        let last = (w.note.lane + w.note.width) as usize;
        if !w.note.critical() {
            let early = w.early[first..last]
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min);
            let late = w.late[first..last]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            w.early.fill(early);
            w.late.fill(late);
        }
        w.miss = w.late[first..last].iter().copied().fold(0.0, f64::min);
    }
    ws
}

pub fn bands(w: &Window, l: usize, easy: bool) -> Vec<Band> {
    let attack = outer(&w.note, easy);
    let lo = -attack.min(w.early[l]);
    let hi = -(-attack).max(w.late[l]).max(w.miss);
    let ranges: Vec<(&str, f64, f64)> = if w.note.kind == "FLK" {
        // Entry-touch eligibility only. Slash timing/movement is a separate stage.
        vec![("TOUCH", -attack, attack)]
    } else if w.note.critical() {
        vec![("JC", -attack, attack)]
    } else {
        vec![
            ("ATTACK", -attack, -4.0 / 60.0),
            ("JUSTICE", -4.0 / 60.0, -FIX),
            ("JC", -FIX, FIX),
            ("JUSTICE", FIX, 4.0 / 60.0),
            ("ATTACK", 4.0 / 60.0, attack),
        ]
    };
    ranges
        .into_iter()
        .filter_map(|(grade, a, b)| {
            let from = a.max(lo);
            let to = b.min(hi);
            (to > from).then_some(Band { grade, from, to })
        })
        .collect()
}
