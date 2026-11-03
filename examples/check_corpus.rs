// SPDX-License-Identifier: AGPL-3.0-only
//! Small, bounded-process corpus probe; never serializes every note/window.
use chuni_chart_rs::{chart::Chart, judgement, render, service};
use serde_json::json;
use std::path::{Path, PathBuf};

fn check(path: &Path) -> chuni_chart_rs::Result<serde_json::Value> {
    let chart = Chart::parse(&service::read_chart(path)?)?;
    let mut modes = serde_json::Map::new();
    for (name, judge, early_attack_only) in [
        ("preview", false, false),
        ("judge", true, false),
        ("fastattack", true, true),
    ] {
        let options = render::Options {
            judge,
            early_attack_only,
            ..Default::default()
        };
        let result = match render::dimensions(&chart, &options) {
            Ok(dimensions) => json!({"ok": true, "dimensions": dimensions}),
            Err(error) => json!({"ok": false, "error": error}),
        };
        modes.insert(name.into(), result);
    }
    Ok(
        json!({"ok": true, "notes": chart.notes.len(), "longs": chart.longs.len(),
        "duration": chart.duration, "judgement_supported": judgement::supported(&chart.notes),
        "modes": modes}),
    )
}

fn main() {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: check_corpus CHART.c2s");
        std::process::exit(2);
    };
    let result = match check(&PathBuf::from(path)) {
        Ok(value) => value,
        Err(error) => json!({"ok": false, "error": error}),
    };
    println!("{result}");
}
