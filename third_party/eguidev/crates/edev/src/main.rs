//! Eguidev MCP launcher binary.

use std::process;

/// Entry point for the eguidev MCP launcher.
#[tokio::main]
async fn main() {
    if let Err(error) = edev::run().await {
        eprintln!("edev: {error}");
        process::exit(1);
    }
}
