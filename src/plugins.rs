//! Run a plugin installed on the bot directly, with no AI in the middle.
//!
//! The call is the one the app's employee makes with its `plugin__<slug>`
//! tool (a command and its flags). Nebo runs it the same way: the installed
//! version, the employee's account and the same billing. The result comes
//! straight back. The app's manifest must grant the plugin
//! (`plugin:<slug>`, or `plugin:<slug>:<command>` for one command), and an
//! app installed from the marketplace needs the owner's OK first.
//!
//! ```no_run
//! # async fn strip() -> Result<(), nebo_sdk::NeboError> {
//! use nebo_sdk::plugins::{PluginCall, Plugins};
//!
//! let plugins = Plugins::new(&nebo_sdk::AppEnv::load());
//! let call = PluginCall::new("strip").arg("in", "Media/Projects/reel/cut.mp4").arg("every", 2);
//! let strip = plugins.run("nebo-media", &call, None).await?;
//! println!("{:?}", strip.files);
//! # Ok(()) }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::env::AppEnv;
use crate::error::NeboError;

/// One command for a plugin: the subcommand, its flags (each key becomes
/// `--key`; a number, bool, object or list goes as its JSON) and how long
/// it may take.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PluginCall {
    pub command: String,
    pub args: Map<String, Value>,
    /// Seconds the command may take: 120 unless given, at most 600 (Nebo
    /// holds an app's run to that).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl PluginCall {
    pub fn new(command: impl Into<String>) -> Self {
        Self { command: command.into(), ..Default::default() }
    }

    pub fn arg(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.args.insert(key.into(), value.into());
        self
    }

    pub fn timeout(mut self, seconds: u64) -> Self {
        self.timeout = Some(seconds);
        self
    }
}

/// What a plugin command came to.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PluginResult {
    /// The plugin succeeded.
    pub ok: bool,
    /// What it printed: the text the employee's tool gets back.
    pub output: String,
    /// The files it reported making (its `Result:` lines).
    pub files: Vec<String>,
}

/// The app's door to the bot's plugins, with this launch's `NEBO_APP_TOKEN`.
#[derive(Debug, Clone)]
pub struct Plugins {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Plugins {
    pub fn new(env: &AppEnv) -> Self {
        Self {
            base: format!("{}/api/v1/apps/{}/plugins", env.api_url.trim_end_matches('/'), env.id),
            token: env.token.clone(),
            http: reqwest::Client::new(),
        }
    }

    /// Run `call` on the installed plugin `slug`. With `progress`, the
    /// command runs as a job on the bot and `progress` is called with each
    /// line the plugin writes, as it writes it; `run` returns when the job
    /// ends. A call Nebo refuses is [`NeboError::Refused`] with its reason;
    /// the plugin's own failure is a result with `ok: false`.
    pub async fn run(
        &self,
        slug: &str,
        call: &PluginCall,
        progress: Option<&mut (dyn FnMut(&str) + Send)>,
    ) -> Result<PluginResult, NeboError> {
        let mut body = serde_json::to_value(call).map_err(|e| NeboError::Other(e.to_string()))?;
        body["background"] = Value::Bool(progress.is_some());
        let resp = self
            .http
            .post(format!("{}/{slug}/run", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(http_error)?;
        let resp = ok(resp).await?;
        let Some(progress) = progress else {
            return resp.json().await.map_err(http_error);
        };
        let job: Value = resp.json().await.map_err(http_error)?;
        let job = job["job"].as_str().unwrap_or_default();
        self.follow(job, progress).await
    }

    /// Read a job's event stream to its `result`, handing on each
    /// `progress` line.
    async fn follow(&self, job: &str, progress: &mut (dyn FnMut(&str) + Send)) -> Result<PluginResult, NeboError> {
        let resp = self
            .http
            .get(format!("{}/jobs/{job}", self.base))
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(http_error)?;
        let mut resp = ok(resp).await?;
        let mut pending = Vec::new();
        let (mut event, mut data) = (String::new(), Vec::<String>::new());
        while let Some(chunk) = resp.chunk().await.map_err(http_error)? {
            pending.extend_from_slice(&chunk);
            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                let raw: Vec<u8> = pending.drain(..=end).collect();
                let line = String::from_utf8_lossy(&raw);
                let line = line.trim_end_matches(['\n', '\r']);
                if line.is_empty() {
                    match event.as_str() {
                        "result" => return serde_json::from_str(&data.join("\n")).map_err(|e| NeboError::Other(e.to_string())),
                        "progress" => progress(&data.join("\n")),
                        _ => {}
                    }
                    event.clear();
                    data.clear();
                } else if let Some(name) = line.strip_prefix("event:") {
                    event = name.trim().to_string();
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
                }
            }
        }
        Err(NeboError::Other("the plugin job ended without a result".into()))
    }
}

/// The response, or Nebo's refusal with its reason.
async fn ok(resp: reqwest::Response) -> Result<reqwest::Response, NeboError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let reason = resp.text().await.unwrap_or_default();
    Err(NeboError::Refused { status: status.as_u16(), reason })
}

fn http_error(e: reqwest::Error) -> NeboError {
    NeboError::Http(e.to_string())
}
