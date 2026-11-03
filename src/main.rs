// SPDX-License-Identifier: AGPL-3.0-only
use chuni_chart_rs::{
    Result,
    chart::Chart,
    config::Config,
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
        /// Show only the early half of notes whose early ATTACK survives protection.
        #[arg(long)]
        early_attack_only: bool,
        /// Hide note-spacing labels such as 1/16 and 1/24.
        #[arg(long)]
        no_divisions: bool,
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
    /// Start the image API; reads config.json when present.
    Serve {
        #[arg(long, env = "CHUNI_CHART_CONFIG")]
        config: Option<PathBuf>,
        #[arg(long, env = "CHART_DIR")]
        chart_dir: Option<PathBuf>,
        #[arg(long, env = "LISTEN")]
        listen: Option<String>,
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Create a configuration with a random persistent ETag salt; never overwrite.
    InitConfig {
        #[arg(short, long, default_value = "config.json")]
        output: PathBuf,
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
            early_attack_only,
            no_divisions,
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
                early_attack_only,
                divisions: !no_divisions,
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
            let windows =
                chuni_chart_rs::judgement::protect_with_resolution(&c.notes, easy, c.resolution);
            let ws:Vec<_>=windows.iter().map(|w|serde_json::json!({"note":w.note,"lanes":(w.note.lane as usize..(w.note.lane+w.note.width)as usize).map(|l|chuni_chart_rs::judgement::bands(w,l,easy)).collect::<Vec<_>>()})).collect();
            serde_json::to_writer(
                std::io::stdout(),
                &serde_json::json!({"judgement_supported":chuni_chart_rs::judgement::supported(&c.notes),"chart":c,"windows":ws}),
            )
            .map_err(|_| "Cannot write inspection".into())
        }
        Command::Serve {
            config,
            chart_dir,
            listen,
            timeout,
        } => {
            let mut settings = Config::load(config.as_deref())?;
            if let Some(root) = chart_dir {
                settings.chart_dir = root;
            }
            if let Some(listen) = listen {
                settings.listen = listen;
            }
            if let Some(seconds) = timeout {
                settings.timeout_seconds = seconds;
            }
            match std::env::var("ETAG_SALT") {
                Ok(salt) => settings.etag_salt = Some(salt),
                Err(std::env::VarError::NotPresent) => {}
                Err(_) => return Err("ETAG_SALT must be UTF-8".into()),
            }
            settings.validate()?;
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|_| "Runtime initialization failed")?
                .block_on(service::serve_config(settings))
        }
        Command::InitConfig { output } => {
            Config::initialize(&output)?;
            eprintln!(
                "Configuration created. Set chart_dir to the directory containing C2S files."
            );
            Ok(())
        }
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
            drop(input);
            let c = Chart::parse(&job.chart)?;
            drop(job.chart);
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
