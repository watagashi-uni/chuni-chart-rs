// SPDX-License-Identifier: AGPL-3.0-only
// Marisa.Frontend Preview.vue: trimmed four-beat mean and greedy measure grouping.
use crate::chart::Chart;
#[derive(Debug)]
pub struct Layout {
    pub pps: f64,
    pub ranges: Vec<(f64, f64)>,
}
fn trimmed_mean(mut values: Vec<f64>) -> f64 {
    values.retain(|v| v.is_finite() && *v > 0.0);
    values.sort_by(f64::total_cmp);
    let cut = if values.len() > 10 { 5 } else { 0 };
    let vs = &values[cut..values.len() - cut];
    if vs.is_empty() {
        2.0
    } else {
        vs.iter().sum::<f64>() / vs.len() as f64
    }
}
pub fn layout(c: &Chart, zoom: f64) -> Layout {
    let mut splits: Vec<(f64, usize)> = Vec::new();
    for m in &c.measures {
        let beats: Vec<_> = c
            .beats
            .iter()
            .filter(|b| b.measure == m.id && !b.major)
            .collect();
        for b in beats.iter().skip(15).step_by(16) {
            splits.push((b.time, 16));
        }
    }
    splits.extend(c.measures.iter().map(|m| (m.time, m.numerator)));
    // Preserve upstream insertion order for its initial normalization calculation.
    // Ordinary (<17 beat) measures are already chronological.
    let normalized: Vec<_> = splits.iter().filter(|(_, n)| *n != 1).collect();
    let mut spans: Vec<_> = normalized
        .windows(2)
        .map(|a| (a[1].0 - a[0].0) / a[0].1 as f64 * 4.0)
        .collect();
    if let Some(last) = normalized.last() {
        spans.push(c.duration - last.0);
    }
    let pps = 700.0 / trimmed_mean(spans) * zoom;
    splits.sort_by(|a, b| a.0.total_cmp(&b.0));
    splits.dedup_by(|a, b| a.0 == b.0);
    let points: Vec<_> = splits
        .iter()
        .map(|s| s.0)
        .chain(std::iter::once(c.duration))
        .collect();
    let limit = points
        .windows(2)
        .map(|p| (p[1] - p[0]) * pps)
        .fold(2800.0 * zoom, f64::max)
        * 33.0
        / 32.0;
    let mut boundaries = vec![0.0];
    for i in 0..points.len().saturating_sub(2) {
        if (points[i + 2] - boundaries.last().unwrap()) * pps > limit {
            boundaries.push(points[i + 1]);
        }
    }
    boundaries.push(c.duration.max(0.1));
    let overflow = 50.0 * zoom / pps;
    Layout {
        pps,
        ranges: boundaries
            .windows(2)
            .map(|r| (r[0] - overflow, r[1] + overflow))
            .collect(),
    }
}
