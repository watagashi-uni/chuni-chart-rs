use chuni_chart_rs::{
    chart::{Chart, Note, mark_critical_notes},
    judgement::{bands, protect, protect_with_resolution},
};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}

#[test]
fn off_track_protection_uses_only_the_physical_keys_and_retains_source_geometry() {
    let source = "BPM_DEF 120\n\
        TAP 0 96 -4 6\nTAP 0 108 1 3\n\
        TAP 0 96 14 6\nTAP 0 108 13 2\n\
        TAP 0 100 -2 2\nCHR 0 100 16 4\n\
        CHR 1 0 -16 48\nTAP 1 12 7 2\n";
    let c = Chart::parse(source).unwrap();
    assert!(chuni_chart_rs::judgement::supported(&c.notes));
    let clipped: Vec<_> = c
        .notes
        .iter()
        .filter_map(|n| {
            let lanes = n.physical_lanes();
            if lanes.is_empty() {
                return None;
            }
            let mut n = n.clone();
            n.lane = lanes.start as f64;
            n.width = lanes.len() as f64;
            Some(n)
        })
        .collect();
    for easy in [false, true] {
        let windows = protect(&c.notes, easy);
        let expected = protect(&clipped, easy);
        assert_eq!(windows.len(), 6);
        assert_eq!(windows.len(), expected.len());
        assert!(
            windows
                .iter()
                .any(|w| w.note.lane == -16.0 && w.note.width == 48.0)
        );
        for (w, e) in windows.iter().zip(&expected) {
            assert_eq!(w.note.time, e.note.time);
            assert_eq!(w.note.kind, e.note.kind);
            assert_eq!(w.early, e.early);
            assert_eq!(w.late, e.late);
            assert_eq!(w.miss, e.miss);
            for l in 0..16 {
                assert_eq!(bands(w, l, easy), bands(e, l, easy));
            }
            assert!(bands(w, 16, easy).is_empty());
            assert!(bands(w, usize::MAX, easy).is_empty());
        }
        // The wide EXT keeps per-key notches; no off-track indexes are visited.
        let wide = windows.iter().find(|w| w.note.width == 48.0).unwrap();
        near(wide.late[7], -2.0 / 60.0);
        near(wide.late[0], -if easy { 0.1 } else { 5.0 / 60.0 });
    }
}

#[test]
fn clipping_does_not_invent_critical_coverage_or_windows_for_off_track_art() {
    let c =
        Chart::parse("BPM_DEF 120\nTAP 0 96 -2 4\nCHR 0 96 0 2\nTAP 0 100 -8 4\nTAP 0 100 16 4")
            .unwrap();
    let ws = protect(&c.notes, false);
    assert_eq!(ws.len(), 2);
    let tap = ws.iter().find(|w| w.note.kind == "TAP").unwrap();
    assert_eq!(tap.note.lane, -2.0);
    assert_eq!(tap.note.width, 4.0);
    assert!(!tap.note.critical()); // EXT covers only half the original TAP.
    near(tap.early[0], 5.0 / 60.0);
    near(tap.late[0], -5.0 / 60.0);
    assert!(bands(tap, 2, false).is_empty());
    assert!(bands(tap, 0, false).iter().any(|b| b.grade == "ATTACK"));
    let art = Chart::parse("BPM_DEF 120\nTAP 0 96 -4 4\nCHR 0 108 16 4").unwrap();
    assert!(chuni_chart_rs::judgement::supported(&art.notes));
    assert!(protect(&art.notes, false).is_empty());
}

#[test]
fn fifty_ms_pair_retains_fix_and_minimalizes_only_tap_and_touch() {
    for kind in ["TAP", "HLD_H", "SLD_H", "CHR", "FLK"] {
        for critical in [false, true] {
            let a = Note {
                kind: kind.into(),
                tick: 192.0,
                time: 1.0,
                lane: 0.0,
                width: 4.0,
                critical,
                slide_style: None,
            };
            let b = Note {
                kind: "TAP".into(),
                tick: 201.6,
                time: 1.05,
                lane: 2.0,
                width: 4.0,
                critical: false,
                slide_style: None,
            };
            let ws = protect(&[a, b], false);
            let a = &ws[0];
            let b = &ws[1];
            near(-a.late[2], 2.0 / 60.0);
            near(b.early[2], 2.0 / 60.0);
            near(
                -a.late[0],
                if a.note.critical() {
                    5.0 / 60.0
                } else {
                    2.0 / 60.0
                },
            );
            near(b.early[5], 2.0 / 60.0);
        }
    }
}

#[test]
fn three_note_chain_does_not_reallocate_a_shortened_window() {
    // Geometry/time gaps of the disputed 2140_03 passage, without game assets.
    let c = Chart::parse("BPM_DEF 178\nTAP 74 336 3 4\nTAP 74 352 0 4\nTAP 74 368 4 4").unwrap();
    let ws = protect(&c.notes, false);
    let dt = ws[2].note.time - ws[0].note.time;
    near(-ws[0].late[4], 2.0 / 60.0);
    near(ws[2].early[4], dt / 2.0);
    near(ws[2].early[4] * 1000.0, 56.1797752809);
    assert!(
        !bands(&ws[2], 4, false)
            .iter()
            .any(|b| b.grade == "ATTACK" && b.to <= 0.0)
    );
    near(
        (dt + ws[0].late[4] - ws[2].early[4]) * 1000.0,
        22.8464419476,
    );
}

#[test]
fn jc_overlap_is_kept_as_an_internal_window() {
    let c = Chart::parse("BPM_DEF 178\nTAP 0 192 4 4\nTAP 0 208 4 4").unwrap();
    let ws = protect(&c.notes, false);
    let a = bands(&ws[0], 4, false);
    let b = bands(&ws[1], 4, false);
    near(a.iter().find(|b| b.grade == "JC").unwrap().to, 2.0 / 60.0);
    near(
        b.iter().find(|b| b.grade == "JC").unwrap().from,
        -2.0 / 60.0,
    );
    let overlap = ws[0].note.time - ws[0].late[4] - (ws[1].note.time - ws[1].early[4]);
    near(overlap * 1000.0, 10.4868913858);
}

#[test]
fn same_position_tolerance_scales_with_chart_resolution() {
    for resolution in [192.0, 384.0, 768.0] {
        for (factor, cut) in [(0.5, false), (1.0, false), (1.5, true)] {
            let tick = resolution / 768.0 * factor;
            let c = Chart::parse(&format!(
                "RESOLUTION {resolution}\nBPM_DEF 120\nTAP 0 0 0 4\nTAP 0 {tick} 0 4"
            ))
            .unwrap();
            let ws = protect_with_resolution(&c.notes, false, c.resolution);
            near(-ws[0].late[0], if cut { 2.0 / 60.0 } else { 5.0 / 60.0 });
        }
    }
}

#[test]
fn flick_touch_keeps_its_profile_on_easy() {
    let c = Chart::parse("BPM_DEF 120\nTAP 0 0 0 4\nFLK 0 36.48 0 4").unwrap(); // 190 ms
    let ws = protect(&c.notes, true);
    near(-ws[0].late[0], 0.1); // 100 + 83.33 ms do not overlap.
    near(ws[1].early[0], 5.0 / 60.0);
    assert_eq!(bands(&ws[1], 0, true)[0].grade, "TOUCH");
    near(bands(&ws[1], 0, true)[0].from, -5.0 / 60.0);
}

#[test]
fn critical_coverage_promotes_tap_without_losing_source_identity() {
    let c = Chart::parse("BPM_DEF 120\nTAP 0 192 2 4\nCHR 0 192 2 2\nCHR 0 192 4 2").unwrap();
    let tap = c.notes.iter().find(|n| n.kind == "TAP").unwrap();
    assert!(tap.critical());
    let ws = protect(&c.notes, false);
    let w = ws.iter().find(|w| w.note.kind == "TAP").unwrap();
    near(bands(w, 2, false)[0].from, -5.0 / 60.0);
    assert_eq!(bands(w, 2, false)[0].grade, "JC");
}

#[test]
fn critical_coverage_uses_quantized_tick_and_existing_critical_heads() {
    let mut notes = vec![
        Note {
            kind: "HLD_H".into(),
            tick: 192.1,
            time: 1.0,
            lane: 0.0,
            width: 8.0,
            critical: true,
            slide_style: None,
        },
        Note {
            kind: "TAP".into(),
            tick: 192.2,
            time: 1.0005,
            lane: 2.0,
            width: 4.0,
            critical: false,
            slide_style: None,
        },
        Note {
            kind: "TAP".into(),
            tick: 192.6,
            time: 1.003,
            lane: 2.0,
            width: 4.0,
            critical: false,
            slide_style: None,
        },
        Note {
            kind: "TAP".into(),
            tick: 192.2,
            time: 1.0005,
            lane: 6.0,
            width: 4.0,
            critical: false,
            slide_style: None,
        },
        Note {
            kind: "HLD_T".into(),
            tick: 192.2,
            time: 1.0005,
            lane: 2.0,
            width: 4.0,
            critical: false,
            slide_style: None,
        },
    ];
    mark_critical_notes(&mut notes);
    assert!(notes[1].critical());
    assert!(!notes[2].critical());
    assert!(!notes[3].critical());
    assert!(!notes[4].critical());
}

#[test]
fn non_cut_note_types_do_not_shorten_ground_windows() {
    let c = Chart::parse("BPM_DEF 120\nTAP 0 192 0 4\nMNE 0 200 0 4\nAIR 0 201 0 4").unwrap();
    let ws = protect(&c.notes, false);
    assert_eq!(ws.len(), 1);
    near(-ws[0].late[0], 5.0 / 60.0);
}
