use anyhow::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize the logging system
pub fn init_logging() -> Result<()> {
    // Configure tracing subscriber with environment filter
    // RUST_LOG env var controls verbosity (e.g., RUST_LOG=debug)
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer())
        .init();

    Ok(())
}
