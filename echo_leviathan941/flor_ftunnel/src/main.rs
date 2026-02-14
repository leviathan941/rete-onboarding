use flor::CoreManifest;
use flor::clap::{self, Parser};
use flor::error_stack::{Report, ResultExt};
use flor::macros::app_setup;
use flor::package::{DeployDeclSource, package};
use flor::util::logger::init_logger;
use flor::util::{AsErr, FromString, decl_options};
use floretum::FloretumManifest;

mod capabilities;
mod services;

#[package("component_manifest.yaml")]
struct EchoFlorManifest;

#[app_setup(CoreManifest, EchoFlorManifest, FloretumManifest)]
struct AppSetup;

decl_options!(Deps: flor);

#[derive(Debug, thiserror::Error, FromString)]
#[error("{0}")]
struct Error(String);

#[tokio::main]
async fn main() -> Result<(), Report<Error>> {
    init_logger(log::LevelFilter::Debug).change_context("Failed to initialize logger".as_err())?;
    let options = Options::parse();
    flor::run::run(AppSetup, DeployDeclSource::File(options.config))
        .await
        .change_context("Framework error".as_err())
}
