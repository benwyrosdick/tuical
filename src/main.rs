mod app;
mod config;
mod model;
mod provider;
mod ui;

use anyhow::Result;

use crate::{app::App, config::TuicalConfig};

#[tokio::main]
async fn main() -> Result<()> {
    let config = TuicalConfig::load("config.toml")?;
    App::new(config).run().await
}
