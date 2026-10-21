//! The sidecar contract, proved on a real process (the `calculator` example,
//! which `cargo test` builds): it listens on loopback and says where, it
//! refuses calls without its launch token, and it exits when Nebo stops it —
//! its stdin closing included.

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// Generated client stubs (codegen/): the tests use the tool service's, not
// every message type in the file.
#[allow(dead_code)]
mod pb {
    include!("pb/apps.v0.rs");
}

const SIDECAR_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const APP_TOKEN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn example() -> PathBuf {
    let deps = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let bin = deps.parent().unwrap().join("examples").join(format!("calculator{}", std::env::consts::EXE_SUFFIX));
    assert!(bin.exists(), "the calculator example is built by `cargo test`: {}", bin.display());
    bin
}

/// Launch the example the way Nebo does: a cleared environment, stdin and
/// stdout piped. Returns it and the address it announced.
fn launch() -> (Child, SocketAddr) {
    let data = tempfile_dir();
    let mut cmd = Command::new(example());
    cmd.env_clear()
        .env("NEBO_APP_ID", "calc")
        .env("NEBO_APP_NAME", "Calculator")
        .env("NEBO_APP_VERSION", "1.0.0")
        .env("NEBO_APP_DIR", &data)
        .env("NEBO_DATA_DIR", &data)
        .env("NEBO_API_URL", "http://127.0.0.1:27895")
        .env("NEBO_APP_TOKEN", APP_TOKEN)
        .env("NEBO_SIDECAR_TOKEN", SIDECAR_TOKEN)
        .current_dir(&data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Ok(root) = std::env::var("SystemRoot") {
        cmd.env("SystemRoot", root);
    }
    let mut child = cmd.spawn().expect("spawn the example");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).expect("read its stdout");
    let addr: SocketAddr = line
        .trim_end()
        .strip_prefix(nebo_sdk::app::LISTENING)
        .unwrap_or_else(|| panic!("the first line announces the address: {line:?}"))
        .parse()
        .expect("an address");
    (child, addr)
}

fn tempfile_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nebo-sdk-test-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn exits_within(child: &mut Child, secs: u64) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

async fn name(addr: SocketAddr, bearer: Option<&str>) -> Result<String, tonic::Status> {
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}")).unwrap().connect_lazy();
    let mut client = pb::tool_service_client::ToolServiceClient::new(channel);
    let mut req = tonic::Request::new(pb::Empty {});
    if let Some(b) = bearer {
        req.metadata_mut().insert("authorization", b.parse().unwrap());
    }
    client.name(req).await.map(|r| r.into_inner().name)
}

#[tokio::test]
async fn it_listens_on_loopback_and_refuses_calls_without_the_token() {
    let (mut child, addr) = launch();
    assert!(addr.ip().is_loopback(), "{addr}");
    assert_ne!(addr.port(), 0);

    let none = name(addr, None).await.expect_err("no token");
    assert_eq!(none.code(), tonic::Code::Unauthenticated, "{none:?}");
    let wrong = name(addr, Some("Bearer not-the-token")).await.expect_err("another token");
    assert_eq!(wrong.code(), tonic::Code::Unauthenticated, "{wrong:?}");
    let outbound = name(addr, Some(&format!("Bearer {APP_TOKEN}"))).await.expect_err("the app's outbound token");
    assert_eq!(outbound.code(), tonic::Code::Unauthenticated, "{outbound:?}");
    let ok = name(addr, Some(&format!("Bearer {SIDECAR_TOKEN}"))).await.expect("the sidecar token");
    assert_eq!(ok, "calculator");

    // Nebo's health probe is token-gated too.
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}")).unwrap().connect_lazy();
    let mut ui = pb::ui_service_client::UiServiceClient::new(channel);
    let refused = ui.health_check(tonic::Request::new(pb::HealthCheckRequest {})).await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::Unauthenticated);

    child.kill().unwrap();
    child.wait().unwrap();
}

fn hmac(key: &str, nonce: &[u8], sidecar_token: &str, port: u16) -> Vec<u8> {
    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(nonce);
    mac.update(sidecar_token.as_bytes());
    mac.update(&port.to_be_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Nebo's health probe as Nebo sends it: the sidecar token and a fresh
/// nonce. The real app (the calculator has no UI handler; the SDK serves the
/// health check for it anyway) answers with the proof, computed here
/// independently from the address it announced. A process that saw only the
/// sidecar token — an imposter on the app's port — could at best key the
/// HMAC with that, and that is not the proof.
#[tokio::test]
async fn it_proves_itself_to_nebos_health_probe() {
    let (mut child, addr) = launch();
    let nonce: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(37).wrapping_add(child.id() as u8)).collect();
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}")).unwrap().connect_lazy();
    let mut ui = pb::ui_service_client::UiServiceClient::new(channel);
    let mut req = tonic::Request::new(pb::HealthCheckRequest {});
    req.metadata_mut().insert("authorization", format!("Bearer {SIDECAR_TOKEN}").parse().unwrap());
    req.metadata_mut().insert("x-nebo-challenge", hex(&nonce).parse().unwrap());
    let answer = ui.health_check(req).await.expect("the probe is answered");
    let presented = answer.metadata().get("x-nebo-proof").expect("with a proof").to_str().unwrap().to_string();
    assert_eq!(presented, hex(&hmac(APP_TOKEN, &nonce, SIDECAR_TOKEN, addr.port())), "the proof Nebo checks");
    assert_ne!(presented, hex(&hmac(SIDECAR_TOKEN, &nonce, SIDECAR_TOKEN, addr.port())), "an imposter's proof differs");
    assert!(answer.into_inner().healthy);

    // No UI handler: the app's own requests are not served.
    let mut req = tonic::Request::new(pb::HttpRequest { method: "GET".into(), path: "x".into(), ..Default::default() });
    req.metadata_mut().insert("authorization", format!("Bearer {SIDECAR_TOKEN}").parse().unwrap());
    assert_eq!(ui.handle_request(req).await.unwrap_err().code(), tonic::Code::Unimplemented);

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn it_exits_when_its_stdin_closes() {
    let (mut child, addr) = launch();
    drop(child.stdin.take());
    let status = exits_within(&mut child, 10).expect("it exits on its own once Nebo's end closes");
    assert!(status.success(), "{status:?}");
    assert!(std::net::TcpStream::connect(addr).is_err(), "nothing listens");
}

#[cfg(unix)]
#[test]
fn it_exits_when_nebo_stops_it() {
    let (mut child, addr) = launch();
    sigterm(child.id());
    let status = exits_within(&mut child, 10).expect("it exits on SIGTERM");
    assert!(status.success(), "{status:?}");
    assert!(std::net::TcpStream::connect(addr).is_err(), "nothing listens");
}

#[cfg(unix)]
fn sigterm(pid: u32) {
    let ok = Command::new("kill").arg("-TERM").arg(pid.to_string()).status().unwrap();
    assert!(ok.success());
}

#[test]
fn it_will_not_start_without_both_tokens() {
    for (name, value) in [("NEBO_APP_TOKEN", APP_TOKEN), ("NEBO_SIDECAR_TOKEN", SIDECAR_TOKEN)] {
        let out = Command::new(example())
            .env_clear()
            .env(name, value)
            .stdin(Stdio::piped())
            .output()
            .expect("run the example");
        assert!(!out.status.success(), "it refuses to start with only {name}");
        assert!(out.stdout.is_empty(), "nothing announced, so nothing listens");
    }
}
