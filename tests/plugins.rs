//! `Plugins::run` against a stand-in for Nebo's plugin routes: the call goes
//! to the app's own route with its `NEBO_APP_TOKEN`, a refusal comes back
//! with Nebo's reason, and long work reports each line before its result.

use std::sync::{Arc, Mutex};

use nebo_sdk::plugins::{PluginCall, PluginResult, Plugins};
use nebo_sdk::{AppEnv, NeboError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TOKEN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

/// One request as the stand-in saw it: its first line, its token, its body.
#[derive(Debug, Clone)]
struct Seen {
    line: String,
    bearer: String,
    accept: String,
    body: serde_json::Value,
}

/// A stand-in Nebo on loopback that answers each request with `answer`.
async fn nebo(answer: fn(&Seen) -> String) -> (AppEnv, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            let (head, body) = loop {
                let n = sock.read(&mut buf).await.unwrap();
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if body.len() >= len {
                        break (head.to_string(), body.to_string());
                    }
                }
            };
            let header = |name: &str| {
                head.lines()
                    .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()))
                    .unwrap_or_default()
            };
            let s = Seen {
                line: head.lines().next().unwrap().to_string(),
                bearer: header("authorization"),
                accept: header("accept"),
                body: serde_json::from_str(&body).unwrap_or_default(),
            };
            log.lock().unwrap().push(s.clone());
            sock.write_all(answer(&s).as_bytes()).await.unwrap();
            sock.shutdown().await.unwrap();
        }
    });
    let env = AppEnv {
        dir: String::new(),
        id: "studio".into(),
        name: "Studio".into(),
        version: "1.0.0".into(),
        data_dir: String::new(),
        api_url: format!("http://{addr}"),
        token: TOKEN.into(),
    };
    (env, seen)
}

fn reply(status: &str, kind: &str, body: &str) -> String {
    format!("HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
}

#[tokio::test]
async fn a_command_runs_on_the_apps_own_route_with_its_token() {
    let (env, seen) = nebo(|_| reply("200 OK", "application/json", r#"{"ok":true,"output":"Result: /x/strip.jpg","files":["/x/strip.jpg"]}"#)).await;
    let call = PluginCall::new("strip").arg("every", 2).timeout(30);
    let out = Plugins::new(&env).run("nebo-media", &call, None).await.unwrap();
    assert_eq!(out, PluginResult { ok: true, output: "Result: /x/strip.jpg".into(), files: vec!["/x/strip.jpg".into()] });
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].line, "POST /api/v1/apps/studio/plugins/nebo-media/run HTTP/1.1");
    assert_eq!(seen[0].bearer, format!("Bearer {TOKEN}"));
    assert_eq!(seen[0].body, serde_json::json!({"command": "strip", "args": {"every": 2}, "timeout": 30, "background": false}));
}

#[tokio::test]
async fn a_refusal_carries_nebos_reason() {
    let (env, _) = nebo(|_| reply("403 Forbidden", "text/plain", "Studio wants to use Nebo Media, and the owner has not allowed it.")).await;
    match Plugins::new(&env).run("nebo-media", &PluginCall::new("strip"), None).await {
        Err(NeboError::Refused { status: 403, reason }) => assert!(reason.contains("wants to use Nebo Media"), "{reason}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn long_work_reports_each_line_then_its_result() {
    let (env, seen) = nebo(|s| {
        if s.line.starts_with("POST") {
            return reply("202 Accepted", "application/json", r#"{"job":"j1"}"#);
        }
        let events = "event: progress\ndata: frame 1 of 2\n\nevent: progress\ndata: frame 2 of 2\n\nevent: result\ndata: {\"ok\":true,\"output\":\"done\",\"files\":[]}\n\n";
        reply("200 OK", "text/event-stream", events)
    })
    .await;
    let mut lines = Vec::new();
    let mut progress = |l: &str| lines.push(l.to_string());
    let out = Plugins::new(&env).run("nebo-media", &PluginCall::new("preview"), Some(&mut progress)).await.unwrap();
    assert_eq!(out, PluginResult { ok: true, output: "done".into(), files: vec![] });
    assert_eq!(lines, ["frame 1 of 2", "frame 2 of 2"]);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].body["background"], true);
    assert_eq!(seen[1].line, "GET /api/v1/apps/studio/plugins/jobs/j1 HTTP/1.1");
    assert_eq!((seen[1].bearer.as_str(), seen[1].accept.as_str()), (format!("Bearer {TOKEN}").as_str(), "text/event-stream"));
}
