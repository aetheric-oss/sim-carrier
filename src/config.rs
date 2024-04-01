//! # Config
//!
//! Define and implement config options for module

use anyhow::Result;
use config::{ConfigError, Environment};
use dotenv::dotenv;
use serde::Deserialize;

/// struct holding configuration options
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub host: String,
    pub cargo_host_port_rest: u16,
    pub atc_host_port_rest: u16,
    pub telemetry_host_port_rest: u16
}

impl Config {
    /// Create a new `Config` object using environment variables
    pub fn try_from_env() -> Result<Self, ConfigError> {
        // read .env file if present
        dotenv().ok();
        config::Config::builder()
            .add_source(Environment::default().separator("__"))
            .build()?
            .try_deserialize()
    }
}
