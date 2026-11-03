use chuni_chart_rs::{
    chart::{Chart, SlideStyle},
    judgement::protect,
    render::{self, Options},
};

#[test]
fn slp_zero_is_the_global_sfl_program_without_an_area() {
    let source = "RESOLUTION 384\nBPM_DEF 120\nBPM 1 0 240\nTAP 0 0 0 4\nTAP 3 0 0 4\nSFL 0 192 576 -2\nSFL 2 0 384 0\n";
    let legacy = Chart::parse(source).unwrap();
    let modern = Chart::parse(
        &source
            .replace("SFL 0 192 576 -2", "SLP 0 192 576 -2 0")
            .replace("SFL 2 0 384 0", "SLP 2 0 384 0 0"),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::to_value(&modern).unwrap()
    );
    assert_eq!(modern.scrolls.len(), 3);
    assert!(modern.scrolls.iter().all(|s| s.global && s.profile_id == 0));
    assert_eq!(modern.scroll_profiles[0].segments.len(), 2);
    assert_eq!(modern.scrolls[0].time, 1.0);
    assert_eq!(modern.scrolls[0].end_time, 2.0);
    assert_eq!(modern.scrolls[1].end_time, 3.0);
    assert_eq!(
        render::render(&legacy, &Options::default()).unwrap(),
        render::render(&modern, &Options::default()).unwrap()
    );
}

#[test]
fn area_scope_is_independent_of_program_timing_and_uses_exe_priority() {
    // A note at tick 800 uses a program which began at tick 96. Clipping the
    // program to the selection area would discard the earlier speed change.
    let c = Chart::parse("RESOLUTION 384\nBPM_DEF 120\nBPM 2 0 240\nTAP 3 0 0 4\nSLP 0 96 288 2 2\nSLP 0 0 384 -1 7\nSLA 2 0 2 8 384 7\nSLA 2 0 0 16 384 2\nSLA 2 0 0 16 384 0\n").unwrap();
    assert_eq!(c.scroll_profiles[0].id, 2);
    assert_eq!(c.scroll_profiles[0].segments[0].tick, 96.0);
    assert_eq!(c.scroll_areas[0].tick, 768.0);
    assert_eq!(
        (c.scroll_areas[0].time, c.scroll_areas[0].end_time),
        (4.0, 5.0)
    );
    assert_eq!(c.scroll_profile_at(800.0, 2.0, 8.0), 7);
    assert_eq!(c.scroll_profile_at(800.0, 1.0, 2.0), 2); // Partial overlap is insufficient.
    assert_eq!(c.scroll_profile_at(800.0, 9.0, 2.0), 2);
    assert_eq!(c.scroll_profile_at(800.0, -1.0, 2.0), 0);
    assert_eq!(c.scroll_profile_at(767.0, 2.0, 4.0), 0);
    assert_eq!(c.scroll_profile_at(767.5, 2.0, 4.0), 7); // +1/192 beat tolerance.
    assert_eq!(c.scroll_profile_at(1151.0, 2.0, 4.0), 7);
    assert_eq!(c.scroll_profile_at(1151.5, 2.0, 4.0), 0); // End is exclusive.
    assert_eq!(c.scroll_profile_at(f64::NAN, 2.0, 4.0), 0);
    assert!(c.scrolls.iter().all(|s| !s.global && s.area.is_some()));
    let annotation = c.scrolls.iter().find(|s| s.profile_id == 2).unwrap();
    assert_eq!(annotation.time, 0.5);
    assert_eq!(c.scroll_areas[annotation.area.unwrap()].tick, 768.0);
}

#[test]
fn scroll_programs_preserve_unreferenced_and_zero_duration_records() {
    let c =
        Chart::parse("BPM_DEF 120\nTAP 1 0 0 4\nSLP 0 96 0 0 9\nSLP 0 192 96 -3 0\nSFL 0 0 96 2\n")
            .unwrap();
    assert_eq!(
        c.scroll_profiles.iter().map(|p| p.id).collect::<Vec<_>>(),
        [0, 9]
    );
    assert_eq!(c.scroll_profiles[0].segments[0].tick, 0.0);
    assert_eq!(c.scroll_profiles[1].segments[0].tick, 96.0);
    assert_eq!(c.scroll_profiles[1].segments[0].end_tick, 96.0);
    assert_eq!(c.scrolls.len(), 2);
}

#[test]
fn invalid_scroll_records_are_rejected_even_without_an_area() {
    for record in [
        "SLP 0 0 96 2 -1",
        "SLP 0 0 96 2 1.5",
        "SLP 0 0 96 2 2147483648",
        "SLP 0 0 96 NaN 0",
        "SLP 0 0 -96 2 0",
        "SLP 0 0 200000 2 9",
        "SLA 0 0 0 4 384 1.5",
        "SLA 0 0 0 4 -384 1",
        "SLA 0 0 0 0 384 1",
    ] {
        assert!(
            Chart::parse(&format!("BPM_DEF 120\nTAP 1 0 0 4\n{record}\n")).is_err(),
            "accepted {record}"
        );
    }
}

#[test]
fn slide_styles_parse_without_changing_head_judgement() {
    for (field, style) in [
        ("", SlideStyle::Slide),
        ("SLD", SlideStyle::Slide),
        ("NCL", SlideStyle::NoCenterLine),
        ("HLD", SlideStyle::Hold),
        ("GRN", SlideStyle::Green),
        ("UNKNOWN", SlideStyle::Slide),
    ] {
        for kind in ["SLD", "SLC", "SXD", "SXC"] {
            let c =
                Chart::parse(&format!("BPM_DEF 120\n{kind} 0 0 4 4 384 4 4 {field}\n")).unwrap();
            assert_eq!(c.longs[0].head.kind, kind);
            assert_eq!(c.longs[0].head.slide_style, Some(style));
            let head = c.notes.iter().find(|n| n.ground()).unwrap();
            assert_eq!(
                head.kind,
                if kind.starts_with("SX") {
                    "CHR"
                } else {
                    "SLD_H"
                }
            );
            assert_eq!(head.slide_style, Some(style));
            assert_eq!(head.critical(), kind.starts_with("SX"));
            assert_eq!(protect(&c.notes, false).len(), 1);
            if !field.is_empty() && field != "UNKNOWN" {
                assert_eq!(serde_json::to_value(head).unwrap()["slide_style"], field);
            }
        }
    }
}

#[test]
fn only_matching_styles_join_and_coincident_styles_remain_distinct() {
    for style in ["SLD", "NCL", "HLD", "GRN"] {
        for next in ["SLD", "NCL", "HLD", "GRN"] {
            let c = Chart::parse(&format!(
                "BPM_DEF 120\nSLC 0 0 4 4 96 8 4 {style}\nSLD 0 96 8 4 96 4 4 {next}\n"
            ))
            .unwrap();
            assert_eq!(
                c.notes.iter().filter(|n| n.ground()).count(),
                if style == next { 1 } else { 2 }
            );
        }
    }
    let source = ["SLD", "NCL", "HLD", "GRN"]
        .map(|s| format!("SLD 0 0 4 4 96 8 4 {s}\n"))
        .join("");
    let c = Chart::parse(&format!("BPM_DEF 120\n{source}")).unwrap();
    assert_eq!(c.notes.iter().filter(|n| n.ground()).count(), 4);
    assert_eq!(c.notes.iter().filter(|n| n.kind == "SLD_T").count(), 4);
}

#[test]
fn modern_and_legacy_slide_styles_render_in_all_modes() {
    let c = |record: &str| Chart::parse(&format!("BPM_DEF 120\n{record}\n")).unwrap();
    let hold = c("HLD 0 0 4 4 384");
    let styled = c("SLD 0 0 4 4 384 4 4 NCL");
    let critical = c("SXD 0 0 4 4 384 4 4 NCL");
    let normal = c("SLD 0 0 4 4 384 4 4 SLD");
    let legacy_hold = c("SLD 0 0 4 4 384 4 4 HLD");
    let green = c("SLD 0 0 4 4 384 4 4 GRN");
    let green_ext = c("SXD 0 0 4 4 384 4 4 GRN");
    let hold_ext = c("SXD 0 0 4 4 384 4 4 HLD");
    // Rhythm labels intentionally retain the source SLIDE/HOLD distinction.
    for options in [
        Options {
            divisions: false,
            ..Default::default()
        },
        Options {
            judge: true,
            divisions: false,
            ..Default::default()
        },
        Options {
            early_attack_only: true,
            divisions: false,
            ..Default::default()
        },
    ] {
        let decode = |chart: &Chart| {
            let bytes = render::render(chart, &options).unwrap();
            let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            (info.width as usize, info.height as usize, pixels)
        };
        let plain = decode(&styled);
        let ext = decode(&critical);
        let hold = decode(&hold);
        let normal = decode(&normal);
        let legacy_hold = decode(&legacy_hold);
        let green = decode(&green);
        let green_ext = decode(&green_ext);
        let hold_ext = decode(&hold_ext);
        assert_eq!((plain.0, plain.1), (normal.0, normal.1));
        let pixel = |image: &(usize, usize, Vec<u8>), x: usize, y: usize| {
            let offset = (y * image.0 + x) * 4;
            <[u8; 4]>::try_from(&image.2[offset..offset + 4]).unwrap()
        };
        let layout = chuni_chart_rs::layout::layout(&styled, options.zoom);
        let scale = render::dimensions(&styled, &options)
            .unwrap()
            .pixels_per_second
            / layout.pps;
        // Column top = 44, track left = 68; sample away from beats and windows.
        let xy = |lane: f64, time: f64| {
            (
                ((68.0 + lane * 10.0) * scale) as usize,
                ((44.0 + (layout.ranges[0].1 - time) * layout.pps) * scale) as usize,
            )
        };
        for time in [0.25, 0.75, 1.25, 1.75] {
            let (center, y) = xy(6.0, time);
            let (side, _) = xy(5.0, time);
            let body = pixel(&plain, center, y);
            assert_eq!(body, pixel(&plain, side, y), "NCL has a center stripe");
            assert_eq!(
                body,
                pixel(&ext, center, y),
                "Critical head recoloured SLIDE body"
            );
            assert_eq!(body, pixel(&normal, side, y), "NCL changed the SLIDE body");
            let orange = pixel(&hold, center, y);
            assert!(orange[0] > orange[2], "Ordinary HOLD lost its warm colour");
            assert_ne!(body, orange, "NCL inherited HOLD colour");
            assert_ne!(pixel(&normal, center, y), pixel(&normal, side, y));
            assert_eq!(
                pixel(&legacy_hold, center, y),
                orange,
                "HLD style must use the HOLD body"
            );
            assert_eq!(
                pixel(&legacy_hold, side, y),
                orange,
                "HLD style has a center stripe"
            );
            assert_eq!(
                pixel(&hold_ext, center, y),
                orange,
                "EXT changed HLD style body"
            );
            let green_body = pixel(&green, side, y);
            let green_center = pixel(&green, center, y);
            for p in [green_body, green_center] {
                assert!(p[1] > p[0] && p[1] > p[2], "GRN must be green: {p:?}");
            }
            assert_ne!(green_body, green_center, "GRN lost its center stripe");
            assert_eq!(
                green_body,
                pixel(&green_ext, side, y),
                "EXT changed GRN body"
            );
        }
        for time in [0.0, 2.0] {
            let (x, y) = xy(6.0, time);
            assert_eq!(
                pixel(&plain, x, y + 2),
                pixel(&normal, x, y + 2),
                "SLIDE endpoint inherited HOLD colour"
            );
        }
        let (x, y) = xy(6.0, 0.0);
        let gold = pixel(&ext, x, y + 2);
        assert_eq!(pixel(&green_ext, x, y + 2), gold);
        assert_eq!(pixel(&hold_ext, x, y + 2), gold);
        assert!(
            gold[0] > 200 && gold[1] > 150 && gold[2] < 100,
            "Critical head must remain gold"
        );
    }
}
