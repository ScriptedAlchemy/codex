use assert_cmd::cargo::CommandCargoExt;
use reqwest::Client;
use reqwest::multipart;
use std::time::Duration;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn proxies_chat_completions_with_auth_passthrough() -> anyhow::Result<()> {
    // Upstream mock provider
    let upstream = MockServer::start().await;
    let template = ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "id": "test",
        "object": "chat.completion",
        "choices": [ { "index": 0, "message": {"role":"assistant","content":"hi"}, "finish_reason": "stop" } ]
    }));
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(template)
        .mount(&upstream)
        .await;

    // Pick an unused local port and spawn the proxy binary
    let port = portpicker::pick_unused_port().expect("pick port");

    // Temp CODEX_HOME to avoid touching user files
    let codex_home = TempDir::new()?;

    let mut cmd = std::process::Command::cargo_bin("codex-proxy")?;
    cmd.arg("--bind")
        .arg(format!("127.0.0.1:{port}"))
        .arg("-c")
        .arg(format!(
            "model_providers.mock={{ name = \"mock\", base_url = \"{}/v1\", wire_api = \"chat\" }}",
            upstream.uri()
        ))
        .arg("-c")
        .arg("model_provider=\"mock\"")
        .env("CODEX_HOME", codex_home.path())
        .env("RUST_LOG", "info");

    let mut child = cmd.spawn()?;

    // Probe until ready
    let client = Client::builder().timeout(Duration::from_secs(2)).build()?;
    let base = format!("http://127.0.0.1:{port}");
    for _ in 0..50u8 {
        if client.get(format!("{base}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Make a proxied request
    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "model": "gpt-4o-mini",
            "messages": [{"role":"user","content":"hello"}]
        }))
        .send()
        .await?;
    assert!(resp.status().is_success());
    let val: serde_json::Value = resp.json().await?;
    assert_eq!(val["choices"][0]["message"]["content"], "hi");

    let _ = child.kill();
    Ok(())
}

#[tokio::test]
async fn proxies_responses_with_injected_auth() -> anyhow::Result<()> {
    let upstream = MockServer::start().await;
    let template = ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "id": "resp_123",
        "output": [{ "type": "message", "content": [{ "type": "output_text", "text": "ok" }] }]
    }));
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(template)
        .mount(&upstream)
        .await;

    let port = portpicker::pick_unused_port().expect("pick port");
    let codex_home = TempDir::new()?;
    let mut cmd = std::process::Command::cargo_bin("codex-proxy")?;
    cmd.arg("--bind")
        .arg(format!("127.0.0.1:{port}"))
        .arg("-c")
        .arg(format!(
            "model_providers.mock={{ name = \"mock\", base_url = \"{}/v1\", env_key = \"MOCK_API_KEY\", wire_api = \"responses\" }}",
            upstream.uri()
        ))
        .arg("-c")
        .arg("model_provider=\"mock\"")
        .env("CODEX_HOME", codex_home.path())
        .env("MOCK_API_KEY", "sk-test");
    let mut child = cmd.spawn()?;

    let client = Client::builder().timeout(Duration::from_secs(3)).build()?;
    let base = format!("http://127.0.0.1:{port}");
    for _ in 0..50u8 {
        if client.get(format!("{base}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let resp = client
        .post(format!("{base}/v1/responses"))
        .json(&serde_json::json!({
            "model": "gpt-4o-mini",
            "input": [{ "role": "user", "content": [{ "type": "input_text", "text": "ping" }] }]
        }))
        .send()
        .await?;
    assert!(resp.status().is_success());
    let val: serde_json::Value = resp.json().await?;
    assert_eq!(val["output"][0]["content"][0]["text"], "ok");

    let _ = child.kill();
    Ok(())
}

#[tokio::test]
async fn proxies_multipart_file_upload() -> anyhow::Result<()> {
    // Upstream mock server that records requests
    let upstream = MockServer::start().await;
    let template = ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "id": "file_123",
        "object": "file",
        "filename": "image.jpg"
    }));
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(template)
        .mount(&upstream)
        .await;

    // Start proxy on a free port
    let port = portpicker::pick_unused_port().expect("pick port");
    let codex_home = TempDir::new()?;
    let mut cmd = std::process::Command::cargo_bin("codex-proxy")?;
    cmd.arg("--bind")
        .arg(format!("127.0.0.1:{port}"))
        .arg("-c")
        .arg(format!(
            "model_providers.mock={{ name = \"mock\", base_url = \"{}/v1\", wire_api = \"chat\" }}",
            upstream.uri()
        ))
        .arg("-c")
        .arg("model_provider=\"mock\"")
        .env("CODEX_HOME", codex_home.path());
    let mut child = cmd.spawn()?;

    // Wait for health
    let client = Client::builder().timeout(Duration::from_secs(3)).build()?;
    let base = format!("http://127.0.0.1:{port}");
    for _ in 0..50u8 {
        if client.get(format!("{base}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Build a ~1.5MiB payload to ensure we exercise streaming path
    let data = vec![0u8; 1_500_000];
    let part = multipart::Part::bytes(data)
        .file_name("image.jpg")
        .mime_str("image/jpeg")?;
    let form = multipart::Form::new()
        .text("purpose", "assistants")
        .part("file", part);

    let resp = client
        .post(format!("{base}/v1/files"))
        .header("Authorization", "Bearer test-token")
        .multipart(form)
        .send()
        .await?;
    assert!(resp.status().is_success());

    // Inspect what the upstream saw
    let requests = upstream
        .received_requests()
        .await
        .expect("failed to fetch received requests");
    let upload = requests
        .iter()
        .find(|r| r.url.path() == "/v1/files" && r.method.as_str() == "POST")
        .expect("upstream received upload");
    let ct = upload
        .headers
        .get("content-type")
        .expect("content-type present");
    let ct_val = ct.to_str().unwrap();
    assert!(ct_val.starts_with("multipart/form-data"));
    assert!(upload.body.len() > 1_000_000);

    let _ = child.kill();
    Ok(())
}
