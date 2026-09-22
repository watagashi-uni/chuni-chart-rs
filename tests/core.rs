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
    assert!(Chart::parse(&" ".repeat(2 * 1024 * 1024 + 1)).is_err());
}
#[derive(Deserialize)]
struct Case {
    notes: Vec<Note>,
    easy: bool,
    expected: Vec<Expected>,
}
#[derive(Deserialize)]
struct Expected {
    note: serde_json::Value,
    lanes: Vec<Vec<serde_json::Value>>,
}
#[test]
fn numerical_regression_against_javascript_port() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/protection.json")).unwrap();
    for (i, c) in cases.iter().enumerate() {
        let actual = protect(&c.notes, c.easy);
        assert_eq!(actual.len(), c.expected.len());
        for (w, e) in actual.iter().zip(&c.expected) {
            assert_eq!(w.note.kind, e.note["type"]);
            for (lane, expected) in
                (w.note.lane as usize..(w.note.lane + w.note.width) as usize).zip(&e.lanes)
            {
                let bs = bands(w, lane, c.easy);
                assert_eq!(
                    bs.len(),
                    expected.len(),
                    "case {i}, note {:?}, lane {lane}, actual {bs:?}, expected {expected:?}",
                    w.note
                );
                for (b, e) in bs.iter().zip(expected) {
                    assert_eq!(b.grade, e["grade"]);
                    assert!((b.from - e["from"].as_f64().unwrap()).abs() < 1e-12);
                    assert!((b.to - e["to"].as_f64().unwrap()).abs() < 1e-12);
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
    assert!(d.width as u64 * d.height as u64 <= 12_000_000);
    assert!(d.width <= 16384 && d.height <= 16384);
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
    let router = service::router(d.path().to_owned(), std::time::Duration::from_secs(1));
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
