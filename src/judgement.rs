// SPDX-License-Identifier: AGPL-3.0-only
// Independently named port of UMIGURI cw(); see NOTICE for reference revision.
use crate::chart::Note;
use serde::Serialize;
#[derive(Clone, Debug)]
pub struct Window {
    pub note: Note,
    pub early: [f64; 16],
    pub late: [f64; 16],
    pub miss: f64,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Band {
    pub grade: &'static str,
    pub from: f64,
    pub to: f64,
}
fn overlap(a: &Note, b: &Note) -> bool {
    a.lane < b.lane + b.width && b.lane < a.lane + a.width
}
fn inside(l: usize, a: &Note, b: &Note) -> bool {
    (l as f64) >= a.lane.max(b.lane) && (l as f64) < (a.lane + a.width).min(b.lane + b.width)
}
pub fn supported(input: &[Note]) -> bool {
    input
        .iter()
        .filter(|n| n.ground())
        .all(|n| n.lane >= 0.0 && n.lane + n.width <= 16.0)
}

// Off-lane geometry has no verified judgement model; only the ordinary renderer supports it.
pub fn protect(input: &[Note], easy: bool) -> Vec<Window> {
    if !supported(input) {
        return Vec::new();
    }
    let jc = 2.0 / 60.0;
    let attack: f64 = if easy { 6.0 } else { 5.0 } / 60.0;
    let mut ns: Vec<_> = input
        .iter()
        .filter(|n| n.ground())
        .map(|n| Window {
            note: n.clone(),
            early: [f64::INFINITY; 16],
            late: [f64::NEG_INFINITY; 16],
            miss: 0.0,
        })
        .collect();
    ns.sort_by(|a, b| a.note.time.total_cmp(&b.note.time));
    for i in 0..ns.len() {
        let (before, rest) = ns.split_at_mut(i);
        let b = &mut rest[0];
        if i == 0 {
            b.early.fill(attack);
            continue;
        }
        for a in before.iter() {
            if a.note.time >= b.note.time {
                break;
            }
            if !overlap(&a.note, &b.note) {
                continue;
            }
            let half = (b.note.time - a.note.time) / 2.0;
            for l in 0..16 {
                b.early[l] = b.early[l].min(if inside(l, &a.note, &b.note) {
                    half
                } else if b.note.critical() || b.note.kind == "FLK" {
                    attack
                } else {
                    half.max(jc)
                });
            }
        }
    }
    for i in 0..ns.len() {
        let (before, after) = ns.split_at_mut(i + 1);
        let a = &mut before[i];
        for b in after.iter().rev() {
            let dt = b.note.time - a.note.time;
            if dt <= 0.0 {
                break;
            }
            if !overlap(&a.note, &b.note) {
                continue;
            }
            if a.note.critical() {
                for l in 0..16 {
                    a.late[l] = a.late[l].max(if inside(l, &a.note, &b.note) {
                        -dt + b.early[l]
                    } else {
                        -attack
                    });
                }
            } else {
                let edge = a.late[a.note.lane as usize]
                    .max(-dt + b.early[a.note.lane.max(b.note.lane) as usize]);
                for l in 0..16 {
                    a.late[l] = if inside(l, &a.note, &b.note) {
                        edge.max(-dt + b.early[l])
                    } else {
                        a.late[l].max(edge.min(-jc))
                    };
                }
            }
        }
        for l in a.note.lane as usize..(a.note.lane + a.note.width) as usize {
            a.miss = a.miss.min(a.late[l].max(-attack));
        }
    }
    for i in 0..ns.len() {
        let (before, after) = ns.split_at_mut(i + 1);
        let a = &before[i];
        for b in after {
            let dt = b.note.time - a.note.time;
            if dt <= 0.0 || !overlap(&a.note, &b.note) {
                continue;
            }
            if b.note.critical() {
                for l in 0..16 {
                    if inside(l, &a.note, &b.note) {
                        b.early[l] = dt + a.late[l];
                    }
                }
            } else {
                let mut edge = f64::INFINITY;
                for l in 0..16 {
                    if inside(l, &a.note, &b.note) {
                        edge = edge.min(dt + a.late[l]);
                    }
                }
                for l in 0..16 {
                    b.early[l] = if inside(l, &a.note, &b.note) {
                        edge
                    } else {
                        edge.max(jc).min(b.early[l])
                    };
                }
            }
        }
    }
    ns
}
pub fn bands(w: &Window, l: usize, easy: bool) -> Vec<Band> {
    let attack: f64 = if easy { 6.0 } else { 5.0 } / 60.0;
    let lo = -attack.min(w.early[l]);
    let hi = -(-attack).max(w.late[l]).max(w.miss);
    let ranges: Vec<(&str, f64, f64)> = if w.note.kind == "FLK" {
        vec![("TOUCH", -5.0 / 60.0, 5.0 / 60.0)]
    } else if w.note.critical() {
        vec![("JC", -attack, attack)]
    } else {
        vec![
            ("ATTACK", -attack, -4.0 / 60.0),
            ("JUSTICE", -4.0 / 60.0, -2.0 / 60.0),
            ("JC", -2.0 / 60.0, 2.0 / 60.0),
            ("JUSTICE", 2.0 / 60.0, 4.0 / 60.0),
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
