// SPDX-License-Identifier: AGPL-3.0-only
use axum::{
    extract::Query,
    http::{Request, Uri},
    response::Response,
};
use chrono::Local;
use http_body::Body;
use std::{
    io::Write,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    time::Instant,
};

const MAX_TARGET: usize = 2048;
const BUFFER_LINES: usize = 512;
static WRITER: OnceLock<Option<SyncSender<String>>> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Entry {
    request: String,
    started: Instant,
}

impl Entry {
    pub(crate) fn start<B>(request: &Request<B>) -> Self {
        let method = if request.method().as_str().len() <= 32 {
            request.method().as_str()
        } else {
            "[method-too-long]"
        };
        Self {
            request: format!("{method} {} {:?}", target(request.uri()), request.version()),
            started: Instant::now(),
        }
    }

    pub(crate) fn finish(self, response: &Response) {
        let bytes = response
            .body()
            .size_hint()
            .exact()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".into());
        let line = format!(
            "{}: [render] \"{}\" {} {} {:.1}ms",
            Local::now().format("%Y-%m-%dT%H:%M:%S%:z"),
            self.request,
            response.status().as_u16(),
            bytes,
            self.started.elapsed().as_secs_f64() * 1000.0,
        );
        if let Some(writer) = WRITER.get_or_init(|| {
            let (sender, receiver) = sync_channel::<String>(BUFFER_LINES);
            let thread = std::thread::Builder::new()
                .name("access-log".into())
                .spawn(move || {
                    for line in receiver {
                        let skipped = DROPPED.swap(0, Ordering::Relaxed);
                        let mut stderr = std::io::stderr().lock();
                        if skipped > 0 {
                            let _ = writeln!(
                                stderr,
                                "{}: [render] log buffer full; skipped {skipped} lines",
                                Local::now().format("%Y-%m-%dT%H:%M:%S%:z")
                            );
                        }
                        // Logging failures must not fail requests or panic the server.
                        let _ = writeln!(stderr, "{line}");
                    }
                });
            match thread {
                Ok(_) => Some(sender),
                Err(_) => {
                    let _ = writeln!(std::io::stderr(), "Cannot start access log writer");
                    None
                }
            }
        }) && writer.try_send(line).is_err()
        {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn target(uri: &Uri) -> String {
    if uri
        .path_and_query()
        .is_some_and(|p| p.as_str().len() > MAX_TARGET)
    {
        return "[uri-too-long]".into();
    }
    let mut target = uri.path().to_owned();
    if uri.query().is_some() {
        target.push('?');
        match Query::<Vec<(String, String)>>::try_from_uri(uri) {
            Ok(Query(fields)) => {
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        target.push('&');
                    }
                    if matches!(
                        key.as_str(),
                        "name"
                            | "judge"
                            | "fastattack"
                            | "early_attack_only"
                            | "div"
                            | "easy"
                            | "format"
                            | "column"
                            | "zoom"
                    ) {
                        target.push_str(key);
                        target.push('=');
                        // Keep values URL-encoded: no decoded terminal controls or
                        // line breaks, and no unbounded values in a log record.
                        for byte in value.bytes().take(128) {
                            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                                target.push(byte as char);
                            } else {
                                use std::fmt::Write;
                                let _ = write!(target, "%{byte:02X}");
                            }
                        }
                        if value.len() > 128 {
                            target.push_str("...");
                        }
                    } else {
                        // Unknown query names can themselves contain secrets.
                        target.push_str("[redacted]");
                    }
                }
            }
            Err(_) => target.push_str("[invalid-query]"),
        }
    }
    let escaped: String = target.escape_default().take(MAX_TARGET).collect();
    if target.escape_default().count() > MAX_TARGET {
        escaped + "..."
    } else {
        escaped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_parameters_are_visible_but_unknown_secrets_are_redacted() {
        let uri: Uri =
            "/api/render?div=0&judge=1&name=1086_03&%74oken=private-value&private-key=secret"
                .parse()
                .unwrap();
        assert_eq!(
            target(&uri),
            "/api/render?div=0&judge=1&name=1086_03&[redacted]&[redacted]"
        );
    }

    #[test]
    fn control_characters_and_long_targets_cannot_forge_log_lines() {
        let uri: Uri = "/judge?name=%1B%5B2J%0Aforged%22&div=0".parse().unwrap();
        let logged = target(&uri);
        assert!(!logged.contains(['\n', '\r', '\x1b', '"']));
        assert!(logged.contains("%1B%5B2J%0Aforged%22"));
        let uri: Uri = format!("/api/render?name={}", "x".repeat(3000))
            .parse()
            .unwrap();
        assert_eq!(target(&uri), "[uri-too-long]");
    }
}
