// SPDX-License-Identifier: AGPL-3.0-only
use crate::{MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, Result, render::Options};
use axum::{
    Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
};
#[derive(Clone)]
struct App {
    root: PathBuf,
    slots: Arc<Semaphore>,
    timeout: Duration,
}
#[derive(Serialize, Deserialize)]
pub struct Job {
    pub chart: String,
    pub options: Options,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    name: String,
    judge: Option<u8>,
    easy: Option<u8>,
    format: Option<String>,
    column: Option<usize>,
    zoom: Option<f64>,
}
pub fn chart_path(root: &Path, name: &str) -> Result<PathBuf> {
    let base = name.strip_suffix(".c2s").unwrap_or(name);
    if base.is_empty()
        || base.len() > 80
        || !base
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("Invalid chart filename".into());
    }
    let path = root
        .join(format!("{base}.c2s"))
        .canonicalize()
        .map_err(|_| "Chart not found")?;
    if !path.starts_with(root) || !path.is_file() {
        return Err("Chart not found".into());
    }
    Ok(path)
}
pub fn read_chart(path: &Path) -> Result<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|_| "Cannot open chart")?;
    if !f.metadata().map_err(|_| "Cannot inspect chart")?.is_file() {
        return Err("Not a chart file".into());
    }
    let mut bytes = Vec::new();
    f.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read chart")?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("Chart exceeds 2 MiB input limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "Chart must be UTF-8".into())
}
pub async fn run_worker(job: Job, timeout: Duration) -> Result<Vec<u8>> {
    use std::process::Stdio;
    let exe = std::env::current_exe().map_err(|_| "Cannot locate worker")?;
    let mut command = tokio::process::Command::new(exe);
    command
        .arg("_worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    unsafe {
        command.pre_exec(|| {
            for (resource, value) in [
                (libc::RLIMIT_AS, 256 * 1024 * 1024),
                (libc::RLIMIT_CPU, 12),
                (libc::RLIMIT_CORE, 0),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Cannot start bounded render worker")?;
    let mut stdin = child.stdin.take().ok_or("Worker stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("Worker stdout unavailable")?;
    let payload = serde_json::to_vec(&job).map_err(|_| "Cannot encode worker input")?;
    let result = tokio::time::timeout(timeout, async {
        stdin
            .write_all(&payload)
            .await
            .map_err(|_| "Worker input failed")?;
        drop(stdin);
        let mut bytes = Vec::new();
        stdout
            .take(MAX_OUTPUT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "Worker output failed")?;
        if bytes.len() > MAX_OUTPUT_BYTES {
            return Err("Worker output exceeds limit");
        }
        let status = child.wait().await.map_err(|_| "Worker wait failed")?;
        if !status.success() || bytes.is_empty() {
            return Err("Invalid chart or render resource limit exceeded");
        }
        Ok(bytes)
    })
    .await;
    match result {
        Ok(Ok(bytes)) => Ok(bytes),
        other => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(match other {
                Err(_) => "Render timed out",
                Ok(Err(e)) => e,
                _ => unreachable!(),
            }
            .into())
        }
    }
}
// Keep the slot until the image body is consumed or the connection is dropped.
// Small frames also bound hyper's pending write buffer for slow readers.
struct ImageBody {
    bytes: axum::body::Bytes,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl http_body::Body for ImageBody {
    type Data = axum::body::Bytes;
    type Error = std::convert::Infallible;
    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<std::result::Result<http_body::Frame<Self::Data>, Self::Error>>>
    {
        if self.bytes.is_empty() {
            return std::task::Poll::Ready(None);
        }
        let len = self.bytes.len().min(64 * 1024);
        std::task::Poll::Ready(Some(Ok(http_body::Frame::data(self.bytes.split_to(len)))))
    }
    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(self.bytes.len() as u64)
    }
}
fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        message.to_owned(),
    )
        .into_response()
}
async fn preview(State(app): State<App>, Query(p): Query<Params>) -> Response {
    generate(app, p, false).await
}
async fn judge(State(app): State<App>, Query(p): Query<Params>) -> Response {
    generate(app, p, true).await
}
async fn generate(app: App, p: Params, forced: bool) -> Response {
    if p.judge.is_some_and(|v| v > 1) || p.easy.is_some_and(|v| v > 1) {
        return error(StatusCode::BAD_REQUEST, "Flags must be 0 or 1");
    }
    let format = p.format.unwrap_or_else(|| "png".into());
    if !matches!(format.as_str(), "png" | "jpg" | "jpeg") {
        return error(StatusCode::BAD_REQUEST, "Unsupported image format");
    }
    let Ok(permit) = app.slots.clone().try_acquire_owned() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [
                (header::RETRY_AFTER, "2"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            "Renderer busy; retry later",
        )
            .into_response();
    };
    let path = match chart_path(&app.root, &p.name) {
        Ok(p) => p,
        Err(_) => return error(StatusCode::NOT_FOUND, "Chart not found"),
    };
    let chart = match read_chart(&path) {
        Ok(c) => c,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "Cannot read chart or input exceeds limit",
            );
        }
    };
    let content = if format == "png" {
        "image/png"
    } else {
        "image/jpeg"
    };
    let job = Job {
        chart,
        options: Options {
            judge: forced || p.judge == Some(1),
            easy: p.easy == Some(1),
            format,
            column: p.column,
            zoom: p.zoom.unwrap_or(1.0),
        },
    };
    match run_worker(job, app.timeout).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, content),
                (header::CACHE_CONTROL, "no-store"),
                (
                    header::HeaderName::from_static("x-content-type-options"),
                    "nosniff",
                ),
            ],
            axum::body::Body::new(ImageBody {
                bytes: bytes.into(),
                _permit: permit,
            }),
        )
            .into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}
pub fn router(root: PathBuf, timeout: Duration) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../assets/index.html")) }),
        )
        .route("/healthz", get(|| async { "ok" }))
        .route("/preview", get(preview))
        .route("/api/render", get(preview))
        .route("/judge", get(judge))
        .with_state(App {
            root,
            slots: Arc::new(Semaphore::new(1)),
            timeout,
        })
}
pub async fn serve(root: PathBuf, listen: &str, timeout: u64) -> Result<()> {
    let root = root
        .canonicalize()
        .map_err(|_| "CHART_DIR does not exist")?;
    if !root.is_dir() {
        return Err("CHART_DIR must be a directory".into());
    }
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|_| "Cannot bind listen address")?;
    eprintln!(
        "Listening on {} (one worker, 256 MiB Linux worker limit)",
        listener.local_addr().map_err(|_| "Address unavailable")?
    );
    axum::serve(
        listener,
        router(root, Duration::from_secs(timeout.clamp(1, 30))),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|_| "HTTP server failed".into())
}
