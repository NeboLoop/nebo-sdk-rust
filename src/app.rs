use tokio::signal;
use tonic::transport::Server;

use crate::env::AppEnv;
use crate::error::NeboError;
use crate::pb;

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
    /// Create a new NeboApp. Reads NEBO_APP_* from environment.
    pub fn new() -> Result<Self, NeboError> {
        let env = AppEnv::load();
        if env.sock_path.is_empty() {
            return Err(NeboError::NoSockPath);
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

    /// Start the gRPC server and block until SIGTERM/SIGINT.
    ///
    /// On Unix: listens on a Unix domain socket (NEBO_APP_SOCK is a file path).
    /// On Windows: listens on TCP localhost (NEBO_APP_SOCK is `host:port` or just a port).
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

        let sock_path = &self.env.sock_path;

        let mut builder = Server::builder();

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

        // Platform-specific listener
        #[cfg(unix)]
        {
            use std::path::Path;
            use tokio::net::UnixListener;
            use tokio_stream::wrappers::UnixListenerStream;

            let _ = std::fs::remove_file(sock_path);
            let uds = UnixListener::bind(sock_path)?;
            let uds_stream = UnixListenerStream::new(uds);

            eprintln!(
                "[{}] listening on {}",
                self.env.name,
                Path::new(sock_path).display()
            );

            router
                .serve_with_incoming_shutdown(uds_stream, async {
                    let _ = signal::ctrl_c().await;
                })
                .await?;
        }

        #[cfg(windows)]
        {
            let addr = if sock_path.contains(':') {
                sock_path.to_string()
            } else {
                format!("127.0.0.1:{}", sock_path)
            };
            let addr: std::net::SocketAddr = addr
                .parse()
                .map_err(|e| NeboError::Other(format!("invalid address '{}': {}", addr, e)))?;

            eprintln!("[{}] listening on {}", self.env.name, addr);

            router
                .serve_with_shutdown(addr, async {
                    let _ = signal::ctrl_c().await;
                })
                .await?;
        }

        Ok(())
    }
}
