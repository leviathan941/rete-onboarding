use std::sync::Arc;

use async_trait::async_trait;
use error_stack::{Report, ResultExt, report};
use flor::component::Component;
use flor::component_factory::ComponentMaker;
use flor::config::Config;
use flor::context::Context;
use flor::provided_capabilities::ProvidedCapabilities;
use flor::util::{log_debug, log_error};
use floretum::portal::client_cert_verifier::{AllowAnyTrusted, ClientCertVerifier};
use floretum::portal::identity::Identity;
use floretum::portal::publisher::{Listener, Options, Publisher, ServiceDesc};
use floretum::portal::service_name::ServiceName;
use floretum::tunnel::stream::StreamTunnel;
use floretum::tunnel::supported_protocols::{SupportedProtocols, Transport};
use floretum::tunnel::{IntoSplit, Tunnel, TunnelExt};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    service_name: ServiceName,
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
    fn new(service_name: ServiceName, max_connections: usize) -> Self {
        let (command_sender, command_receiver) = mpsc::channel(10);
        let (event_sender, _) = broadcast::channel(10);

        Server {
            service_name,
            max_connections,
            command_sender,
            command_receiver,
            event_sender,
        }
    }

    fn obtain_observable(&self) -> Box<dyn ConnectionObservable> {
        ServerObservable::new_boxed(self.command_sender.clone())
    }

    async fn start(&mut self, mut listener: Listener) -> Result<(), Report<Error>> {
        let mut handlers = FuturesUnordered::new();

        loop {
            tokio::select! {
                Ok(Tunnel::Stream(stream_tunnel)) = listener.accept() => {
                    let client_name = stream_tunnel.peer_name().cloned().unwrap_or_else(|| ServiceName::new("unknown"));
                    self.handle_connection(stream_tunnel, client_name, &mut handlers).await?;
                }

                Some(_) = handlers.next() => {
                    log_debug!("Server: A connection handler finished");
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
        stream: StreamTunnel,
        client_name: ServiceName,
        handlers: &mut FuturesUnordered<JoinHandle<()>>,
    ) -> Result<(), Report<Error>> {
        if handlers.len() < self.max_connections {
            let handler = tokio::spawn(async move {
                if let Err(e) = Server::echo(stream).await {
                    log_error!("Server: Failed to handle connection: {e}");
                }
            });
            handlers.push(handler);
            self.send_event(ConnectionEvent::Accept(Connection {
                id: client_name.to_string(),
            }));
        } else {
            log_debug!(
                "Server: Max connections {} reached; rejecting new connection.",
                self.max_connections
            );
            self.send_event(ConnectionEvent::Reject(Connection {
                id: client_name.to_string(),
            }));
        }
        Ok(())
    }

    fn send_event(&self, event: ConnectionEvent) {
        if let Err(e) = self.event_sender.send(event) {
            log_debug!("Server: Failed to send connection event: {e}");
        }
    }

    fn handle_command(&self, cmd: ObservableCommand) {
        match cmd {
            ObservableCommand::Subscribe { responder } => {
                log_debug!("Server: Received command: Subscribe");
                let subscribe_result = Ok(self.event_sender.subscribe());
                if responder.send(subscribe_result).is_err() {
                    log_debug!("Server: Failed to send subscription response");
                }
            }
        }
    }

    async fn echo(stream: StreamTunnel) -> Result<(), Report<Error>> {
        // Why do we need more complex solution here compared to the TcpStream case?
        //
        // This is how TcpStream solution works:
        // The server accepted the connection, received a stream, split it, performed reads and
        // writes, and then dropped it.
        // The client connected, received a stream, split it as well, moved the writer into
        // a separate task, dropped the writer, and continued reading in the main task.
        // Therefore, when a TcpStream is dropped, it initiates a graceful shutdown of
        // the underlying system socket.
        // Any pending data is written out, and once everything that hasn’t yet been sent
        // is flushed, a FIN is sent.
        // As a result, the server reads the stream cleanly, and the client also finishes
        // reading everything it needs without any issues.
        //
        // Floretum's StreamTunnel uses QUIC under the hood (quinn implementation),
        // which works differently.
        // When all clones of the connection and its streams are dropped, close() is called,
        // which immediately tears everything down.
        // Therefore, if we won't change anything in the solution, the server will drop
        // the entire connection as soon as it finishes writing the response.
        // The client, on the other hand, receives an error when trying to read the response so
        // receives nothing.
        // To solve this, we need to keep the stream alive on the server side until the client
        // finishes reading the response.
        let (mut rd, mut wr) = stream.into_split();
        loop {
            let mut buf = vec![0; 128];
            match rd.read(&mut buf).await {
                Ok(0) => {
                    log_debug!("Server: Stream closed by client");
                    break;
                }
                Ok(len) => {
                    log_debug!(
                        "Server: Received message: {}",
                        String::from_utf8_lossy(&buf[..len])
                    );
                    wr.write_all(&buf[..len])
                        .await
                        .change_context("Failed to write a message".into())?;
                }
                Err(e) => match e.kind() {
                    std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::UnexpectedEof => {
                        log_debug!("Server: Connection closed by client");
                        break;
                    }
                    _ => {
                        return Err(report!(Error::from(format!(
                            "Failed to read from connection: {e}"
                        ))));
                    }
                },
            }
        }
        wr.shutdown()
            .await
            .change_context("Failed to shutdown write stream".into())?;
        Ok(())
    }
}

#[async_trait]
impl Component for Server {
    async fn on_start(
        &mut self,
        ctx: &mut Context,
    ) -> error_stack::Result<(), flor::component::Error> {
        let publisher = ctx
            .get_capability::<Box<dyn Publisher>>("publisher")
            .change_context("Failed to get publisher capability".into())?;

        let identity = Identity::new(
            include_bytes!("../../certs/echo_server_cert.pem"),
            include_bytes!("../../certs/echo_server_key.pem"),
        )
        .change_context("Failed to create server identity".into())?;
        let client_verifier: Option<Arc<dyn ClientCertVerifier>> = Some(Arc::new(AllowAnyTrusted));

        let listener = publisher
            .publish(
                ServiceDesc {
                    name: self.service_name.clone(),
                    identity,
                },
                Options {
                    supported_protocols: SupportedProtocols::raw(&[Transport::Stream]),
                    client_verifier,
                },
            )
            .await
            .change_context(
                format!("Failed to publish service {} in node", self.service_name).into(),
            )?;

        self.start(listener)
            .await
            .change_context("Server failed to start listening".into())
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

        let service_name = cfg
            .get::<ServiceName>("name")
            .change_context_lazy(make_error)?;

        let server = Server::new(
            service_name,
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
