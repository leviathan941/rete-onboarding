use async_trait::async_trait;
use error_stack::{Report, ResultExt};
use flor::component::Component;
use flor::component_factory::ComponentMaker;
use flor::config::Config;
use flor::context::Context;
use flor::provided_capabilities::ProvidedCapabilities;
use flor::util::{log_debug, log_error};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::net::{IpAddr, SocketAddr};
use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::capabilities::observable::connection::{
    Connection, ConnectionEvent, ConnectionObservable,
};
use crate::capabilities::observable::{self, Observable};

#[derive(Debug, thiserror::Error, flor::util::FromString)]
#[error("{0}")]
struct Error(String);

pub struct Server {
    addr: IpAddr,
    port: u16,
    max_connections: usize,
    command_sender: mpsc::Sender<ObservableCommand>,
    command_receiver: mpsc::Receiver<ObservableCommand>,
    event_sender: broadcast::Sender<ConnectionEvent>,
}

type SubscribeResult = Result<broadcast::Receiver<ConnectionEvent>, Report<Error>>;

#[derive(Debug)]
enum ObservableCommand {
    Subscribe {
        responder: oneshot::Sender<SubscribeResult>,
    },
}

impl Server {
    fn new(addr: IpAddr, port: u16, max_connections: usize) -> Self {
        let (command_sender, command_receiver) = mpsc::channel(10);
        let (event_sender, _) = broadcast::channel(10);

        Server {
            addr,
            port,
            max_connections,
            command_sender,
            command_receiver,
            event_sender,
        }
    }

    fn obtain_observable(&self) -> Box<dyn ConnectionObservable> {
        ServerObservable::new_boxed(self.command_sender.clone())
    }

    async fn start(&mut self) -> Result<(), Report<Error>> {
        let make_error = || Error("Failed to start server".into());
        let conn_listener = TcpListener::bind((self.addr, self.port))
            .await
            .change_context_lazy(make_error)?;
        let mut handlers = FuturesUnordered::new();

        loop {
            tokio::select! {
                Ok((stream, addr)) = conn_listener.accept() => {
                    self.handle_connection(stream, addr, &mut handlers).await?;
                }

                Some(_) = handlers.next() => {
                    log_debug!("A connection handler finished");
                }

                Some(cmd) = self.command_receiver.recv() => {
                    self.handle_command(cmd);
                }

                else => {
                    // No more events to process
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_connection(
        &self,
        conn_stream: TcpStream,
        addr: SocketAddr,
        handlers: &mut FuturesUnordered<JoinHandle<()>>,
    ) -> Result<(), Report<Error>> {
        if handlers.len() < self.max_connections {
            let handler = tokio::spawn(async move {
                if let Err(e) = Server::echo(conn_stream).await {
                    log_error!("Server failed to handle connection: {e}");
                }
            });
            handlers.push(handler);
            self.send_event(ConnectionEvent::Accept(Connection { addr }));
        } else {
            log_debug!(
                "Server: Max connections {} reached; rejecting new connection.",
                self.max_connections
            );
            self.send_event(ConnectionEvent::Reject(Connection { addr }));
        }
        Ok(())
    }

    fn send_event(&self, event: ConnectionEvent) {
        if let Err(e) = self.event_sender.send(event) {
            log_debug!("Failed to send connection event: {e}");
        }
    }

    fn handle_command(&self, cmd: ObservableCommand) {
        match cmd {
            ObservableCommand::Subscribe { responder } => {
                log_debug!("Received command: Subscribe");
                let subscribe_result = Ok(self.event_sender.subscribe());
                if responder.send(subscribe_result).is_err() {
                    log_debug!("Failed to send subscription response");
                }
            }
        }
    }

    async fn echo(mut stream: TcpStream) -> Result<(), Error> {
        let (mut rd, mut wr) = stream.split();
        match io::copy(&mut rd, &mut wr).await {
            Ok(_) => Ok(()),
            Err(e) => Err(Error::from(format!("Failed to echo: {e}"))),
        }
    }
}

#[async_trait]
impl Component for Server {
    async fn on_start(
        &mut self,
        _ctx: &mut Context,
    ) -> error_stack::Result<(), flor::component::Error> {
        self.start()
            .await
            .change_context(flor::component::Error::from(
                "Server failed to start listening",
            ))
    }
}

#[async_trait]
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

        let server = Server::new(
            addr,
            cfg.get("port").change_context_lazy(make_error)?,
            cfg.get("max_connections").change_context_lazy(make_error)?,
        );

        let mut provided_capabilities = ProvidedCapabilities::default();
        provided_capabilities
            .insert::<Box<dyn ConnectionObservable>>(
                "connection_observable",
                server.obtain_observable(),
            )
            .change_context_lazy(make_error)?;

        Ok((ctx, provided_capabilities, Box::new(server)))
    }
}

#[derive(Debug, Clone)]
struct ServerObservable {
    command_sender: mpsc::Sender<ObservableCommand>,
}

impl ServerObservable {
    fn new_boxed(command_sender: mpsc::Sender<ObservableCommand>) -> Box<dyn ConnectionObservable> {
        Box::new(ServerObservable { command_sender })
    }
}

#[async_trait]
impl Observable for ServerObservable {
    type Event = ConnectionEvent;

    async fn subscribe(
        &self,
    ) -> Result<broadcast::Receiver<ConnectionEvent>, Report<observable::Error>> {
        let make_error = || observable::Error::from("Failed to subscribe to connection events");
        let (response_sender, response_receiver) = oneshot::channel();
        self.command_sender
            .send(ObservableCommand::Subscribe {
                responder: response_sender,
            })
            .await
            .change_context_lazy(make_error)?;

        response_receiver
            .await
            .change_context_lazy(make_error)?
            .change_context_lazy(make_error)
    }
}

impl ConnectionObservable for ServerObservable {}
