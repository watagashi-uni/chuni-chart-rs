// SPDX-License-Identifier: AGPL-3.0-only
use chuni_chart_rs::{
    Result,
    chart::Chart,
    render::{self, Options},
    service::{self, Job},
};
use clap::{Parser, Subcommand};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
#[derive(Parser)]
#[command(version, about = "C2S chart PNG/JPEG renderer; no browser required")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Render {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        judge: bool,
        #[arg(long)]
        easy: bool,
        #[arg(long, default_value_t = 1.0)]
        zoom: f64,
        #[arg(long)]
        column: Option<usize>,
    },
    Inspect {
        input: PathBuf,
        #[arg(long)]
        easy: bool,
    },
    Serve {
        #[arg(long, env = "CHART_DIR", default_value = "charts")]
        chart_dir: PathBuf,
        #[arg(long, env = "LISTEN", default_value = "127.0.0.1:3000")]
        listen: String,
        #[arg(long, default_value_t = 15)]
        timeout: u64,
    },
    #[command(name = "_worker", hide = true)]
    Worker,
}
fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Render {
            input,
            output,
            judge,
            easy,
            zoom,
            column,
        } => {
            let chart = Chart::parse(&service::read_chart(&input)?)?;
            let format = output
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("png")
                .to_ascii_lowercase();
            let options = Options {
                judge,
                easy,
                format,
                zoom,
                column,
            };
            let d = render::dimensions(&chart, &options)?;
            let bytes = render::render(&chart, &options)?;
            std::fs::write(output, &bytes).map_err(|_| "Cannot write output")?;
            eprintln!(
                "{} x {}; {:.3} px/s; {} bytes",
                d.width,
                d.height,
                d.pixels_per_second,
                bytes.len()
            );
            Ok(())
        }
        Command::Inspect { input, easy } => {
            let c = Chart::parse(&service::read_chart(&input)?)?;
            let windows = chuni_chart_rs::judgement::protect(&c.notes, easy);
            let ws:Vec<_>=windows.iter().map(|w|serde_json::json!({"note":w.note,"lanes":(w.note.lane as usize..(w.note.lane+w.note.width)as usize).map(|l|chuni_chart_rs::judgement::bands(w,l,easy)).collect::<Vec<_>>()})).collect();
            serde_json::to_writer(
                std::io::stdout(),
                &serde_json::json!({"judgement_supported":chuni_chart_rs::judgement::supported(&c.notes),"chart":c,"windows":ws}),
            )
            .map_err(|_| "Cannot write inspection".into())
        }
        Command::Serve {
            chart_dir,
            listen,
            timeout,
        } => tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| "Runtime initialization failed")?
            .block_on(service::serve(chart_dir, &listen, timeout)),
        Command::Worker => {
            let mut input = Vec::new();
            std::io::stdin()
                .take((chuni_chart_rs::MAX_INPUT_BYTES * 7) as u64 + 1)
                .read_to_end(&mut input)
                .map_err(|_| "Worker input failed")?;
            if input.len() > chuni_chart_rs::MAX_INPUT_BYTES * 7 {
                return Err("Worker input too large".into());
            }
            let job: Job = serde_json::from_slice(&input).map_err(|_| "Invalid worker input")?;
            let c = Chart::parse(&job.chart)?;
            let bytes = render::render(&c, &job.options)?;
            std::io::stdout()
                .write_all(&bytes)
                .map_err(|_| "Worker output failed".into())
        }
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
