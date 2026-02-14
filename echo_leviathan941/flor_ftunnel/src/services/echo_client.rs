use error_stack::{Report, ResultExt, report};
use flor::component::Component;
use flor::component_factory::ComponentMaker;
use flor::config::Config;
use flor::context::Context;
use flor::provided_capabilities::ProvidedCapabilities;
use flor::util::{log_debug, log_error};
use floretum::portal::connector::{self, Connector, Options};
use floretum::portal::identity::Identity;
use floretum::portal::service_name::ServiceName;
use floretum::tunnel::stream::StreamTunnel;
use floretum::tunnel::supported_protocols::{SupportedProtocols, Transport};
use floretum::tunnel::{IntoSplit, Tunnel};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinSet;
use tokio::time::timeout;

#[derive(Debug, thiserror::Error, flor::util::FromString)]
#[error("{0}")]
struct Error(String);

pub struct Client {
    name: ServiceName,
    server_name: ServiceName,
}

impl Client {
    async fn start(&self, connector: Box<dyn Connector>) -> Result<(), Report<Error>> {
        let identity = Some(
            Identity::new(
                include_bytes!("../../certs/echo_client_cert.pem"),
                include_bytes!("../../certs/echo_client_key.pem"),
            )
            .change_context("Failed to create client identity".into())?,
        );

        let supported_protocols = SupportedProtocols::raw(&[Transport::Stream]);

        let mut handles = JoinSet::new();

        for i in 0..10 {
            let connector = connector.clone();
            let self_name = self.name.clone();
            let server_name = self.server_name.clone();
            let supported_protocols = supported_protocols.clone();
            let identity = identity.clone();
            handles.spawn(async move {
                log_debug!("Client: Starting connection {i}");
                Client::connect(
                    i,
                    self_name,
                    server_name,
                    supported_protocols,
                    identity,
                    connector,
                )
                .await
            });
        }

        while let Some(res) = handles.join_next().await {
            if let Err(e) = res {
                log_error!("Client task failed: {e}");
            }
        }

        log_debug!("Client: All connections completed");
        Ok(())
    }

    async fn connect(
        i: usize,
        self_name: ServiceName,
        server_name: ServiceName,
        supported_protocols: SupportedProtocols,
        identity: Option<Identity>,
        connector: Box<dyn Connector>,
    ) -> Result<(), Report<Error>> {
        let tunnel = connector
            .connect(
                server_name.clone(),
                Options {
                    supported_protocols,
                    identity,
                },
            )
            .await
            .change_context(
                format!(
                    "Failed to connect to server {} from client {}",
                    server_name, self_name
                )
                .into(),
            )?;
        let Tunnel::Stream(stream_tunnel) = tunnel else {
            return Err(report!(Error::from("Unsupported tunnel type")));
        };

        Client::handle_connection(i, stream_tunnel).await
    }

    async fn handle_connection(
        index: usize,
        stream_tunnel: StreamTunnel,
    ) -> Result<(), Report<Error>> {
        let (mut rd, mut wr) = stream_tunnel.into_split();

        // Keep the writer alive until the end of the function.
        // Otherwise, the tunnel might be closed before we finish reading the response.
        log_debug!("Client {index}: writing message");
        wr.write_all(format!("hello world {index}").as_bytes())
            .await
            .change_context(Error::from("Failed to write message"))?;

        let mut buf = vec![0; 128];
        log_debug!("Client {index}: reading message");
        let result = timeout(Duration::from_secs(5), rd.read(&mut buf)).await;
        match result {
            Ok(Ok(0)) => {
                log_debug!("Client {index}: connection closed by server");
            }
            Ok(Ok(n)) => {
                let str = String::from_utf8_lossy(&buf[..n]);
                log_debug!("Client {index}: GOT {str}");
            }
            Ok(Err(e)) => {
                log_error!("Client {index}: read error: {e}");
            }
            Err(_) => {
                log_error!("Client {index}: read timed out");
            }
        };
        wr.shutdown()
            .await
            .change_context(Error::from("Failed to shutdown write stream"))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Component for Client {
    async fn on_start(
        &mut self,
        ctx: &mut Context,
    ) -> error_stack::Result<(), flor::component::Error> {
        let connector = ctx
            .get_capability::<Box<dyn connector::Connector>>("connector")
            .change_context("Failed to get connector capability".into())?;

        self.start(connector.clone())
            .await
            .change_context(flor::component::Error::from("Failed to start Client"))
    }
}

#[async_trait::async_trait]
impl ComponentMaker for Client {
    async fn make(
        ctx: Context,
        cfg: Config,
    ) -> error_stack::Result<
        (Context, ProvidedCapabilities, Box<dyn Component>),
        flor::component_factory::Error,
    > {
        let make_error =
            || flor::component_factory::Error::from("Failed to create Client component");
        let name = cfg
            .get::<ServiceName>("name")
            .change_context_lazy(make_error)?;
        let server_name = cfg
            .get::<ServiceName>("server_name")
            .change_context_lazy(make_error)?;
        let client = Client { name, server_name };
        Ok((ctx, ProvidedCapabilities::default(), Box::new(client)))
    }
}
