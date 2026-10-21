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

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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
        .env("NEBO_APP_TOKEN", TOKEN)
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
    let ok = name(addr, Some(&format!("Bearer {TOKEN}"))).await.expect("the launch token");
    assert_eq!(ok, "calculator");

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
fn it_will_not_start_without_a_token() {
    let out = Command::new(example())
        .env_clear()
        .stdin(Stdio::piped())
        .output()
        .expect("run the example");
    assert!(!out.status.success(), "it refuses to start");
    assert!(out.stdout.is_empty(), "nothing announced, so nothing listens");
}
