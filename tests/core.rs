use chuni_chart_rs::{
    chart::{Chart, Note},
    judgement::{bands, protect},
    render::{self, Options},
    service,
};
use serde::Deserialize;
const SIMPLE: &str = "RESOLUTION 384\nBPM_DEF 170.5\nTAP 0 0 0 4\nCHR 1 0 3 4\nHLD 1 96 0 4 192\nSLC 2 0 4 4 96 8 4\nSLD 2 96 8 4 96 10 4\nAIR 2 192 10 4\n";
#[test]
fn fractional_bpm_and_change() {
    let c =
        Chart::parse("RESOLUTION 384\nBPM_DEF 170.5\nBPM 1 0 95.25\nTAP 0 192 0 1\nTAP 2 0 0 1")
            .unwrap();
    assert!((c.notes[0].time - 120.0 / 170.5).abs() < 1e-12);
    assert!((c.notes[1].time - (240.0 / 170.5 + 240.0 / 95.25)).abs() < 1e-12);
}
#[test]
fn heads_and_continuations() {
    let c = Chart::parse(SIMPLE).unwrap();
    assert_eq!(c.notes.iter().filter(|n| n.kind == "SLD_H").count(), 1);
    assert_eq!(c.notes.iter().filter(|n| n.ground()).count(), 4);
    assert_eq!(c.longs.len(), 3);
}
#[test]
fn malformed_and_expanding_inputs() {
    for source in [
        "BPM_DEF NaN\nTAP 0 0 0 1",
        "BPM_DEF 0\nTAP 0 0 0 1",
        "BPM_DEF 120\nTAP 0 0 144 1",
        "BPM_DEF 120\nTAP 0 0 0.5 1",
        "BPM_DEF 120\nHLD 0 0 0 1 -1",
        "BPM_DEF 120\nALD 0 0 0 1 0.00000001 0 384 0 1 0 DEF",
        "BPM_DEF 0.000001\nTAP 2 0 0 1",
        "BPM_DEF 120\nMET_DEF 0 4\nTAP 0 0 0 1",
        "BPM_DEF 120\nALIEN 0 0 0 1",
    ] {
        assert!(Chart::parse(source).is_err(), "accepted: {source}");
    }
}
#[test]
fn input_size_boundary_in_parser_and_cli_reader() {
    let mut source = String::from(SIMPLE);
    source.push_str("//");
    source.extend(std::iter::repeat_n('x', 20 * 1024 * 1024 - source.len()));
    assert!(Chart::parse(&source).is_ok());
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), &source).unwrap();
    assert_eq!(service::read_chart(file.path()).unwrap(), source);
    source.push('x');
    assert_eq!(
        Chart::parse(&source).unwrap_err(),
        "Chart exceeds 20 MiB input limit"
    );
    std::fs::write(file.path(), &source).unwrap();
    assert_eq!(
        service::read_chart(file.path()).unwrap_err(),
        "Chart exceeds 20 MiB input limit"
    );
}
#[derive(Deserialize)]
struct Case {
    notes: Vec<Note>,
    easy: bool,
}
#[test]
fn protection_is_bounded_and_independent_of_input_order() {
    // Retained random inputs from the original reference suite, now with
    // coherent chart ticks at BPM 120. Old JS outputs are not the EXE oracle.
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/protection.json")).unwrap();
    for c in cases {
        let actual = protect(&c.notes, c.easy);
        let mut reversed = c.notes.clone();
        reversed.reverse();
        let reversed = protect(&reversed, c.easy);
        let signature = |ws: &[chuni_chart_rs::judgement::Window]| {
            let mut rows: Vec<_> = ws
                .iter()
                .map(|w| {
                    serde_json::to_string(&serde_json::json!({
                        "note": w.note, "early": w.early, "late": w.late, "miss": w.miss
                    }))
                    .unwrap()
                })
                .collect();
            rows.sort();
            rows
        };
        assert_eq!(signature(&actual), signature(&reversed));
        for w in &actual {
            let outer = (if c.easy && w.note.kind != "FLK" {
                6.0
            } else {
                5.0
            }) / 60.0;
            for lane in w.note.lane as usize..(w.note.lane + w.note.width) as usize {
                assert!((2.0 / 60.0..=outer).contains(&w.early[lane]));
                assert!((2.0 / 60.0..=outer).contains(&-w.late[lane]));
                if !w.note.critical() {
                    assert_eq!(w.early[lane], w.early[w.note.lane as usize]);
                    assert_eq!(w.late[lane], w.late[w.note.lane as usize]);
                }
                for b in bands(w, lane, c.easy) {
                    assert!(b.from.is_finite() && b.to.is_finite() && b.to > b.from);
                    assert!(b.from >= -outer && b.to <= outer);
                }
            }
        }
    }
}
#[test]
fn critical_has_only_jc() {
    let c = Chart::parse("BPM_DEF 120\nCHR 0 0 0 16").unwrap();
    let ws = protect(&c.notes, false);
    let bs = bands(&ws[0], 0, false);
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].grade, "JC");
    assert!((bs[0].from + 5.0 / 60.0).abs() < 1e-12);
}
#[test]
fn critical_coverage_promotes_full_heads_with_overhangs_and_protection() {
    for record in ["HLD 0 192 4 4 96", "SLC 0 192 4 4 96 8 4"] {
        for cover in [
            "CHR 0 192 0 8",
            "CHR 0 192 4 8",
            "CHR 0 192 0 16",
            "CHR 0 192 6 6\nCHR 0 192 0 6",
        ] {
            let c =
                Chart::parse(&format!("BPM_DEF 120\n{record}\n{cover}\nTAP 0 201.6 6 4")).unwrap();
            let head = c
                .notes
                .iter()
                .find(|n| matches!(n.kind.as_str(), "HLD_H" | "SLD_H"))
                .unwrap();
            assert!(head.critical());
            for easy in [false, true] {
                let mut notes = c.notes.clone();
                // The direct numerical API must also recognize unannotated heads.
                for n in &mut notes {
                    n.critical = false;
                }
                let ws = protect(&notes, easy);
                let w = ws.iter().find(|w| w.note.kind == head.kind).unwrap();
                for lane in 4..8 {
                    let bs = bands(w, lane, easy);
                    assert_eq!(bs.len(), 1);
                    assert_eq!(bs[0].grade, "JC");
                }
                let attack = if easy { 0.1 } else { 5.0 / 60.0 };
                assert!((bands(w, 4, easy)[0].to - attack).abs() < 1e-12);
                assert!((bands(w, 7, easy)[0].to - 2.0 / 60.0).abs() < 1e-12);
            }
        }
    }
}
#[test]
fn partial_gapped_or_different_tick_critical_does_not_promote_heads_or_tails() {
    for cover in [
        "CHR 0 192 0 6",
        "CHR 0 192 0 5\nCHR 0 192 6 6",
        "CHR 0 193 0 16",
    ] {
        let c = Chart::parse(&format!("BPM_DEF 120\nHLD 0 192 4 4 96\n{cover}")).unwrap();
        assert!(
            !c.notes
                .iter()
                .find(|n| n.kind == "HLD_H")
                .unwrap()
                .critical()
        );
    }
    let c = Chart::parse("BPM_DEF 120\nHLD 0 96 4 4 96\nTAP 0 192 4 4\nCHR 0 192 0 16").unwrap();
    assert!(
        c.notes
            .iter()
            .filter(|n| n.kind == "HLD_T")
            .all(|n| !n.critical())
    );
}

fn decode_png(c: &Chart, o: &Options) -> (u32, u32, Vec<u8>) {
    let bytes = render::render(c, o).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    (info.width, info.height, pixels)
}

#[test]
fn early_attack_mode_filters_after_protection_and_draws_only_early_bands() {
    let chart = Chart::parse(
        "BPM_DEF 120\nCHR 0 144 0 4\nTAP 0 163.2 0 4\nTAP 0 240 4 4\n\
         HLD 0 288 8 4 96\nSLD 1 0 12 4 96 12 4\n\
         FLK 0 192 12 4\nAIR 0 240 12 4\nMNE 0 216 12 4",
    )
    .unwrap();
    for easy in [false, true] {
        // The mode must imply judge even when the caller leaves judge=false.
        let options = Options {
            early_attack_only: true,
            easy,
            zoom: 2.0,
            ..Default::default()
        };
        let (width, height, selected) = decode_png(&chart, &options);
        let all_options = Options {
            judge: true,
            early_attack_only: false,
            ..options.clone()
        };
        let (all_width, all_height, all) = decode_png(&chart, &all_options);
        assert_eq!((width, height), (all_width, all_height));
        let mut empty = chart.clone();
        empty.notes.clear();
        empty.longs.clear();
        let (_, _, blank) = decode_png(&empty, &options);
        let layout = chuni_chart_rs::layout::layout(&chart, options.zoom);
        let scale = render::dimensions(&chart, &options)
            .unwrap()
            .pixels_per_second
            / layout.pps;
        let bottom = layout
            .ranges
            .iter()
            .map(|(a, b)| (b - a) * layout.pps)
            .fold(0.0, f64::max)
            + 44.0;
        let pixel = |pixels: &[u8], lane: usize, time: f64| -> [u8; 4] {
            let x = ((68.0 + (lane as f64 + 0.5) * 10.0) * scale).round() as usize;
            let y = ((bottom - (time - layout.ranges[0].0) * layout.pps) * scale).round() as usize;
            pixels[(y * width as usize + x) * 4..][..4]
                .try_into()
                .unwrap()
        };
        // Each grade of a qualifying TAP survives, not just its green ATTACK.
        for time in [1.175, 1.2, 1.23] {
            assert_eq!(pixel(&selected, 5, time), pixel(&all, 5, time));
            assert_ne!(pixel(&selected, 5, time), pixel(&blank, 5, time));
        }
        // The CHR before the 0.85s TAP removes its early ATTACK. Filtering that
        // CHR's bands must not restore the TAP's window. Late-only ATTACK is excluded.
        for (lane, time) in [(1, 0.81), (1, 0.675), (13, 0.925), (5, 1.27)] {
            assert_eq!(
                pixel(&selected, lane, time),
                pixel(&blank, lane, time),
                "lane={lane}, time={time}, easy={easy}"
            );
        }
        // Filtering affects bands only: keep even excluded notes, AIR, mines,
        // and the bodies of HOLD / SLIDE as in the complete judgement chart.
        for (lane, time) in [
            (1, 0.75),
            (1, 0.85),
            (5, 1.25),
            (13, 1.0),
            (13, 1.26),
            (13, 1.125),
            (9, 1.75),
            (13, 2.25),
        ] {
            assert_eq!(
                pixel(&selected, lane, time),
                pixel(&all, lane, time),
                "note/body lane={lane}, time={time}"
            );
            assert_ne!(
                pixel(&selected, lane, time),
                pixel(&blank, lane, time),
                "missing note/body lane={lane}, time={time}"
            );
        }
        // Ordinary long-note heads still contribute their early judgement bands.
        for (lane, time) in [(9, 1.48), (13, 1.98)] {
            assert_ne!(pixel(&selected, lane, time), pixel(&blank, lane, time));
        }
    }
}

#[test]
fn overlapping_jc_windows_do_not_accumulate_fill_opacity() {
    let c = Chart::parse("BPM_DEF 600\nCHR 0 192 4 4\nCHR 0 201.6 4 4\nTAP 1 0 12 1").unwrap();
    for zoom in [0.5, 1.0, 2.0] {
        let o = Options {
            judge: true,
            zoom,
            ..Default::default()
        };
        let layout = chuni_chart_rs::layout::layout(&c, zoom);
        let d = render::dimensions(&c, &o).unwrap();
        let scale = d.pixels_per_second / layout.pps;
        let bottom = layout
            .ranges
            .iter()
            .map(|(a, b)| (b - a) * layout.pps)
            .fold(0.0, f64::max)
            + 44.0;
        // The faster chart leaves an interior sample clear of the soft bevels
        // and glyphs at every zoom. Overlap is 176.67–233.33 ms.
        let x = (123.0 * scale).round() as usize;
        let y = ((bottom - (0.225 - layout.ranges[0].0) * layout.pps) * scale).round() as usize;
        let (w, _, both) = decode_png(&c, &o);
        let mut single = c.clone();
        single.notes.retain(|n| n.time != c.notes[1].time);
        let (_, _, one) = decode_png(&single, &o);
        let at = (y * w as usize + x) * 4;
        assert_eq!(&both[at..at + 4], &one[at..at + 4], "zoom={zoom}");
        assert_ne!(
            &both[at..at + 3],
            &[55, 65, 81],
            "JC should still have a visible fill"
        );
    }
}

#[test]
fn thin_attack_is_bright_green_in_final_pixels_without_white_contours() {
    for (zoom, end_measure) in [(0.5, 2), (1.0, 2), (2.0, 2), (1.0, 299)] {
        for height in [1.0, 2.0] {
            for early_attack_only in [false, true] {
                let mut c = Chart::parse(&format!(
                    "BPM_DEF 120\nTAP 0 192 0 4\nTAP 0 220 0 4\nTAP {end_measure} 0 12 1"
                ))
                .unwrap();
                let o = Options {
                    judge: true,
                    early_attack_only,
                    zoom,
                    ..Default::default()
                };
                let d = render::dimensions(&c, &o).unwrap();
                c.notes[1].time = 1.0 + 2.0 * (4.0 / 60.0 + height / d.pixels_per_second);
                let layout = chuni_chart_rs::layout::layout(&c, zoom);
                let scale = d.pixels_per_second / layout.pps;
                if end_measure == 299 {
                    assert!(scale < 1.0, "Exercise automatic image downscaling");
                }
                let bottom = layout
                    .ranges
                    .iter()
                    .map(|(a, b)| (b - a) * layout.pps)
                    .fold(0.0, f64::max)
                    + 44.0;
                let boundary_time = (c.notes[0].time + c.notes[1].time) / 2.0;
                let boundary_y =
                    (bottom - (boundary_time - layout.ranges[0].0) * layout.pps) * scale;
                let (w, _, pixels) = decode_png(&c, &o);
                // Full mode joins two strips at the midpoint. Early-only keeps
                // one: sample inside it and allow its one-pixel antialiasing.
                let row = if early_attack_only {
                    (boundary_y - height / 2.0).floor() as usize
                } else {
                    boundary_y.round() as usize
                };
                let min_green = if early_attack_only { 130 } else { 200 };
                // Include both outer sides: relief must not hide the green strip.
                for x in (68.0 * scale).ceil() as usize..(108.0 * scale).floor() as usize {
                    let p = &pixels[(row * w as usize + x) * 4..][..4];
                    assert!(
                        p[1] > min_green && p[0] < 100 && p[2] < 180,
                        "zoom={zoom}, height={height}, early_only={early_attack_only}, pixel ({x},{row})={p:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn direct_images_and_pixel_budget() {
    let c = Chart::parse(SIMPLE).unwrap();
    for format in ["png", "jpg"] {
        let o = Options {
            format: format.into(),
            judge: true,
            ..Default::default()
        };
        let image = render::render(&c, &o).unwrap();
        if format == "png" {
            assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n");
        } else {
            assert_eq!(&image[..2], b"\xff\xd8");
        }
    }
    let c = Chart::parse("BPM_DEF 120\nTAP 299 0 0 16").unwrap();
    let d = render::dimensions(&c, &Options::default()).unwrap();
    assert!(d.width as u64 * d.height as u64 <= 40_000_000);
    assert!(d.width <= 32768 && d.height <= 32768);
}
#[test]
fn wide_charts_keep_native_text_size_until_the_image_budget_is_reached() {
    let c = Chart::parse("BPM_DEF 120\nTAP 0 0 0 4\nTAP 161 383 0 4").unwrap();
    let o = Options {
        judge: true,
        ..Default::default()
    };
    let d = render::dimensions(&c, &o).unwrap();
    assert_eq!(d.columns, 41);
    assert_eq!(d.width, 10216);
    assert_eq!(d.height, 3016);
    assert_eq!(d.pixels_per_second, 350.0);
    // Very wide charts still have bounded allocations and JPEG-safe dimensions.
    let c = Chart::parse("BPM_DEF 1000\nTAP 0 0 0 4\nTAP 1999 383 0 4").unwrap();
    let d = render::dimensions(&c, &Options { zoom: 0.5, ..o }).unwrap();
    assert!(d.width as u64 * d.height as u64 <= 40_000_000);
    assert!((32767..=32768).contains(&d.width));
    assert!(d.height <= 32768);
}
#[test]
fn filename_boundary() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::write(root.join("1086_03.c2s"), SIMPLE).unwrap();
    assert!(service::chart_path(&root, "1086_03").is_ok());
    assert!(service::chart_path(&root, "1086_03.c2s").is_ok());
    for n in [
        "../secret",
        "/etc/passwd",
        "1086_03.c2s.c2s",
        ".",
        "x?name=foo",
        "",
    ] {
        assert!(service::chart_path(&root, n).is_err());
    }
    #[cfg(unix)]
    {
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("escape.c2s")).unwrap();
        assert!(service::chart_path(&root, "escape").is_err());
    }
}
#[tokio::test]
async fn health_and_fixed_errors() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let d = tempfile::tempdir().unwrap();
    let router = service::router(d.path().to_owned(), std::time::Duration::from_secs(1)).unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/preview?name=private-user-text")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("private-user-text"));
}

#[test]
fn original_measure_layout_and_scroll_annotations() {
    let chart = Chart::parse("BPM_DEF 120\nMET_DEF 4 4\nTAP 0 0 0 4\nTAP 15 383 0 4\nSFL 0 0 768 2\nSLP 0 0 384 0.5 7\nSLA 0 0 0 4 768 7").unwrap();
    let layout = chuni_chart_rs::layout::layout(&chart, 1.0);
    assert!((layout.pps - 350.0).abs() < 1e-12);
    assert_eq!(layout.ranges.len(), 4);
    assert!((layout.ranges[0].1 - (8.0 + 50.0 / 350.0)).abs() < 1e-12);
    assert_eq!(chart.scrolls.iter().filter(|s| s.global).count(), 2);
    assert_eq!(chart.scrolls.iter().filter(|s| !s.global).count(), 2);
    assert!(
        render::dimensions(
            &chart,
            &Options {
                column: Some(5),
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn long_measure_subdivisions_match_upstream_split_positions() {
    let chart = Chart::parse(include_str!("fixtures/layout-long-measures.c2s")).unwrap();
    let layout = chuni_chart_rs::layout::layout(&chart, 1.0);
    // Generated by running MarisaBot's actual parser and Preview.vue functions
    // on this synthetic fixture. Preview.vue blob: 04ea518cc049cc2cef7f51e295ef6b41032e33d1.
    let expected = [
        0.0,
        6.0,
        11.666666666666666,
        17.0,
        22.866666666666667,
        27.666666666666664,
        32.46666666666667,
        37.26666666666667,
        43.491666666666674,
        49.49166666666667,
        55.491666666666674,
    ];
    assert!((layout.pps - 443.22092222986925).abs() < 1e-9);
    assert_eq!(layout.ranges.len(), expected.len() - 1);
    for (range, ends) in layout.ranges.iter().zip(expected.windows(2)) {
        assert!((range.0 + 50.0 / layout.pps - ends[0]).abs() < 1e-9);
        assert!((range.1 - 50.0 / layout.pps - ends[1]).abs() < 1e-9);
    }
}

#[test]
fn off_lane_art_is_clipped_without_changing_layout_or_timing() {
    // Synthetic overflowing heads, a gradient slide crossing the track, AIR and SV2.
    let overflowing = "BPM_DEF 120\nTAP 0 96 -2 4\nMNE 0 144 -16 4\nAUL 0 192 15 4\nSLD 0 0 -16 4 384 28 4\nALD 0 0 -16 2 0 5 384 30 2 5 RED\nSLP 0 0 384 2 1\nSLA 0 0 -16 48 384 1\n";
    let chart = Chart::parse(overflowing).unwrap();
    let baseline = Chart::parse(
        &overflowing
            .replace("-2 4", "0 2")
            .replace("-16 4", "0 4")
            .replace("15 4", "12 4")
            .replace("28 4", "12 4")
            .replace("-16 2", "0 2")
            .replace("30 2", "14 2")
            .replace("-16 48", "0 16"),
    )
    .unwrap();
    assert_eq!(chart.duration, baseline.duration);
    assert_eq!(chart.notes[0].time, baseline.notes[0].time);
    assert!(chart.notes.iter().any(|n| n.lane < 0.0));
    let o = Options::default();
    let decode = |c: &Chart| {
        let bytes = render::render(c, &o).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        (info.width, info.height, pixels)
    };
    let (w, h, pixels) = decode(&chart);
    let (bw, bh, reference) = decode(&baseline);
    assert_eq!((w, h), (bw, bh));
    assert_eq!(render::dimensions(&chart, &o).unwrap().columns, 1);
    for y in 0..h as usize {
        for x in 0..w as usize {
            // Only track pixels may change. Gutter text and neighbouring space stay intact.
            if !(68..228).contains(&x) {
                let i = (y * w as usize + x) * 4;
                assert_eq!(
                    &pixels[i..i + 4],
                    &reference[i..i + 4],
                    "Gutter changed at {x},{y}"
                );
            }
        }
    }
    assert!(!chuni_chart_rs::judgement::supported(&chart.notes));
    assert!(protect(&chart.notes, false).is_empty());
    assert!(render::render(&chart, &Options { judge: true, ..o }).is_err());
}
