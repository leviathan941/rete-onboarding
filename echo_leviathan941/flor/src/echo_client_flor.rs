use error_stack::{Report, ResultExt};
use flor::component::Component;
use flor::component_factory::ComponentMaker;
use flor::config::Config;
use flor::context::Context;
use flor::provided_capabilities::ProvidedCapabilities;
use flor::util::{log_debug, log_error};
use futures::stream::StreamExt;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::{interval, timeout};
use tokio_stream::wrappers::IntervalStream;

#[derive(Debug, thiserror::Error, flor::util::FromString)]
#[error("{0}")]
struct Error(String);

pub struct Client {
    addr: SocketAddr,
}

impl Client {
    async fn start(&self) -> Result<(), Report<Error>> {
        let mut handles = JoinSet::new();
        let task_interval = interval(Duration::from_secs(1));
        let mut stream = IntervalStream::new(task_interval).enumerate().take(10);
        while let Some((i, _)) = stream.next().await {
            let addr = self.addr;
            handles.spawn(async move { Client::run_connection(i, addr).await });
        }

        while let Some(res) = handles.join_next().await {
            if let Err(e) = res {
                log_error!("Client task failed: {e}");
            }
        }

        log_debug!("Client: All connections completed");
        Ok(())
    }

    async fn run_connection(index: usize, addr: SocketAddr) -> Result<(), Report<Error>> {
        let make_error = || Error("Failed to connect to server".into());
        let socket = TcpStream::connect(addr)
            .await
            .change_context_lazy(make_error)?;
        let (mut rd, mut wr) = io::split(socket);

        let write_task = tokio::spawn(async move {
            log_debug!("Client {index} sending message");
            wr.write_all(format!("hello world {index}").as_bytes())
                .await
        });

        let mut buf = vec![0; 128];

        loop {
            let result = timeout(Duration::from_secs(5), rd.read(&mut buf)).await;

            match result {
                Ok(Ok(0)) => {
                    log_debug!("Client {index} connection closed by server");
                    break;
                }
                Ok(Ok(n)) => {
                    let str = String::from_utf8_lossy(&buf[..n]);
                    log_debug!("Client {index} GOT {str}");
                }
                Ok(Err(e)) => {
                    log_error!("Client {index} read error: {e}");
                    break;
                }
                Err(_) => {
                    log_error!("Client {index} read timed out");
                    break;
                }
            };
        }

        write_task
            .await
            .map_err(|e| Error(format!("Failed to write to server: {e}")))?
            .change_context_lazy(make_error)?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl Component for Client {
    async fn on_start(
        &mut self,
        _ctx: &mut Context,
    ) -> error_stack::Result<(), flor::component::Error> {
        self.start()
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
        let addr = cfg
            .get::<String>("addr")
            .change_context_lazy(make_error)?
            .parse::<SocketAddr>()
            .change_context_lazy(make_error)?;
        let client = Client { addr };
        Ok((ctx, ProvidedCapabilities::default(), Box::new(client)))
    }
}
