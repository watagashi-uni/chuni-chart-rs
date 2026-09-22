// SPDX-License-Identifier: AGPL-3.0-only
use crate::{
    MAX_OUTPUT_BYTES, Result,
    chart::Chart,
    judgement::{bands, protect},
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub judge: bool,
    pub easy: bool,
    pub format: String,
    pub zoom: f64,
    pub column: Option<usize>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            judge: false,
            easy: false,
            format: "png".into(),
            zoom: 1.0,
            column: None,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
    pub pixels_per_second: f64,
    pub columns: usize,
}
const MAX_PIXELS: f64 = 12_000_000.0;
const STRIDE: f64 = 248.0;
const FOOT: f64 = 42.0;
type Color = [u8; 4];
fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    [r, g, b, a]
}
fn grade(g: &str) -> Color {
    match g {
        "JC" => rgba(255, 225, 94, 101),
        "JUSTICE" => rgba(255, 143, 72, 85),
        "ATTACK" => rgba(82, 218, 139, 69),
        _ => rgba(119, 186, 255, 68),
    }
}
fn sheet(c: &Chart, o: &Options) -> Result<(crate::layout::Layout, f64, f64)> {
    if !o.zoom.is_finite() || !(0.5..=2.0).contains(&o.zoom) {
        return Err("zoom must be between 0.5 and 2".into());
    }
    if !matches!(o.format.as_str(), "png" | "jpg" | "jpeg") {
        return Err("Unsupported output format".into());
    }
    let mut layout = crate::layout::layout(c, o.zoom);
    if let Some(column) = o.column {
        if column == 0 || column > layout.ranges.len() {
            return Err("Column outside chart".into());
        }
        layout.ranges = vec![layout.ranges[column - 1]];
    }
    let w = (48.0 + STRIDE * layout.ranges.len() as f64).max(720.0);
    let h = layout
        .ranges
        .iter()
        .map(|(a, b)| (b - a) * layout.pps)
        .fold(0.0, f64::max)
        + 88.0
        + FOOT
        + if o.judge { 38.0 } else { 0.0 };
    Ok((layout, w, h))
}
pub fn dimensions(c: &Chart, o: &Options) -> Result<Dimensions> {
    let (layout, w, h) = sheet(c, o)?;
    let scale = (MAX_PIXELS / (w * h))
        .sqrt()
        .min(1.0)
        .min(16384.0 / w)
        .min(16384.0 / h);
    Ok(Dimensions {
        width: (w * scale).floor() as u32,
        height: (h * scale).floor() as u32,
        pixels_per_second: layout.pps * scale,
        columns: layout.ranges.len(),
    })
}
struct Canvas {
    pix: Pixmap,
    scale: f32,
    font: fontdue::Font,
    blend: tiny_skia::BlendMode,
}
impl Canvas {
    fn path(&mut self, points: &[(f64, f64)], color: Color, stroke: Option<f32>, closed: bool) {
        if points.len() < 2 {
            return;
        }
        let mut p = PathBuilder::new();
        p.move_to(points[0].0 as f32, points[0].1 as f32);
        for &(x, y) in &points[1..] {
            p.line_to(x as f32, y as f32);
        }
        if closed {
            p.close();
        }
        if let Some(p) = p.finish() {
            let mut paint = Paint {
                blend_mode: self.blend,
                ..Default::default()
            };
            paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
            let tr = Transform::from_scale(self.scale, self.scale);
            if let Some(width) = stroke {
                self.pix.stroke_path(
                    &p,
                    &paint,
                    &Stroke {
                        width,
                        ..Default::default()
                    },
                    tr,
                    None,
                );
            } else {
                self.pix.fill_path(&p, &paint, FillRule::Winding, tr, None);
            }
        }
    }
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: Color) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.path(
            &[(x, y), (x + w, y), (x + w, y + h), (x, y + h)],
            color,
            None,
            true,
        );
    }
    fn text(&mut self, x: f64, y: f64, size: f32, text: &str, color: Color) {
        let s = self.scale;
        let mut px = x as f32 * s;
        let baseline = y as f32 * s;
        let (w, h) = (self.pix.width() as i32, self.pix.height() as i32);
        for ch in text.chars() {
            let (m, b) = self.font.rasterize(ch, size * s);
            let left = px.floor() as i32 + m.xmin;
            let top = baseline.floor() as i32 - m.ymin - m.height as i32;
            for row in 0..m.height {
                let yy = top + row as i32;
                if yy < 0 || yy >= h {
                    continue;
                }
                for col in 0..m.width {
                    let xx = left + col as i32;
                    if xx < 0 || xx >= w {
                        continue;
                    }
                    let a = b[row * m.width + col] as u32 * color[3] as u32 / 255;
                    let at = (yy as usize * w as usize + xx as usize) * 4;
                    let data = self.pix.data_mut();
                    for k in 0..3 {
                        data[at + k] =
                            ((color[k] as u32 * a + data[at + k] as u32 * (255 - a)) / 255) as u8;
                    }
                }
            }
            px += m.advance_width;
        }
    }
}
// Sutherland-Hodgman horizontal clipping: paths crossing columns never bleed.
fn clip_y(mut p: Vec<(f64, f64)>, lo: f64, hi: f64) -> Vec<(f64, f64)> {
    for (edge, lower) in [(lo, true), (hi, false)] {
        let mut out = Vec::new();
        if p.is_empty() {
            break;
        }
        let mut a = *p.last().unwrap();
        for &b in &p {
            let ia = if lower { a.1 >= edge } else { a.1 <= edge };
            let ib = if lower { b.1 >= edge } else { b.1 <= edge };
            if ia != ib {
                let r = (edge - a.1) / (b.1 - a.1);
                out.push((a.0 + (b.0 - a.0) * r, edge));
            }
            if ib {
                out.push(b);
            }
            a = b;
        }
        p = out;
    }
    p
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.0.len() + b.len() > MAX_OUTPUT_BYTES {
            return Err(std::io::Error::other("Encoded image too large"));
        }
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Canvas {
    #[allow(clippy::too_many_arguments)]
    fn rounded(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        r: f64,
        color: Color,
        top: f64,
        bottom: f64,
    ) {
        let r = r.min(w / 2.0).min(h / 2.0);
        let mut ps = Vec::new();
        for (cx, cy, start) in [
            (x + r, y + r, std::f64::consts::PI),
            (x + w - r, y + r, std::f64::consts::PI * 1.5),
            (x + w - r, y + h - r, 0.0),
            (x + r, y + h - r, std::f64::consts::PI / 2.0),
        ] {
            for i in 0..=4 {
                let a = start + i as f64 * std::f64::consts::PI / 8.0;
                ps.push((cx + r * a.cos(), cy + r * a.sin()));
            }
        }
        self.path(&clip_y(ps, top, bottom), color, None, true);
    }
    fn gradient(
        &mut self,
        points: &[(f64, f64)],
        yb: f64,
        yt: f64,
        unit_start: f64,
        unit_end: f64,
    ) {
        use tiny_skia::{GradientStop, LinearGradient, Point, SpreadMode};
        if points.len() < 3 || yb <= yt {
            return;
        }
        let color = |u: f64| {
            let t = if u < 0.15 {
                1.0 - u / 0.15
            } else if u > 0.85 {
                (u - 0.85) / 0.15
            } else {
                0.0
            };
            tiny_skia::Color::from_rgba8(
                (253.0 * t) as u8,
                (255.0 - 163.0 * t) as u8,
                (255.0 - 10.0 * t) as u8,
                230,
            )
        };
        let mut stops = vec![GradientStop::new(0.0, color(unit_end))];
        for u in [0.85, 0.15] {
            if u > unit_start && u < unit_end {
                stops.push(GradientStop::new(
                    ((unit_end - u) / (unit_end - unit_start)) as f32,
                    color(u),
                ));
            }
        }
        stops.push(GradientStop::new(1.0, color(unit_start)));
        let Some(shader) = LinearGradient::new(
            Point::from_xy(0.0, yt as f32),
            Point::from_xy(0.0, yb as f32),
            stops,
            SpreadMode::Pad,
            Transform::identity(),
        ) else {
            return;
        };
        let mut p = PathBuilder::new();
        p.move_to(points[0].0 as f32, points[0].1 as f32);
        for &(x, y) in &points[1..] {
            p.line_to(x as f32, y as f32);
        }
        p.close();
        if let Some(p) = p.finish() {
            let paint = Paint {
                shader,
                blend_mode: tiny_skia::BlendMode::Lighten,
                ..Default::default()
            };
            self.pix.fill_path(
                &p,
                &paint,
                FillRule::Winding,
                Transform::from_scale(self.scale, self.scale),
                None,
            );
        }
    }
}
fn slide_units(c: &Chart) -> Vec<(f64, f64)> {
    use std::collections::{HashMap, HashSet, VecDeque};
    let mut indices: Vec<_> = c
        .longs
        .iter()
        .enumerate()
        .filter(|(_, n)| n.head.kind.starts_with('S'))
        .map(|(i, _)| i)
        .collect();
    indices.sort_by_key(|i| match c.longs[*i].head.kind.as_str() {
        "SLD" => 0,
        "SLC" => 1,
        "SXD" => 2,
        _ => 3,
    });
    let key = |t: f64, x: f64, w: f64| (t.to_bits(), x.to_bits(), w.to_bits());
    let mut begins: HashMap<_, VecDeque<usize>> = HashMap::new();
    let mut ends = HashSet::new();
    for &i in &indices {
        let n = &c.longs[i];
        begins
            .entry(key(n.head.tick, n.head.lane, n.head.width))
            .or_default()
            .push_back(i);
        ends.insert(key(n.end_tick, n.end_lane, n.end_width));
    }
    let mut heads: Vec<_> = indices
        .iter()
        .copied()
        .filter(|&i| {
            let n = &c.longs[i];
            !ends.contains(&key(n.head.tick, n.head.lane, n.head.width))
        })
        .collect();
    for &i in &indices {
        let n = &c.longs[i];
        if n.head.kind.ends_with('D')
            && let Some(next) = begins.get(&key(n.end_tick, n.end_lane, n.end_width))
        {
            for &j in next {
                if !heads.contains(&j) {
                    heads.push(j);
                }
            }
        }
    }
    let head_set: HashSet<_> = heads.iter().copied().collect();
    let mut units = vec![(0.0, 1.0); c.longs.len()];
    for head in heads {
        let mut chain = Vec::new();
        let mut current = Some(head);
        let mut visited = HashSet::new();
        while let Some(i) = current {
            if !visited.insert(i) {
                break;
            }
            chain.push(i);
            let n = &c.longs[i];
            current = begins
                .get_mut(&key(n.end_tick, n.end_lane, n.end_width))
                .and_then(VecDeque::pop_front);
            if current.is_some_and(|i| head_set.contains(&i)) {
                break;
            }
        }
        let start = chain
            .iter()
            .map(|&i| c.longs[i].head.tick)
            .fold(f64::INFINITY, f64::min);
        let end = chain
            .iter()
            .map(|&i| c.longs[i].end_tick)
            .fold(start, f64::max);
        if end > start {
            for i in chain {
                let n = &c.longs[i];
                units[i] = (
                    (n.head.tick - start) / (end - start),
                    (n.end_tick - start) / (end - start),
                );
            }
        }
    }
    units
}
fn divisions(c: &Chart) -> Vec<(f64, String)> {
    let mut ticks: Vec<_> = c
        .notes
        .iter()
        .filter(|n| n.kind != "SLD_T")
        .map(|n| n.tick)
        .collect();
    ticks.sort_by(f64::total_cmp);
    ticks.dedup();
    let mut kept = Vec::new();
    for (i, &t) in ticks.iter().enumerate() {
        let mi = c
            .measures
            .partition_point(|m| m.tick <= t)
            .saturating_sub(1);
        if i > 0
            && t - ticks[i - 1] == 1.0
            && c.measures
                .partition_point(|m| m.tick <= ticks[i - 1])
                .saturating_sub(1)
                == mi
        {
            continue;
        }
        kept.push((t, mi));
    }
    kept.iter()
        .enumerate()
        .filter_map(|(i, &(t, mi))| {
            let next = kept.get(i + 1);
            let gap = match next {
                Some(&(next, nmi)) if nmi == mi => next - t,
                Some(_) => c.measures.get(mi)?.end_tick - t,
                None => c.resolution,
            };
            if gap <= 0.0 || gap.fract() != 0.0 || c.resolution.fract() != 0.0 {
                return None;
            }
            let (mut a, mut b) = (gap as u64, c.resolution as u64);
            while b != 0 {
                (a, b) = (b, a % b);
            }
            Some((
                c.seconds_at(t),
                format!("{}/{}", gap as u64 / a, c.resolution as u64 / a),
            ))
        })
        .collect()
}
fn air_color(k: &str, color: &str) -> Color {
    let a = if k == "ALD" { 255 } else { 153 };
    match color {
        "GRY" => rgba(156, 163, 175, a),
        "NON" => rgba(0, 0, 0, 0),
        "AQA" => rgba(0, 255, 255, a),
        "CYN" => rgba(0, 180, 180, a),
        "BLK" => rgba(0, 0, 0, a),
        "BLU" => rgba(0, 0, 255, a),
        "DGR" => rgba(8, 174, 226, a),
        "GRN" => rgba(0, 128, 0, a),
        "LIM" => rgba(50, 205, 50, a),
        "ORG" | "ORN" => rgba(255, 165, 0, a),
        "PNK" => rgba(255, 192, 203, a),
        "PPL" => rgba(128, 0, 128, a),
        "RED" => rgba(255, 0, 0, a),
        "VLT" => rgba(238, 130, 238, a),
        "YEL" => rgba(255, 255, 0, a),
        _ => {
            if k == "ALD" {
                rgba(236, 72, 153, 255)
            } else {
                rgba(22, 101, 52, 153)
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn long_path(
    v: &mut Canvas,
    n: &crate::chart::LongNote,
    x: f64,
    y: &impl Fn(f64) -> f64,
    top: f64,
    bottom: f64,
    unit: (f64, f64),
) {
    let air = n.head.kind.starts_with('A');
    let hold = matches!(n.head.kind.as_str(), "HLD" | "HXD");
    let polygon = clip_y(
        vec![
            (x + n.head.lane * 10.0, y(n.head.time)),
            (x + (n.head.lane + n.head.width) * 10.0, y(n.head.time)),
            (x + (n.end_lane + n.end_width) * 10.0, y(n.end_time)),
            (x + n.end_lane * 10.0, y(n.end_time)),
        ],
        top,
        bottom,
    );
    if hold {
        v.blend = tiny_skia::BlendMode::Lighten;
        v.path(&polygon, rgba(253, 186, 116, 204), None, true);
        v.blend = tiny_skia::BlendMode::SourceOver;
    } else {
        if air {
            if n.head.kind != "ALD" {
                v.path(&polygon, rgba(74, 222, 128, 13), None, true);
            }
        } else {
            v.gradient(&polygon, y(n.head.time), y(n.end_time), unit.0, unit.1);
        }
        let middle = clip_y(
            vec![
                (
                    x + (n.head.lane + n.head.width / 2.0) * 10.0 - 3.5,
                    y(n.head.time),
                ),
                (
                    x + (n.head.lane + n.head.width / 2.0) * 10.0 + 3.5,
                    y(n.head.time),
                ),
                (
                    x + (n.end_lane + n.end_width / 2.0) * 10.0 + 3.5,
                    y(n.end_time),
                ),
                (
                    x + (n.end_lane + n.end_width / 2.0) * 10.0 - 3.5,
                    y(n.end_time),
                ),
            ],
            top,
            bottom,
        );
        if !air {
            v.blend = tiny_skia::BlendMode::Lighten;
        }
        v.path(
            &middle,
            if air {
                air_color(&n.head.kind, &n.color)
            } else {
                rgba(0, 255, 255, 255)
            },
            None,
            true,
        );
        v.blend = tiny_skia::BlendMode::SourceOver;
    }
}
fn glyph(v: &mut Canvas, n: &crate::chart::Note, x: f64, yy: f64, top: f64, bottom: f64) {
    let xx = x + n.lane * 10.0 + 1.0;
    let ww = n.width * 10.0 - 1.0;
    let color = match n.kind.as_str() {
        "CHR" => rgba(250, 204, 21, 255),
        "TAP" => rgba(248, 113, 113, 255),
        "HLD_H" | "HLD_T" => rgba(251, 146, 60, 255),
        "SLD_H" | "SLD_T" => rgba(38, 93, 243, 255),
        "FLK" => rgba(147, 197, 253, 255),
        "MNE" => rgba(29, 78, 216, 255),
        _ => rgba(192, 132, 252, 255),
    };
    if matches!(
        n.kind.as_str(),
        "AIR" | "AUR" | "AUL" | "ADW" | "ADR" | "ADL" | "AHD_H" | "ASD_H" | "ASC_H"
    ) {
        let down = matches!(n.kind.as_str(), "ADW" | "ADR" | "ADL");
        let skew = match n.kind.as_str() {
            "AUR" | "ADL" => -0.7002075,
            "AUL" | "ADR" => 0.7002075,
            _ => 0.0,
        };
        let transform = |p: Vec<(f64, f64)>| {
            clip_y(
                p.into_iter()
                    .map(|(a, b)| (xx + a * ww + (b - 1.0) * 15.0 * skew, yy - 15.0 + b * 15.0))
                    .collect(),
                top,
                bottom,
            )
        };
        v.path(
            &transform(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
            if down {
                rgba(236, 72, 153, 51)
            } else {
                rgba(74, 222, 128, 51)
            },
            None,
            true,
        );
        let arrow = if down {
            vec![(0.2, 0.0), (0.8, 0.0), (0.8, 0.6), (0.5, 1.0), (0.2, 0.6)]
        } else {
            vec![(0.2, 1.0), (0.8, 1.0), (0.8, 0.4), (0.5, 0.0), (0.2, 0.4)]
        };
        v.path(
            &transform(arrow),
            if down {
                rgba(244, 114, 182, 255)
            } else {
                rgba(134, 239, 172, 255)
            },
            None,
            true,
        );
    } else {
        let h = if n.kind.starts_with('A') { 4.0 } else { 8.0 };
        v.rounded(xx, yy - h / 2.0 - 1.0, ww, h, 4.0, color, top, bottom);
        if matches!(n.kind.as_str(), "HLD_H" | "SLD_H") && yy >= top && yy < bottom {
            v.rounded(
                xx + 3.0,
                yy - 1.0,
                ww - 6.0,
                1.0,
                0.5,
                if n.kind == "HLD_H" {
                    rgba(246, 195, 147, 255)
                } else {
                    rgba(98, 137, 244, 255)
                },
                top,
                bottom,
            );
        }
    }
}
fn scroll_color(v: f64) -> Color {
    let stops = [-10.0, -1.0, 0.0, 1.0, 3.0, 100.0, 500.0];
    let colors = [
        [255.0, 0.0, 242.0],
        [255.0, 130.0, 249.0],
        [0.0, 0.0, 255.0],
        [107.0, 114.0, 128.0],
        [255.0, 0.0, 0.0],
        [82.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
    ];
    let i = stops.partition_point(|x| *x <= v).saturating_sub(1).min(5);
    let r = ((v - stops[i]) / (stops[i + 1] - stops[i])).clamp(0.0, 1.0);
    let mut color = [0, 0, 0, 255];
    for (k, c) in color.iter_mut().enumerate().take(3) {
        let a: f64 = colors[i][k];
        let b: f64 = colors[i + 1][k];
        *c = (a.powf(2.2) * (1.0 - r) + b.powf(2.2) * r)
            .powf(1.0 / 2.2)
            .round() as u8;
    }
    color
}
pub fn render(c: &Chart, o: &Options) -> Result<Vec<u8>> {
    let d = dimensions(c, o)?;
    let (layout, w, h) = sheet(c, o)?;
    let scale = d.pixels_per_second / layout.pps;
    let pps = layout.pps;
    let bottom = h - FOOT - 44.0;
    let mut v = Canvas {
        pix: Pixmap::new(d.width, d.height).ok_or("Image allocation failed")?,
        scale: scale as f32,
        font: fontdue::Font::from_bytes(
            include_bytes!("../assets/DejaVuSans.ttf") as &[u8],
            fontdue::FontSettings::default(),
        )
        .map_err(|_| "Font load failed")?,
        blend: tiny_skia::BlendMode::SourceOver,
    };
    v.pix.fill(tiny_skia::Color::from_rgba8(107, 114, 128, 255));
    if o.judge {
        v.rect(0.0, 0.0, w, 38.0, rgba(32, 43, 60, 255));
        for (i, (label, g)) in [
            ("JUSTICE CRITICAL", "JC"),
            ("JUSTICE", "JUSTICE"),
            ("ATTACK", "ATTACK"),
            ("FLICK TOUCH", "TOUCH"),
        ]
        .iter()
        .enumerate()
        {
            let x = 20.0 + [0.0, 202.0, 330.0, 450.0][i];
            let mut color = grade(g);
            color[3] = 255;
            v.rect(x, 14.0, 10.0, 10.0, color);
            v.text(x + 17.0, 25.0, 15.0, label, rgba(238, 242, 252, 255));
        }
    }
    let windows = if o.judge {
        protect(&c.notes, o.easy)
    } else {
        Vec::new()
    };
    let units = slide_units(c);
    let divs = divisions(c);
    for (col, &(start, end)) in layout.ranges.iter().enumerate() {
        let x = 68.0 + col as f64 * STRIDE;
        let top = bottom - (end - start) * pps;
        let y = |t: f64| bottom - (t - start) * pps;
        v.rect(x, top, 160.0, bottom - top, rgba(55, 65, 81, 255));
        for b in &c.beats {
            if b.time >= start && b.time < end {
                let yy = y(b.time);
                if b.major {
                    v.rect(x - 40.0, yy - 2.0, 200.0, 2.0, rgba(22, 163, 74, 204));
                    v.text(
                        x - 40.0,
                        yy - 6.0,
                        16.0,
                        &format!("#{}", b.measure),
                        rgba(74, 222, 128, 255),
                    );
                } else {
                    v.rect(x - 10.0, yy - 2.0, 10.0, 2.0, rgba(255, 255, 255, 102));
                    v.rect(x, yy, 160.0, 2.0, rgba(75, 85, 99, 102));
                }
            }
        }
        for b in &c.tempos {
            if b.time >= start && b.time < end {
                let yy = y(b.time);
                v.rect(x - 40.0, yy - 2.0, 40.0, 2.0, rgba(75, 85, 99, 255));
                v.text(
                    x - 39.0,
                    yy + 18.0,
                    16.0,
                    &format!("{}", b.bpm),
                    rgba(209, 213, 219, 255),
                );
            }
        }
        for (t, label) in &divs {
            if *t >= start && *t < end {
                let yy = y(*t);
                v.rect(x + 160.0, yy - 2.0, 4.0, 2.0, rgba(255, 255, 255, 255));
                v.text(x + 168.0, yy + 1.0, 16.0, label, rgba(255, 255, 255, 255));
            }
        }
        for sv in &c.scrolls {
            if sv.time < end && sv.end_time > start {
                let yt = y(sv.end_time).max(top);
                let yb = y(sv.time).min(bottom);
                let mut color = scroll_color(sv.velocity);
                if sv.global {
                    v.rect(x - 4.0, yt, 4.0, yb - yt, color);
                    v.text(
                        x - 37.0,
                        (yt + yb) / 2.0 + 5.0,
                        14.0,
                        &format!("{:.2}x", sv.velocity),
                        rgba(156, 163, 175, 255),
                    );
                } else {
                    color[3] = 26;
                    v.blend = tiny_skia::BlendMode::Screen;
                    v.rect(x + sv.lane * 10.0, yt, sv.width * 10.0, yb - yt, color);
                    v.blend = tiny_skia::BlendMode::SourceOver;
                }
            }
        }
        for layer in 2..=5 {
            for (i, n) in c.longs.iter().enumerate() {
                let hold = matches!(n.head.kind.as_str(), "HLD" | "HXD");
                if n.end_time >= start
                    && n.head.time < end
                    && !n.head.kind.starts_with('A')
                    && ((layer == 2 && hold) || (layer == 4 && !hold))
                {
                    long_path(&mut v, n, x, &y, top, bottom, units[i]);
                }
            }
            if !o.judge {
                for n in &c.notes {
                    if n.time >= start
                        && n.time < end
                        && ((layer == 3 && n.kind == "HLD_T")
                            || (layer == 5 && matches!(n.kind.as_str(), "SLD_H" | "SLD_T")))
                    {
                        glyph(&mut v, n, x, y(n.time), top, bottom);
                    }
                }
            }
        }
        for w in &windows {
            if w.note.time + 0.11 < start || w.note.time - 0.11 > end {
                continue;
            }
            let first = w.note.lane as usize;
            let last = (w.note.lane + w.note.width) as usize;
            let all: Vec<_> = (first..last).map(|l| bands(w, l, o.easy)).collect();
            // Merge equal adjacent lanes; a note's outer contour remains visible.
            let mut i = 0;
            while i < all.len() {
                let mut j = i + 1;
                while j < all.len() && all[j] == all[i] {
                    j += 1;
                }
                for b in &all[i] {
                    let yt = y(w.note.time + b.to).max(top);
                    let yb = y(w.note.time + b.from).min(bottom);
                    v.rect(
                        x + (first + i) as f64 * 10.0,
                        yt,
                        (j - i) as f64 * 10.0,
                        yb - yt,
                        grade(b.grade),
                    );
                }
                i = j;
            }
            let bounds: Vec<_> = all
                .iter()
                .map(|bs| {
                    bs.first().zip(bs.last()).map(|(a, b)| {
                        (
                            y(w.note.time + b.to).max(top),
                            y(w.note.time + a.from).min(bottom),
                        )
                    })
                })
                .collect();
            let ink = rgba(231, 243, 255, 205);
            for (i, b) in bounds.iter().enumerate() {
                if let Some((yt, yb)) = *b {
                    if yb <= yt {
                        continue;
                    }
                    let xl = x + (first + i) as f64 * 10.0;
                    let xr = xl + 10.0;
                    v.path(&[(xl, yt), (xr, yt)], ink, Some(1.1), false);
                    v.path(&[(xl, yb), (xr, yb)], ink, Some(1.1), false);
                    let prev = i.checked_sub(1).and_then(|k| bounds[k]);
                    if let Some((pt, pb)) = prev {
                        v.path(&[(xl, yt), (xl, pt)], ink, Some(1.1), false);
                        v.path(&[(xl, yb), (xl, pb)], ink, Some(1.1), false);
                    } else {
                        v.path(&[(xl, yt), (xl, yb)], ink, Some(1.1), false);
                    }
                    if i + 1 == bounds.len() || bounds[i + 1].is_none() {
                        v.path(&[(xr, yt), (xr, yb)], ink, Some(1.1), false);
                    }
                }
            }
        }

        for n in &c.notes {
            if n.time >= start
                && n.time < end
                && !n.kind.starts_with('A')
                && (o.judge || !matches!(n.kind.as_str(), "HLD_T" | "SLD_H" | "SLD_T"))
            {
                glyph(&mut v, n, x, y(n.time), top, bottom);
            }
        }
        for (i, n) in c.longs.iter().enumerate() {
            if n.end_time >= start && n.head.time < end && n.head.kind.starts_with('A') {
                long_path(&mut v, n, x, &y, top, bottom, units[i]);
            }
        }
        for n in &c.notes {
            if n.time >= start && n.time < end && n.kind.starts_with('A') {
                glyph(&mut v, n, x, y(n.time), top, bottom);
            }
        }
    }
    v.rect(0.0, h - FOOT, w, FOOT, rgba(32, 43, 60, 255));
    v.text(
        12.0,
        h - 23.0,
        12.0,
        "github.com/watagashi-uni/chuni-chart-rs (AGPL-3.0)",
        rgba(212, 225, 238, 255),
    );
    v.text(
        12.0,
        h - 7.0,
        11.0,
        "Original: QingQiz/MarisaBot",
        rgba(212, 225, 238, 255),
    );
    let mut out = Bounded(Vec::new());
    if o.format == "png" {
        let mut enc = png::Encoder::new(&mut out, d.width, d.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|_| "PNG header failed")?;
        writer
            .write_image_data(v.pix.data())
            .map_err(|_| "PNG encoding failed or output limit exceeded")?;
        writer.finish().map_err(|_| "PNG finish failed")?;
    } else {
        jpeg_encoder::Encoder::new(&mut out, 90)
            .encode(
                v.pix.data(),
                d.width as u16,
                d.height as u16,
                jpeg_encoder::ColorType::Rgba,
            )
            .map_err(|_| "JPEG encoding failed or output limit exceeded")?;
    }
    Ok(out.0)
}
