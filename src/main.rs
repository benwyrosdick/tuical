mod app;
mod config;
mod model;
mod provider;
mod settings;
mod ui;

use anyhow::Result;

use crate::{app::App, config::TuicalConfig, settings::AppSettings};

#[tokio::main]
async fn main() -> Result<()> {
    let config = TuicalConfig::load("config.toml")?;
    let settings = AppSettings::load("settings.toml")?;
    App::new(config, settings).run().await
}
