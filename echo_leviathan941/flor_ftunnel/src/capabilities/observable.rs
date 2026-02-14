use async_trait::async_trait;
use error_stack::Report;
use flor::macros::capability;
use tokio::sync::broadcast;

#[derive(Debug, thiserror::Error, flor::util::FromString)]
#[error("{0}")]
pub struct Error(String);

#[async_trait]
pub trait Observable {
    type Event;

    async fn subscribe(&self) -> Result<broadcast::Receiver<Self::Event>, Report<Error>>;
}

pub(crate) mod connection {
    use super::*;

    #[async_trait]
    #[capability(Clone)]
    pub trait ConnectionObservable: Observable<Event = ConnectionEvent> {}

    #[derive(Debug, Clone)]
    pub struct Connection {
        pub id: String,
    }

    #[derive(Debug, Clone)]
    pub enum ConnectionEvent {
        Accept(Connection),
        Reject(Connection),
    }
}
