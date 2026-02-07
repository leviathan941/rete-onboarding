use async_trait::async_trait;
use error_stack::{Report, ResultExt};
use flor::{
    component::{self, Component},
    component_factory::{self, ComponentMaker},
    config::Config,
    context::Context,
    provided_capabilities::ProvidedCapabilities,
    util::log_info,
};

use crate::capabilities::observable::connection::ConnectionObservable;

const CAPABILITY_NAME: &str = "connection_observable";

#[derive(Debug, thiserror::Error, flor::util::FromString)]
#[error("{0}")]
struct Error(String);

pub struct ConnectionMonitor;

impl ConnectionMonitor {
    async fn start(&self, conn_observable: &dyn ConnectionObservable) -> Result<(), Report<Error>> {
        let mut subscription = conn_observable
            .subscribe()
            .await
            .change_context(Error::from("Failed to subscribe to connection events"))?;

        while let Ok(event) = subscription.recv().await {
            log_info!("Connection event: {:?}", event);
        }

        Ok(())
    }
}

#[async_trait]
impl Component for ConnectionMonitor {
    async fn on_start(&mut self, ctx: &mut Context) -> error_stack::Result<(), component::Error> {
        let make_error = || component::Error::from("Failed to start ConnectionMonitor");
        let connection_observable = ctx
            .get_capability::<Box<dyn ConnectionObservable>>(CAPABILITY_NAME)
            .change_context_lazy(make_error)?;

        self.start(connection_observable.as_ref())
            .await
            .change_context_lazy(make_error)
    }
}

#[async_trait]
impl ComponentMaker for ConnectionMonitor {
    async fn make(
        ctx: Context,
        _cfg: Config,
    ) -> error_stack::Result<
        (Context, ProvidedCapabilities, Box<dyn component::Component>),
        component_factory::Error,
    > {
        Ok((ctx, ProvidedCapabilities::default(), Box::new(Self {})))
    }
}
