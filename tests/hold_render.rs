use chuni_chart_rs::{
    chart::Chart,
    judgement::{bands, protect_with_resolution},
    layout,
    render::{self, Options},
};

fn pixels(chart: &Chart, options: &Options) -> (usize, Vec<u8>) {
    let png = render::render(chart, options).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    (info.width as usize, buffer)
}

#[test]
fn hold_bands_and_bevels_end_at_the_tail_including_ext_heads() {
    let mut chart = Chart::parse(
        "BPM_DEF 180\nHLD 0 144 0 2 6\nHLD 0 144 3 2 6\n\
         HXD 0 144 6 2 6\nTAP 0 144 9 2\nSLD 0 144 12 2 6 12 2 HLD\n\
         HLD 0 288 0 2 96\nHLD 1 192 3 2 0\nHLD 1 192 6 2 1\nTAP 2 0 15 1\n",
    )
    .unwrap();
    // Also cover an ordinary HOLD head promoted to EXT by complete coverage.
    chart
        .notes
        .iter_mut()
        .find(|n| n.kind == "HLD_H" && n.lane == 3.0)
        .unwrap()
        .critical = true;
    let options = Options {
        zoom: 2.0,
        divisions: false,
        ..Default::default()
    };
    let (width, plain) = pixels(&chart, &options);
    let layout = layout::layout(&chart, options.zoom);
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
    let offset = |lane: f64, time: f64| {
        let x = ((68.0 + lane * 10.0) * scale).round() as usize;
        let y = ((bottom - (time - layout.ranges[0].0) * layout.pps) * scale).round() as usize;
        (y * width + x) * 4
    };
    for easy in [false, true] {
        let (_, judged) = pixels(
            &chart,
            &Options {
                judge: true,
                easy,
                ..options.clone()
            },
        );
        // No grade fill or inset bevel may remain beyond a short HOLD's tail.
        for (head_time, lanes) in [(0.5, &[0.0, 3.0, 6.0][..]), (2.0, &[3.0, 6.0][..])] {
            for &lane in lanes {
                for ms in 40..80 {
                    for dx in [0.2, 0.5, 1.0, 1.5, 1.8] {
                        let at = offset(lane + dx, head_time + ms as f64 / 1000.0);
                        assert_eq!(
                            &judged[at..at + 4],
                            &plain[at..at + 4],
                            "late fill/bevel: lane={lane}, ms={ms}, easy={easy}"
                        );
                    }
                }
            }
        }
        // Early bands remain; ordinary TAP, long HOLD, and HLD-styled SLIDE
        // retain their late windows. The latter is still a SLIDE note.
        for (lane, time) in [
            (1.0, 0.45),
            (4.0, 0.45),
            (7.0, 0.45),
            (1.0, 0.512),
            (4.0, 0.512),
            (7.0, 0.512),
            (1.0, 1.075),
            (10.0, 0.575),
            (13.0, 0.575),
        ] {
            let at = offset(lane, time);
            assert_ne!(
                &judged[at..at + 4],
                &plain[at..at + 4],
                "missing band: lane={lane}, time={time}"
            );
        }
        // The renderer's display cap must not alter the numerical API.
        let windows = protect_with_resolution(&chart.notes, easy, chart.resolution);
        let head = windows
            .iter()
            .find(|w| w.note.kind == "HLD_H" && w.note.time == 0.5 && w.note.lane == 0.0)
            .unwrap();
        assert!(bands(head, 0, easy).last().unwrap().to > 0.08);
    }
}

#[test]
fn hold_display_uses_end_time_across_a_bpm_change() {
    let chart = Chart::parse("BPM_DEF 180\nBPM 1 0 360\nHLD 0 378 4 4 12\nTAP 2 0 0 2\n").unwrap();
    let hold = &chart.longs[0];
    assert!((hold.end_time - hold.head.time - 0.03125).abs() < 1e-12);
    let options = Options {
        divisions: false,
        zoom: 2.0,
        ..Default::default()
    };
    let (width, plain) = pixels(&chart, &options);
    let (_, judged) = pixels(
        &chart,
        &Options {
            judge: true,
            ..options.clone()
        },
    );
    let layout = layout::layout(&chart, options.zoom);
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
    // Between the actual tail and the incorrect duration using only head BPM.
    let time = hold.head.time + 0.040;
    let x = ((68.0 + 6.0 * 10.0) * scale).round() as usize;
    let y = ((bottom - (time - layout.ranges[0].0) * layout.pps) * scale).round() as usize;
    let at = (y * width + x) * 4;
    assert_eq!(&judged[at..at + 4], &plain[at..at + 4]);
}
