use async_trait::async_trait;
use std::collections::HashMap;
use tonic::{Request, Response, Status};

use crate::env::AppEnv;
use crate::error::NeboError;
use crate::pb;

/// An HTTP request proxied from the browser to the app.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

/// An HTTP response from the app back to the browser.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status_code: i32,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

/// Trait for UI capability handlers.
#[async_trait]
pub trait UiHandler: Send + Sync + 'static {
    async fn handle_request(&self, req: HttpRequest) -> Result<HttpResponse, NeboError>;
}

/// What the SDK's health answer proves itself with: Nebo's health probe sends
/// a nonce in `x-nebo-challenge`, and the answer carries `x-nebo-proof` =
/// hex HMAC-SHA256(key = NEBO_APP_TOKEN, nonce || NEBO_SIDECAR_TOKEN || port
/// as 2 bytes big-endian). Nebo never sends NEBO_APP_TOKEN, so nothing that
/// takes over the app's port can answer. App code never sees any of it.
pub(crate) struct HealthProof {
    pub app_token: String,
    pub sidecar_token: String,
    pub port: u16,
}

impl HealthProof {
    /// The proof for a hex `challenge`, or `None` for one that is not 32 bytes of hex.
    pub fn answer(&self, challenge: &str) -> Option<String> {
        let nonce = from_hex(challenge).filter(|n| n.len() == 32)?;
        Some(to_hex(&proof(&self.app_token, &nonce, &self.sidecar_token, self.port)))
    }
}

fn proof(app_token: &str, nonce: &[u8], sidecar_token: &str, port: u16) -> [u8; 32] {
    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(app_token.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(nonce);
    mac.update(sidecar_token.as_bytes());
    mac.update(&port.to_be_bytes());
    mac.finalize().into_bytes().into()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// The UI service, always served: Nebo's health probe calls it on every app.
/// `handler` is the app's own (`register_ui`), if it has one.
pub(crate) struct UiBridge {
    pub handler: Option<Box<dyn UiHandler>>,
    pub proof: HealthProof,
    pub on_configure: Option<Box<dyn Fn(HashMap<String, String>) + Send + Sync>>,
    pub env: AppEnv,
}

#[tonic::async_trait]
impl pb::ui_service_server::UiService for UiBridge {
    async fn health_check(
        &self,
        req: Request<pb::HealthCheckRequest>,
    ) -> Result<Response<pb::HealthCheckResponse>, Status> {
        let proof = req
            .metadata()
            .get("x-nebo-challenge")
            .and_then(|v| v.to_str().ok())
            .and_then(|c| self.proof.answer(c));
        let mut answer = Response::new(pb::HealthCheckResponse {
            healthy: true,
            name: self.env.name.clone(),
            version: self.env.version.clone(),
        });
        if let Some(p) = proof.and_then(|p| p.parse().ok()) {
            answer.metadata_mut().insert("x-nebo-proof", p);
        }
        Ok(answer)
    }

    async fn handle_request(
        &self,
        req: Request<pb::HttpRequest>,
    ) -> Result<Response<pb::HttpResponse>, Status> {
        let Some(handler) = &self.handler else {
            return Err(Status::unimplemented("this app has no UI handler"));
        };
        let inner = req.into_inner();
        let http_req = HttpRequest {
            method: inner.method,
            path: inner.path,
            query: inner.query,
            headers: inner.headers,
            body: inner.body,
        };

        let result = handler
            .handle_request(http_req)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(pb::HttpResponse {
            status_code: result.status_code,
            headers: result.headers,
            body: result.body,
        }))
    }

    async fn configure(
        &self,
        req: Request<pb::SettingsMap>,
    ) -> Result<Response<pb::Empty>, Status> {
        if let Some(ref cb) = self.on_configure {
            cb(req.into_inner().values);
        }
        Ok(Response::new(pb::Empty {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_TOKEN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    const SIDECAR_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// The vector Nebo and every SDK check their proof against.
    #[test]
    fn the_proof_matches_the_shared_vector() {
        let p = HealthProof { app_token: APP_TOKEN.into(), sidecar_token: SIDECAR_TOKEN.into(), port: 54321 };
        assert_eq!(
            p.answer("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f").as_deref(),
            Some("f67ca363caac14eb7f345df8c7548d11a40334c55d255e30ec02339265ed8db8")
        );
    }

    #[test]
    fn a_challenge_that_is_not_32_bytes_of_hex_gets_no_proof() {
        let p = HealthProof { app_token: APP_TOKEN.into(), sidecar_token: SIDECAR_TOKEN.into(), port: 1 };
        for bad in ["", "00", "zz0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", "é"] {
            assert_eq!(p.answer(bad), None, "{bad:?}");
        }
    }
}
