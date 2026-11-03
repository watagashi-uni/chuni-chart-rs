// SPDX-License-Identifier: AGPL-3.0-only
use crate::{
    MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, Result,
    config::Config,
    etag::{self, EtagKey},
    queue::{QueueError, RenderPermit, RenderQueue},
    render::Options,
    storage::{ChartRoot, ReadError},
};
use axum::{
    Router,
    extract::{Path as UrlPath, Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
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
    root: Arc<ChartRoot>,
    etag: EtagKey,
    reads: Arc<Semaphore>,
    queue: Arc<RenderQueue>,
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
    name: Option<String>,
    judge: Option<u8>,
    early_attack_only: Option<u8>,
    fastattack: Option<u8>,
    div: Option<u8>,
    easy: Option<u8>,
    format: Option<String>,
    column: Option<usize>,
    zoom: Option<f64>,
}
pub fn chart_path(root: &Path, name: &str) -> Result<PathBuf> {
    let filename = crate::storage::chart_name(name)?;
    let candidate = root.join(filename);
    if !candidate
        .symlink_metadata()
        .map_err(|_| "Chart not found")?
        .file_type()
        .is_file()
    {
        return Err("Chart not found".into());
    }
    let path = candidate.canonicalize().map_err(|_| "Chart not found")?;
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
        return Err("Chart exceeds 20 MiB input limit".into());
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
        .env_remove("ETAG_SALT")
        .kill_on_drop(true);
    #[cfg(windows)]
    crate::worker_limits::configure(&mut command);
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
    #[cfg(windows)]
    let _job = match crate::worker_limits::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
    };
    let mut stdin = child.stdin.take().ok_or("Worker stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("Worker stdout unavailable")?;
    let payload = serde_json::to_vec(&job).map_err(|_| "Cannot encode worker input")?;
    drop(job);
    let result = tokio::time::timeout(timeout, async {
        stdin
            .write_all(&payload)
            .await
            .map_err(|_| "Worker input failed")?;
        drop(stdin);
        drop(payload);
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
    _permit: RenderPermit,
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
type ParsedParams = std::result::Result<Query<Params>, QueryRejection>;
#[derive(Clone, Copy)]
enum Mode {
    Preview,
    Judge,
    FastAttack,
}
async fn preview(State(app): State<App>, headers: HeaderMap, query: ParsedParams) -> Response {
    generate(app, headers, query, Mode::Preview, None).await
}
async fn judge(State(app): State<App>, headers: HeaderMap, query: ParsedParams) -> Response {
    generate(app, headers, query, Mode::Judge, None).await
}
async fn fastattack(State(app): State<App>, headers: HeaderMap, query: ParsedParams) -> Response {
    generate(app, headers, query, Mode::FastAttack, None).await
}
async fn named_preview(
    State(app): State<App>,
    UrlPath(name): UrlPath<String>,
    headers: HeaderMap,
    query: ParsedParams,
) -> Response {
    generate(app, headers, query, Mode::Preview, Some(name)).await
}
async fn named_fastattack(
    State(app): State<App>,
    UrlPath(name): UrlPath<String>,
    headers: HeaderMap,
    query: ParsedParams,
) -> Response {
    generate(app, headers, query, Mode::FastAttack, Some(name)).await
}
fn busy(reason: QueueError) -> Response {
    let message = match reason {
        QueueError::Full => "Render queue full; retry later",
        QueueError::TimedOut => "Render queue wait timed out; retry later",
    };
    let mut response = error(StatusCode::TOO_MANY_REQUESTS, message);
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
    response
}
fn read_error(reason: ReadError) -> Response {
    match reason {
        ReadError::NotFound => error(StatusCode::NOT_FOUND, "Chart not found"),
        ReadError::TooLarge => error(StatusCode::BAD_REQUEST, "Chart exceeds input limit"),
        ReadError::Invalid => error(StatusCode::UNPROCESSABLE_ENTITY, "Cannot read chart"),
    }
}
fn cache_headers(response: &mut Response, tag: &str) {
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(tag).expect("Generated ASCII ETag"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
}
fn not_modified(tag: &str) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    cache_headers(&mut response, tag);
    response
}
async fn generate(
    app: App,
    headers: HeaderMap,
    query: ParsedParams,
    mode: Mode,
    path_name: Option<String>,
) -> Response {
    let Ok(Query(mut p)) = query else {
        return error(StatusCode::BAD_REQUEST, "Invalid query parameters");
    };
    if let Some(name) = path_name {
        if p.name.is_some() {
            return error(StatusCode::BAD_REQUEST, "Specify chart name only once");
        }
        p.name = Some(name);
    }
    let Some(name) = p.name else {
        return error(StatusCode::BAD_REQUEST, "Chart name is required");
    };
    if crate::storage::chart_name(&name).is_err() {
        return error(StatusCode::NOT_FOUND, "Chart not found");
    }
    if [p.judge, p.easy, p.early_attack_only, p.fastattack, p.div]
        .into_iter()
        .flatten()
        .any(|v| v > 1)
    {
        return error(StatusCode::BAD_REQUEST, "Flags must be 0 or 1");
    }
    let mut format = p.format.unwrap_or_else(|| "png".into());
    if !matches!(format.as_str(), "png" | "jpg" | "jpeg") {
        return error(StatusCode::BAD_REQUEST, "Unsupported image format");
    }
    if format == "jpeg" {
        format = "jpg".into();
    }
    let zoom = p.zoom.unwrap_or(1.0);
    if !zoom.is_finite() || !(0.5..=2.0).contains(&zoom) || p.column == Some(0) {
        return error(StatusCode::BAD_REQUEST, "Invalid zoom or column");
    }
    let early = matches!(mode, Mode::FastAttack)
        || p.fastattack == Some(1)
        || p.early_attack_only == Some(1);
    let judge = !matches!(mode, Mode::Preview) || p.judge == Some(1) || early;
    let options = Options {
        judge,
        early_attack_only: early,
        easy: judge && p.easy == Some(1),
        divisions: p.div != Some(0),
        format,
        column: p.column,
        zoom,
    };
    // Conditional hits can bypass a busy render queue. Keep these reads bounded;
    // if all validation readers are busy, use the ordinary bounded queue.
    if headers.contains_key(header::IF_NONE_MATCH)
        && let Ok(read_permit) = app.reads.clone().try_acquire_owned()
    {
        let root = app.root.clone();
        let key = app.etag.clone();
        let read_name = name.clone();
        let hash_options = options.clone();
        let validation = tokio::task::spawn_blocking(move || {
            let result = root
                .read(&read_name)
                .map(|chart| key.tag(&chart, &hash_options));
            // Cancellation cannot release the read slot while I/O still runs.
            drop(read_permit);
            result
        })
        .await;
        match validation {
            Ok(Ok(tag)) if etag::matches(&headers, &tag) => return not_modified(&tag),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return read_error(e),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Cannot read chart"),
        }
    }
    let permit = match app.queue.acquire().await {
        Ok(permit) => permit,
        Err(reason) => return busy(reason),
    };
    // Read only after reaching the front of the queue. In particular, a chart
    // changed while waiting gets a fresh ETag from the exact bytes rendered.
    let root = app.root.clone();
    let key = app.etag.clone();
    let hash_options = options.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let result = root.read(&name).map(|chart| {
            let tag = key.tag(&chart, &hash_options);
            (chart, tag)
        });
        (result, permit)
    })
    .await;
    let Ok((prepared, permit)) = prepared else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "Cannot read chart");
    };
    let (chart, tag) = match prepared {
        Ok(pair) => pair,
        Err(e) => return read_error(e),
    };
    // Recheck validators after waiting, using the same bytes as the worker.
    if etag::matches(&headers, &tag) {
        return not_modified(&tag);
    }
    let content = if options.format == "png" {
        "image/png"
    } else {
        "image/jpeg"
    };
    match run_worker(Job { chart, options }, app.timeout).await {
        Ok(bytes) => {
            // A wildcard did not prove this chart previously rendered. Validate
            // it first, so malformed input cannot receive a successful 304.
            if etag::wildcard(&headers) {
                return not_modified(&tag);
            }
            let mut response = (
                [(header::CONTENT_TYPE, content)],
                axum::body::Body::new(ImageBody {
                    bytes: bytes.into(),
                    _permit: permit,
                }),
            )
                .into_response();
            cache_headers(&mut response, &tag);
            response
        }
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}
async fn response_policy(request: Request<axum::body::Body>, next: Next) -> Response {
    let log = crate::access_log::Entry::start(&request);
    let mut response = if request.uri().to_string().len() > 2048 {
        error(StatusCode::URI_TOO_LONG, "Request URI too long")
    } else if request
        .headers()
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .map(|v| v.as_bytes().len())
        .sum::<usize>()
        > 8192
    {
        error(
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            "Conditional header too large",
        )
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
    );
    if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    log.finish(&response);
    response
}

pub fn router(root: PathBuf, timeout: Duration) -> Result<Router> {
    router_with_config(&Config {
        chart_dir: root,
        timeout_seconds: timeout.as_secs(),
        ..Config::default()
    })
}
pub fn router_with_config(config: &Config) -> Result<Router> {
    config.validate()?;
    let app = App {
        root: Arc::new(ChartRoot::open(&config.chart_dir)?),
        etag: EtagKey::new(config.etag_salt.as_deref())?,
        reads: Arc::new(Semaphore::new(4)),
        queue: Arc::new(RenderQueue::new(
            config.render_workers,
            config.queue_capacity,
            Duration::from_secs(config.queue_timeout_seconds),
        )),
        timeout: Duration::from_secs(config.timeout_seconds),
    };
    Ok(Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/preview", get(preview))
        .route("/api/render", get(preview))
        .route("/api/render/{name}", get(named_preview))
        .route("/judge", get(judge))
        .route("/fastattack", get(fastattack))
        .route("/fastattack/{name}", get(named_fastattack))
        .fallback(|| async { error(StatusCode::NOT_FOUND, "Not found") })
        .layer(middleware::from_fn(response_policy))
        .with_state(app))
}
pub async fn serve(root: PathBuf, listen: &str, timeout: u64) -> Result<()> {
    serve_config(Config {
        chart_dir: root,
        listen: listen.into(),
        timeout_seconds: timeout,
        ..Config::default()
    })
    .await
}
pub async fn serve_config(config: Config) -> Result<()> {
    let app = router_with_config(&config)?;
    let listener = tokio::net::TcpListener::bind(&config.listen)
        .await
        .map_err(|_| "Cannot bind listen address")?;
    eprintln!(
        "Listening on {} ({} render workers, {} queued requests, {}s queue wait)",
        listener.local_addr().map_err(|_| "Address unavailable")?,
        config.render_workers,
        config.queue_capacity,
        config.queue_timeout_seconds,
    );
    if config.etag_salt.is_none() {
        eprintln!(
            "Using a per-process ETag salt; run init-config for validators that survive restarts."
        );
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| "HTTP server failed".into())
}
