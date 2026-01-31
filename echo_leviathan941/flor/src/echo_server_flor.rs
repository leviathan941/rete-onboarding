use error_stack::{Report, ResultExt};
use flor::component::Component;
use flor::component_factory::ComponentMaker;
use flor::config::Config;
use flor::context::Context;
use flor::provided_capabilities::ProvidedCapabilities;
use flor::util::{log_debug, log_error};
use std::net::IpAddr;
use std::sync::Arc;
use tokio::io;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

#[derive(Debug, thiserror::Error, flor::util::FromString)]
#[error("{0}")]
struct Error(String);

pub struct Server {
    addr: IpAddr,
    port: u16,
    max_connections: usize,
}

impl Server {
    async fn start_listening(&self) -> Result<(), Report<Error>> {
        let make_error = || Error("Failed to start listening".into());

        let listener = TcpListener::bind((self.addr, self.port))
            .await
            .change_context_lazy(make_error)?;
        let semaphore = Arc::new(Semaphore::new(self.max_connections));

        loop {
            let (stream, _) = listener.accept().await.change_context_lazy(make_error)?;
            if let Ok(permit) = semaphore.clone().try_acquire_owned() {
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = Server::echo(stream).await {
                        log_error!("Server failed to handle connection: {e}");
                    }
                });
            } else {
                log_debug!(
                    "Server: Max connections {} reached; rejecting new connection.",
                    self.max_connections
                );
                continue;
            }
        }
    }

    async fn echo(mut stream: tokio::net::TcpStream) -> Result<(), Error> {
        let (mut rd, mut wr) = stream.split();
        match io::copy(&mut rd, &mut wr).await {
            Ok(_) => Ok(()),
            Err(e) => Err(Error::from(format!("Failed to echo: {e}"))),
        }
    }
}

#[async_trait::async_trait]
impl Component for Server {
    async fn on_start(
        &mut self,
        _ctx: &mut Context,
    ) -> error_stack::Result<(), flor::component::Error> {
        self.start_listening()
            .await
            .change_context(flor::component::Error::from(
                "Server failed to start listening",
            ))
    }
}

#[async_trait::async_trait]
impl ComponentMaker for Server {
    async fn make(
        ctx: Context,
        cfg: Config,
    ) -> error_stack::Result<
        (Context, ProvidedCapabilities, Box<dyn Component>),
        flor::component_factory::Error,
    > {
        let make_error =
            || flor::component_factory::Error::from("Failed to create Server component");
        let addr = cfg
            .get::<String>("host")
            .change_context_lazy(make_error)?
            .parse::<IpAddr>()
            .change_context_lazy(make_error)?;
        let server = Server {
            addr,
            port: cfg.get("port").change_context_lazy(make_error)?,
            max_connections: cfg.get("max_connections").change_context_lazy(make_error)?,
        };
        Ok((ctx, ProvidedCapabilities::default(), Box::new(server)))
    }
}
