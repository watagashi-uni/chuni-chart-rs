use chuni_chart_rs::{
    chart::Chart,
    render::{self, Options},
};

fn pixels(chart: &Chart, options: &Options) -> Vec<u8> {
    let data = render::render(chart, options).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(data))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    pixels
}

fn route(style: &str, split: bool, bpm_change: bool, wide: bool) -> Chart {
    let mut source = String::from("BPM_DEF 137\n");
    if bpm_change {
        source.push_str("BPM 0 41 191\nBPM 0 343 103\n");
    }
    if split {
        for i in 0..16 {
            let kind = if i == 15 { "SLD" } else { "SLC" };
            source.push_str(&format!("{kind} 0 {} 4 4 24 4 4 {style}\n", 13 + i * 24));
        }
    } else {
        source.push_str(&format!("SLD 0 13 4 4 384 4 4 {style}\n"));
    }
    if wide {
        source.push_str("TAP 299 0 0 1\n");
    }
    Chart::parse(&source).unwrap()
}

fn assert_same_pixels(reference: &Chart, split: &Chart, options: &Options) {
    let expected = pixels(reference, options);
    let actual = pixels(split, options);
    assert_eq!(actual.len(), expected.len());
    let max_delta = actual
        .iter()
        .zip(&expected)
        .map(|(&a, &b)| a.abs_diff(b))
        .max()
        .unwrap();
    // Allow one quantization level, never a darker row at a segment boundary.
    assert!(
        max_delta <= 1,
        "Splitting a continuous SLIDE changed pixels by {max_delta}"
    );
}

#[test]
fn splitting_slides_does_not_leave_body_or_centerline_seams() {
    for style in ["SLD", "NCL", "HLD", "GRN"] {
        let reference = route(style, false, false, false);
        let split = route(style, true, false, false);
        for (judge, early_attack_only) in [(false, false), (true, false), (true, true)] {
            for zoom in [0.5, 1.0, 2.0] {
                assert_same_pixels(
                    &reference,
                    &split,
                    &Options {
                        judge,
                        early_attack_only,
                        zoom,
                        divisions: false,
                        ..Default::default()
                    },
                );
            }
        }
    }
}

#[test]
fn splitting_across_bpm_changes_preserves_slide_gradient() {
    assert_same_pixels(
        &route("SLD", false, true, false),
        &route("SLD", true, true, false),
        &Options {
            divisions: false,
            ..Default::default()
        },
    );
}

#[test]
fn splitting_slides_stays_seamless_after_automatic_downscaling() {
    let reference = route("SLD", false, false, true);
    let split = route("SLD", true, false, true);
    let options = Options {
        divisions: false,
        ..Default::default()
    };
    assert!(
        render::dimensions(&reference, &options)
            .unwrap()
            .pixels_per_second
            < chuni_chart_rs::layout::layout(&reference, 1.0).pps
    );
    assert_same_pixels(&reference, &split, &options);
}

#[test]
fn bent_variable_width_slides_stay_connected_across_columns_and_track_edges() {
    let route = |style: &str, subdivisions: usize| {
        let knots = [
            (2.0, 2.0),
            (10.0, 4.0),
            (18.0, 4.0),
            (-4.0, 4.0),
            (2.0, 2.0),
        ];
        let mut source = String::from("BPM_DEF 137\n");
        for (leg, pair) in knots.windows(2).enumerate() {
            for i in 0..subdivisions {
                let section = |step: usize| {
                    let u = step as f64 / subdivisions as f64;
                    (
                        pair[0].0 + (pair[1].0 - pair[0].0) * u,
                        pair[0].1 + (pair[1].1 - pair[0].1) * u,
                    )
                };
                let (lane, width) = section(i);
                let (end_lane, end_width) = section(i + 1);
                let kind = if leg == 3 && i + 1 == subdivisions {
                    "SLD"
                } else {
                    "SLC"
                };
                source.push_str(&format!(
                    "{kind} 3 {} {lane} {width} {} {end_lane} {end_width} {style}\n",
                    13 + leg * 384 + i * 384 / subdivisions,
                    384 / subdivisions
                ));
            }
        }
        Chart::parse(&source).unwrap()
    };
    for style in ["SLD", "NCL", "HLD", "GRN"] {
        assert_same_pixels(
            &route(style, 1),
            &route(style, 4),
            &Options {
                divisions: false,
                ..Default::default()
            },
        );
    }
}

#[test]
fn terminated_slides_keep_their_own_gradient_ends() {
    let chart = Chart::parse("BPM_DEF 137\nSLD 0 13 4 4 192 4 4\nSLD 0 205 4 4 192 4 4\n").unwrap();
    let options = Options {
        divisions: false,
        ..Default::default()
    };
    let image = pixels(&chart, &options);
    let layout = chuni_chart_rs::layout::layout(&chart, options.zoom);
    let dimensions = render::dimensions(&chart, &options).unwrap();
    let time = chart.seconds_at(213.0);
    let y = (44.0 + (layout.ranges[0].1 - time) * layout.pps).floor() as usize;
    let at = (y * dimensions.width as usize + 113) * 4;
    // Eight ticks after the second start is in its purple gradient, even
    // though its position exactly matches the previous terminated tail.
    assert!(
        image[at] > 100,
        "SLD tail was joined through: {:?}",
        &image[at..at + 4]
    );
}
