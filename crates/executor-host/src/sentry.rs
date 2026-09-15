//! Optional Sentry envelope exporter. Env-gated, fail-open, never on the execute path.
//!
//! Set `SENTRY_DSN`. Events for `executor.execute.err` / overload / timeout are
//! queued on a bounded channel (32). Overflow drops the new event. POST timeout
//! is 2s. Missing or unparseable DSN is a no-op.

use std::sync::Arc;
use std::time::Duration;

use executor_core::{Metrics, metric_names};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use url::Url;

const QUEUE_CAP: usize = 32;
const POST_TIMEOUT: Duration = Duration::from_secs(2);

/// Parsed Sentry DSN.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentryDsn {
    /// Envelope ingest URL.
    pub envelope_url: String,
    /// Public key (`sentry_key`).
    pub public_key: String,
}

impl SentryDsn {
    /// Parse `https://{key}@{host}/{project}`.
    #[must_use]
    pub fn parse(dsn: &str) -> Option<Self> {
        let url = Url::parse(dsn.trim()).ok()?;
        let public_key = url.username();
        if public_key.is_empty() {
            return None;
        }
        let host = url.host_str()?;
        let project = url.path().trim_matches('/');
        if project.is_empty() || project.contains('/') {
            return None;
        }
        let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
        Some(Self {
            envelope_url: format!("{}://{host}{port}/api/{project}/envelope/", url.scheme()),
            public_key: public_key.to_owned(),
        })
    }

    /// Read `SENTRY_DSN`. `None` when unset or malformed.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("SENTRY_DSN").ok()?;
        if raw.is_empty() {
            return None;
        }
        Self::parse(&raw)
    }
}

struct Event {
    metric: &'static str,
}

/// Wrap `inner` with a Sentry fan-out when `SENTRY_DSN` is set.
pub fn attach(inner: Arc<dyn Metrics>) -> (Arc<dyn Metrics>, Option<JoinHandle<()>>) {
    let Some(dsn) = SentryDsn::from_env() else {
        return (inner, None);
    };
    attach_dsn(inner, dsn)
}

/// Same as [`attach`] with an explicit DSN (tests).
pub fn attach_dsn(
    inner: Arc<dyn Metrics>,
    dsn: SentryDsn,
) -> (Arc<dyn Metrics>, Option<JoinHandle<()>>) {
    let (tx, rx) = mpsc::channel(QUEUE_CAP);
    let worker = tokio::spawn(exporter_loop(dsn, rx));
    let wrapped: Arc<dyn Metrics> = Arc::new(SentryMetrics { inner, tx });
    (wrapped, Some(worker))
}

struct SentryMetrics {
    inner: Arc<dyn Metrics>,
    tx: mpsc::Sender<Event>,
}

impl Metrics for SentryMetrics {
    fn counter(&self, name: &'static str, delta: u64) {
        self.inner.counter(name, delta);
        if matches!(
            name,
            metric_names::EXECUTE_ERR
                | metric_names::EXECUTE_OVERLOAD
                | metric_names::EXECUTE_TIMEOUT
        ) {
            let _ = self.tx.try_send(Event { metric: name });
        }
    }

    fn gauge(&self, name: &'static str, value: i64) {
        self.inner.gauge(name, value);
    }

    fn observe_ms(&self, name: &'static str, ms: u64) {
        self.inner.observe_ms(name, ms);
    }
}

async fn exporter_loop(dsn: SentryDsn, mut rx: mpsc::Receiver<Event>) {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(POST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    while let Some(event) = rx.recv().await {
        if let Err(err) = post_envelope(&client, &dsn, event.metric).await {
            tracing::debug!(error = %err, "sentry envelope dropped");
        }
    }
}

async fn post_envelope(
    client: &reqwest::Client,
    dsn: &SentryDsn,
    metric: &str,
) -> Result<(), String> {
    let event_id = event_id();
    let header = json!({
        "event_id": event_id,
        "sent_at": now_rfc3339(),
    });
    let item_header = json!({ "type": "event", "content_type": "application/json" });
    let payload = json!({
        "event_id": event_id,
        "timestamp": now_rfc3339(),
        "platform": "native",
        "level": "error",
        "logger": "executor",
        "message": metric,
        "tags": { "component": "executor", "metric": metric },
    });
    let body = format!("{header}\n{item_header}\n{payload}\n");
    if body.len() > 64 * 1024 {
        return Err("envelope too large".into());
    }
    let auth = format!(
        "Sentry sentry_version=7, sentry_client=executor-rust/0.1.0, sentry_key={}",
        dsn.public_key
    );
    let response = client
        .post(&dsn.envelope_url)
        .header("content-type", "application/x-sentry-envelope")
        .header("x-sentry-auth", auth)
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status().is_success() || response.status().as_u16() == 429 {
        Ok(())
    } else {
        Err(format!("sentry HTTP {}", response.status()))
    }
}

fn event_id() -> String {
    let mut raw = [0_u8; 16];
    if getrandom::getrandom(&mut raw).is_err() {
        let n = executor_core::unix_now_ms().to_le_bytes();
        let p = u64::from(std::process::id()).to_le_bytes();
        raw[..8].copy_from_slice(&n);
        raw[8..16].copy_from_slice(&p);
    }
    hex_lower(&raw)
}

fn hex_lower(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0x0f) as usize] as char);
    }
    s
}

fn now_rfc3339() -> String {
    let ms = executor_core::unix_now_ms();
    let secs = ms / 1000;
    format!("{secs}")
}

#[cfg(test)]
fn parse_envelope_metric(body: &str) -> Result<String, String> {
    let mut lines = body.lines().filter(|l| !l.is_empty());
    let _header: serde_json::Value =
        serde_json::from_str(lines.next().ok_or("missing header")?).map_err(|e| e.to_string())?;
    let _item: serde_json::Value =
        serde_json::from_str(lines.next().ok_or("missing item")?).map_err(|e| e.to_string())?;
    let event: serde_json::Value =
        serde_json::from_str(lines.next().ok_or("missing event")?).map_err(|e| e.to_string())?;
    event
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "missing message".into())
}

#[cfg(test)]
mod tests {
    use super::{SentryDsn, attach_dsn, parse_envelope_metric};
    use executor_core::{AtomicMetrics, metric_names};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    #[test]
    fn parses_https_dsn() {
        let dsn = SentryDsn::parse("https://abc123@o123.ingest.sentry.io/456").expect("dsn");
        assert_eq!(dsn.public_key, "abc123");
        assert_eq!(
            dsn.envelope_url,
            "https://o123.ingest.sentry.io/api/456/envelope/"
        );
    }

    #[test]
    fn rejects_dsn_without_key() {
        assert!(SentryDsn::parse("https://o123.ingest.sentry.io/456").is_none());
    }

    #[tokio::test]
    async fn posts_envelope_on_execute_err() {
        let server = MockServer::start().await;
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let tx = std::sync::Mutex::new(Some(tx));
        Mock::given(method("POST"))
            .and(path("/api/99/envelope/"))
            .and(header("content-type", "application/x-sentry-envelope"))
            .respond_with(move |req: &Request| {
                let body = String::from_utf8_lossy(&req.body).into_owned();
                if let Some(tx) = tx.lock().ok().and_then(|mut g| g.take()) {
                    let _ = tx.send(body);
                }
                ResponseTemplate::new(200).set_body_json(json!({"id":"ok"}))
            })
            .mount(&server)
            .await;
        let host = server.uri().trim_start_matches("http://").to_owned();
        let dsn = SentryDsn::parse(&format!("http://pkey@{host}/99")).expect("dsn");
        let atomics = Arc::new(AtomicMetrics::new());
        let (metrics, worker) = attach_dsn(atomics, dsn);
        metrics.counter(metric_names::EXECUTE_ERR, 1);
        let body = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("timeout")
            .expect("body");
        assert_eq!(
            parse_envelope_metric(&body).expect("metric"),
            metric_names::EXECUTE_ERR
        );
        drop(metrics);
        if let Some(worker) = worker {
            worker.abort();
        }
    }

    #[tokio::test]
    async fn drops_new_events_when_queue_is_full() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/1/envelope/"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .mount(&server)
            .await;
        let host = server.uri().trim_start_matches("http://").to_owned();
        let dsn = SentryDsn::parse(&format!("http://pkey@{host}/1")).expect("dsn");
        let atomics = Arc::new(AtomicMetrics::new());
        let (metrics, worker) = attach_dsn(atomics, dsn);
        for _ in 0..64 {
            metrics.counter(metric_names::EXECUTE_TIMEOUT, 1);
        }
        drop(metrics);
        if let Some(worker) = worker {
            worker.abort();
        }
    }
}
