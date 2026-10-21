use std::io::{Read, Write};
use std::net::Ipv4Addr;

use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::service::{Interceptor, InterceptorLayer};
use tonic::transport::Server;
use tonic::{Request, Status};

use crate::env::AppEnv;
use crate::error::NeboError;
use crate::pb;

/// The line the app prints on stdout once it is listening, followed by its
/// address. Nebo reads it to know where to connect.
pub const LISTENING: &str = "nebo-app-listening ";

/// Main entry point for a Nebo app in Rust.
pub struct NeboApp {
    env: AppEnv,
    tool: Option<Box<dyn crate::tool::ToolHandler>>,
    channel: Option<Box<dyn crate::channel::ChannelHandler>>,
    gateway: Option<Box<dyn crate::gateway::GatewayHandler>>,
    ui: Option<Box<dyn crate::ui::UiHandler>>,
    comm: Option<Box<dyn crate::comm::CommHandler>>,
    schedule: Option<Box<dyn crate::schedule::ScheduleHandler>>,
    on_configure: Option<Box<dyn Fn(std::collections::HashMap<String, String>) + Send + Sync>>,
}

impl NeboApp {
    /// Create a new NeboApp from the environment Nebo launches it with.
    /// Fails without `NEBO_APP_TOKEN`: an app run outside Nebo has no one to
    /// serve.
    pub fn new() -> Result<Self, NeboError> {
        let env = AppEnv::load();
        if env.token.is_empty() {
            return Err(NeboError::NoToken);
        }
        Ok(Self {
            env,
            tool: None,
            channel: None,
            gateway: None,
            ui: None,
            comm: None,
            schedule: None,
            on_configure: None,
        })
    }

    /// Returns the app's environment.
    pub fn env(&self) -> &AppEnv {
        &self.env
    }

    /// Set a callback for settings updates from Nebo.
    pub fn on_configure<F: Fn(std::collections::HashMap<String, String>) + Send + Sync + 'static>(
        mut self,
        f: F,
    ) -> Self {
        self.on_configure = Some(Box::new(f));
        self
    }

    pub fn register_tool(mut self, h: impl crate::tool::ToolHandler) -> Self {
        self.tool = Some(Box::new(h));
        self
    }

    pub fn register_channel(mut self, h: impl crate::channel::ChannelHandler) -> Self {
        self.channel = Some(Box::new(h));
        self
    }

    pub fn register_gateway(mut self, h: impl crate::gateway::GatewayHandler) -> Self {
        self.gateway = Some(Box::new(h));
        self
    }

    pub fn register_ui(mut self, h: impl crate::ui::UiHandler) -> Self {
        self.ui = Some(Box::new(h));
        self
    }

    pub fn register_comm(mut self, h: impl crate::comm::CommHandler) -> Self {
        self.comm = Some(Box::new(h));
        self
    }

    pub fn register_schedule(mut self, h: impl crate::schedule::ScheduleHandler) -> Self {
        self.schedule = Some(Box::new(h));
        self
    }

    /// Serve Nebo until it stops the app. The same on every OS:
    ///
    /// - listens on `127.0.0.1` (never another interface), on a port the OS
    ///   picks, and prints `nebo-app-listening <address>` on stdout;
    /// - refuses every call that does not carry this launch's token as
    ///   `authorization: Bearer <NEBO_APP_TOKEN>`;
    /// - returns when stdin closes (Nebo is gone, however it went), on
    ///   SIGTERM, or on Ctrl-C.
    pub async fn run(self) -> Result<(), NeboError> {
        if self.tool.is_none()
            && self.channel.is_none()
            && self.gateway.is_none()
            && self.ui.is_none()
            && self.comm.is_none()
            && self.schedule.is_none()
        {
            return Err(NeboError::NoHandlers);
        }

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;

        let mut builder = Server::builder().layer(InterceptorLayer::new(Authorize { token: self.env.token.clone() }));

        // Register each capability's gRPC service
        let mut router = builder.add_optional_service(self.tool.map(|h| {
            pb::tool_service_server::ToolServiceServer::new(crate::tool::ToolBridge {
                handler: h,
                on_configure: None,
                env: self.env.clone(),
            })
        }));

        router = router.add_optional_service(self.channel.map(|h| {
            pb::channel_service_server::ChannelServiceServer::new(crate::channel::ChannelBridge {
                handler: h,
                on_configure: None,
                env: self.env.clone(),
            })
        }));

        router = router.add_optional_service(self.gateway.map(|h| {
            pb::gateway_service_server::GatewayServiceServer::new(crate::gateway::GatewayBridge {
                handler: h,
                on_configure: None,
                env: self.env.clone(),
            })
        }));

        router = router.add_optional_service(self.ui.map(|h| {
            pb::ui_service_server::UiServiceServer::new(crate::ui::UiBridge {
                handler: h,
                on_configure: None,
                env: self.env.clone(),
            })
        }));

        router = router.add_optional_service(self.comm.map(|h| {
            pb::comm_service_server::CommServiceServer::new(crate::comm::CommBridge {
                handler: h,
                on_configure: None,
                env: self.env.clone(),
            })
        }));

        router = router.add_optional_service(self.schedule.map(|h| {
            pb::schedule_service_server::ScheduleServiceServer::new(
                crate::schedule::ScheduleBridge {
                    handler: h,
                    on_configure: None,
                    env: self.env.clone(),
                },
            )
        }));

        {
            let mut out = std::io::stdout().lock();
            writeln!(out, "{LISTENING}{addr}")?;
            out.flush()?;
        }
        eprintln!("[{}] listening on {addr}", self.env.name);

        router
            .serve_with_incoming_shutdown(TcpListenerStream::new(listener), stopped())
            .await?;

        Ok(())
    }
}

/// Lets a call through only if it carries the launch's token as
/// `authorization: Bearer <token>`.
#[derive(Clone)]
struct Authorize {
    token: String,
}

impl Interceptor for Authorize {
    fn call(&mut self, req: Request<()>) -> Result<Request<()>, Status> {
        let presented = req
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, t)| t.trim())
            .unwrap_or("");
        if !self.token.is_empty() && same(presented.as_bytes(), self.token.as_bytes()) {
            Ok(req)
        } else {
            Err(Status::unauthenticated("missing or wrong app token"))
        }
    }
}

/// Byte equality in time that does not depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Resolves when the app should stop: its stdin closed (Nebo holds the other
/// end for the app's whole life, so this is how a Nebo that was killed is
/// noticed), SIGTERM, or Ctrl-C.
async fn stopped() {
    let (closed, stdin_closed) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        let mut stdin = std::io::stdin();
        while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
        let _ = closed.send(());
    });
    #[cfg(unix)]
    let terminated = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                term.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminated = std::future::pending::<()>();
    tokio::select! {
        _ = stdin_closed => {}
        _ = terminated => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(bearer: Option<&str>) -> Request<()> {
        let mut req = Request::new(());
        if let Some(b) = bearer {
            req.metadata_mut().insert("authorization", b.parse().unwrap());
        }
        req
    }

    fn authorize(token: &str, req: Request<()>) -> Result<(), tonic::Code> {
        Authorize { token: token.into() }.call(req).map(|_| ()).map_err(|s| s.code())
    }

    #[test]
    fn only_the_launch_token_is_let_through() {
        assert!(authorize("abc", call(Some("Bearer abc"))).is_ok());
        assert!(authorize("abc", call(Some("bearer abc"))).is_ok());
        for refused in [None, Some("Bearer abd"), Some("Bearer ab"), Some("abc"), Some("Basic abc")] {
            assert_eq!(authorize("abc", call(refused)), Err(tonic::Code::Unauthenticated), "{refused:?}");
        }
        assert!(authorize("", call(Some("Bearer "))).is_err(), "no token lets nothing through");
    }
}
