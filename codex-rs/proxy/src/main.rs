use std::collections::HashSet;
use std::net::SocketAddr;

use anyhow::Context;
use axum::Router;
use axum::body::Body as AxumBody;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderName;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response as AxumResponse;
use axum::routing::any;
use axum::routing::get;

use clap::Parser;
use codex_arg0::arg0_dispatch_or_else;
use codex_common::CliConfigOverrides;
use codex_core::ModelProviderInfo;
use codex_core::auth::CodexAuth;
use codex_core::config::Config;
use codex_core::default_client::create_client;
use futures_util::TryStreamExt;
use reqwest::Client;
use serde::Serialize;
use std::io;
use tokio::net::TcpListener;
use tower_http::cors::Any;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug, Clone)]
#[command(name = "codex-proxy", about = "OpenAI-compatible HTTP passthrough API")]
struct Args {
    /// Bind address, e.g. 127.0.0.1:11435 or 0.0.0.0:11435
    #[arg(long, default_value_t = String::from("127.0.0.1:11435"))]
    bind: String,

    /// Enable permissive CORS (Access-Control-Allow-Origin: *)
    #[arg(long, default_value_t = false)]
    allow_cors_any: bool,

    /// Config overrides (merged into ~/.codex/config.toml)
    #[clap(flatten)]
    config: CliConfigOverrides,
}

#[derive(Clone)]
struct AppState {
    client: Client,
    upstream_base_v1: String,
    provider: ModelProviderInfo,
    auth: Option<CodexAuth>,
    hop_by_hop: HashSet<HeaderName>,
}

// Stream request bodies rather than buffering to support large uploads.

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|_codex_linux_sandbox_exe| async move { run().await })
}

async fn run() -> anyhow::Result<()> {
    // Logging
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    // Load Config with -c overrides
    let cli_overrides = args
        .config
        .parse_overrides()
        .map_err(|e| anyhow::anyhow!(e))?;
    let config = Config::load_with_cli_overrides(cli_overrides, Default::default())
        .context("load config")?;

    // Upstream base URL: prefer provider.base_url, else default to OpenAI official
    let upstream_base_v1 = normalize_base_v1(&config.model_provider);

    // Preload auth (if available). We only inject when the incoming request lacks Authorization.
    let auth = CodexAuth::from_codex_home(&config.codex_home)
        .ok()
        .flatten();

    // Share state
    let state = AppState {
        client: create_client(),
        upstream_base_v1,
        provider: config.model_provider.clone(),
        auth,
        hop_by_hop: hop_by_hop_set(),
    };

    // Router
    let mut router = Router::new()
        .route(
            "/health",
            get(|| async { axum::Json(Health { status: "ok" }) }),
        )
        .route("/v1/*tail", any(proxy_v1));

    // CORS + Trace
    let cors = if args.allow_cors_any {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        CorsLayer::permissive()
    };
    router = router.layer(TraceLayer::new_for_http()).layer(cors);

    let listener: TcpListener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("bind {}", args.bind))?;
    let addr: SocketAddr = listener.local_addr()?;
    info!("codex-proxy listening on http://{}", addr);

    axum::serve(listener, router.with_state(state)).await?;
    Ok(())
}

/// Proxy handler for /v1/* routes.
async fn proxy_v1(
    State(state): State<AppState>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: AxumBody,
) -> Result<AxumResponse, AxumResponse> {
    let path_and_query = uri
        .path_and_query()
        .cloned()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing path").into_response())?;

    let full_path = path_and_query.as_str();
    // Compose upstream URL by replacing the leading /v1 with upstream base.
    let tail = if full_path.len() <= 3 {
        ""
    } else {
        &full_path[3..]
    }; // keep leading '/'
    let upstream_url = format!("{}{}", state.upstream_base_v1, tail);

    // Build upstream request
    let mut builder = state
        .client
        .request(method_to_reqwest(&method), &upstream_url);

    // Copy headers except hop-by-hop; we'll set Host automatically
    let mut has_auth_header = false;
    for (name, value) in headers.iter() {
        if state.hop_by_hop.contains(name) {
            continue;
        }
        if name.as_str().eq_ignore_ascii_case("host") {
            continue;
        }
        if name.as_str().eq_ignore_ascii_case("authorization") {
            has_auth_header = true;
        }
        builder = builder.header(name.as_str(), value);
    }

    // Provider extra headers from config (static + env-bound)
    if let Some(extra) = &state.provider.http_headers {
        for (k, v) in extra {
            builder = builder.header(k, v);
        }
    }
    if let Some(env_headers) = &state.provider.env_http_headers {
        for (header, env_var) in env_headers {
            if let Ok(val) = std::env::var(env_var)
                && !val.trim().is_empty()
            {
                builder = builder.header(header, val);
            }
        }
    }

    // Inject Authorization if missing
    if !has_auth_header {
        if let Ok(Some(key)) = state.provider.api_key() {
            builder = builder.bearer_auth(key);
        } else if let Some(auth) = &state.auth
            && let Ok(token) = auth.get_token().await
        {
            builder = builder.bearer_auth(token);
        }
    }

    if requires_body(&method) {
        // Stream request body to upstream (supports large multipart uploads).
        let stream = body
            .into_data_stream()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e));
        let reqwest_body = reqwest::Body::wrap_stream(stream);
        builder = builder.body(reqwest_body);
    }

    // Send
    let upstream = builder.send().await.map_err(internal_error)?;

    // Build response
    let status = upstream.status();
    let mut resp_builder = AxumResponse::builder().status(status);
    let Some(headers_out) = resp_builder.headers_mut() else {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to build headers").into_response());
    };
    copy_response_headers(upstream.headers(), headers_out, &state.hop_by_hop);

    let stream = upstream.bytes_stream().map_ok(|chunk| chunk);
    let body = AxumBody::from_stream(stream);
    match resp_builder.body(body) {
        Ok(resp) => Ok(resp.into_response()),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to build response body: {e}"),
        )
            .into_response()),
    }
}

fn method_to_reqwest(method: &Method) -> reqwest::Method {
    match *method {
        Method::GET => reqwest::Method::GET,
        Method::POST => reqwest::Method::POST,
        Method::PUT => reqwest::Method::PUT,
        Method::DELETE => reqwest::Method::DELETE,
        Method::PATCH => reqwest::Method::PATCH,
        Method::HEAD => reqwest::Method::HEAD,
        Method::OPTIONS => reqwest::Method::OPTIONS,
        Method::CONNECT => reqwest::Method::CONNECT,
        Method::TRACE => reqwest::Method::TRACE,
        _ => {
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET)
        }
    }
}

fn requires_body(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn copy_response_headers(src: &HeaderMap, dst: &mut HeaderMap, hop_by_hop: &HashSet<HeaderName>) {
    for (name, value) in src.iter() {
        if hop_by_hop.contains(name) {
            continue;
        }
        if name.as_str().eq_ignore_ascii_case("content-length") {
            // Let hyper compute this for streaming bodies.
            continue;
        }
        if let Ok(cloned) = HeaderValue::from_bytes(value.as_bytes()) {
            dst.insert(name.clone(), cloned);
        }
    }
}

fn hop_by_hop_set() -> HashSet<HeaderName> {
    [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ]
    .into_iter()
    .filter_map(|s| HeaderName::from_lowercase(s.as_bytes()).ok())
    .collect()
}

fn normalize_base_v1(provider: &ModelProviderInfo) -> String {
    // Determine base root from provider.base_url or defaults.
    // Ensure it ends with "/v1" exactly once, without trailing slash.
    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let mut trimmed = base.trim_end_matches('/').to_string();
    if !trimmed.ends_with("/v1") {
        trimmed = format!("{trimmed}/v1");
    }
    trimmed
}

fn internal_error<E: std::fmt::Display>(err: E) -> AxumResponse {
    (StatusCode::BAD_GATEWAY, format!("Upstream error: {err}")).into_response()
}
