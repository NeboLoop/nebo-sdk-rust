# Nebo Rust SDK

Official Rust SDK for building [Nebo](https://neboloop.com) apps.

## Install

```toml
[dependencies]
nebo-sdk = "0.3"
```

## Quick Start

```rust
use async_trait::async_trait;
use nebo_sdk::{NeboApp, NeboError, SchemaBuilder};
use nebo_sdk::tool::ToolHandler;
use serde::Deserialize;
use serde_json::Value;

struct Calculator;

#[derive(Deserialize)]
struct Input { action: String, a: f64, b: f64 }

#[async_trait]
impl ToolHandler for Calculator {
    fn name(&self) -> &str { "calculator" }
    fn description(&self) -> &str { "Arithmetic calculator." }
    fn schema(&self) -> Value {
        SchemaBuilder::new(&["add", "subtract", "multiply", "divide"])
            .number("a", "First operand", true)
            .number("b", "Second operand", true)
            .build()
    }
    async fn execute(&self, input: Value) -> Result<String, NeboError> {
        let i: Input = serde_json::from_value(input).map_err(|e| NeboError::Execution(e.to_string()))?;
        let r = match i.action.as_str() {
            "add" => i.a + i.b,
            "subtract" => i.a - i.b,
            "multiply" => i.a * i.b,
            "divide" => i.a / i.b,
            _ => return Err(NeboError::Execution("unknown action".into())),
        };
        Ok(format!("{r}"))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    NeboApp::new()?.register_tool(Calculator).run().await?;
    Ok(())
}
```

Building needs no `protoc`: the generated gRPC code is vendored in
`src/pb/`. After changing a file in `proto/`, regenerate it with
`cargo run --manifest-path codegen/Cargo.toml` (that step needs `protoc`).

## How Nebo runs your app

`NeboApp::run` does all of this for you, the same on macOS, Linux and Windows:

- It listens on `127.0.0.1` only, on a port the OS picks, and prints
  `nebo-app-listening <address>` on stdout so Nebo knows where to connect.
  Nothing on another computer can reach it.
- It refuses every call that does not carry this launch's `NEBO_SIDECAR_TOKEN`
  as `authorization: Bearer <token>`. Nebo sends it on every call, including
  its health probe (`UIService.HealthCheck`). The SDK keeps it out of
  `app.env()`, and refuses the app's own `NEBO_APP_TOKEN` on incoming calls.
  It stays in the process environment, so a program your app starts inherits
  it: clear it there (`Command::env_remove("NEBO_SIDECAR_TOKEN")`).
- It answers Nebo's health probe with a proof: the probe carries a nonce in
  `x-nebo-challenge`, and the SDK answers with `x-nebo-proof` = hex
  HMAC-SHA256(key `NEBO_APP_TOKEN`, nonce || `NEBO_SIDECAR_TOKEN` || port as
  2 bytes big-endian). Nebo never sends `NEBO_APP_TOKEN`, so nothing that took
  over the app's port can answer it. The UI service is always served for this,
  even for an app with no UI handler. Your code never sees it.
- It returns when its stdin closes (Nebo holds the other end, so this is how
  the app notices Nebo is gone, however it went), on SIGTERM, or on Ctrl-C.

Your code reads the rest from `app.env()`:

| Field | Variable | What it is |
|---|---|---|
| `dir` | `NEBO_APP_DIR` | The app's code folder, replaced on every update |
| `data_dir` | `NEBO_DATA_DIR` | The app's own data folder, kept across updates; also the working directory |
| `api_url` | `NEBO_API_URL` | Nebo's local API, `http://127.0.0.1:<port>` |
| `token` | `NEBO_APP_TOKEN` | This launch's token for calling Nebo, outbound only: send it as `Authorization: Bearer <token>` to `api_url`. Required: the health proof is keyed with it |
| `id`, `name`, `version` | `NEBO_APP_ID`, `NEBO_APP_NAME`, `NEBO_APP_VERSION` | From the manifest |

Two tokens, one direction each: if another process ever got hold of the app's
port, the token Nebo sends there opens nothing in Nebo.

## Running the bot's plugins

A sidecar runs a plugin installed on the bot directly, with no AI in the
middle: the same command and flags the app's employee passes to its
`plugin__<slug>` tool, run by Nebo the same way (the installed version, the
employee's account, the same billing).

```rust
use nebo_sdk::plugins::{PluginCall, Plugins};

let plugins = Plugins::new(&nebo_sdk::AppEnv::load());
let strip = plugins
    .run("nebo-media", &PluginCall::new("strip").arg("in", "Media/Projects/reel/cut.mp4"), None)
    .await?;

// Long work: each line the plugin writes, as it writes it.
let mut show = |line: &str| println!("{line}");
let preview = plugins
    .run("nebo-media", &PluginCall::new("preview").arg("in", "cut.mp4").timeout(600), Some(&mut show))
    .await?;
```

- The app's manifest grants the plugin: `"plugin:nebo-media"` for every
  command, `"plugin:nebo-media:strip"` for one. An app installed from the
  marketplace needs the owner's OK first.
- `ok: false` in the result is the plugin's own failure, its words in
  `output`. A call Nebo refuses (403: not granted or not allowed; 404: not
  installed or turned off) is `NeboError::Refused` with Nebo's reason.
- The calls go to `api_url` with `NEBO_APP_TOKEN`:
  `POST /api/v1/apps/<id>/plugins/<slug>/run`, and for long work
  `GET /api/v1/apps/<id>/plugins/jobs/<job>` as an event stream.

## Documentation

See [Creating Nebo Apps](https://neboloop.com/developers) for the full guide.
