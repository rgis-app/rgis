//! MCP server implementation for DevMCP.

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{sync::Arc, thread};

use tmcp::Server;
use tokio::runtime::Builder;

use crate::{registry::Inner, runtime::Runtime, tools};

#[cfg(test)]
static START_SERVER_CALLS: AtomicUsize = AtomicUsize::new(0);

#[allow(clippy::needless_pass_by_value)]
pub fn start_server(inner: Arc<Inner>, runtime_state: Arc<Runtime>) {
    if cfg!(test) {
        #[cfg(test)]
        START_SERVER_CALLS.fetch_add(1, Ordering::Relaxed);
        drop(inner);
        drop(runtime_state);
        return;
    }

    thread::spawn(move || {
        let runtime = Builder::new_current_thread().enable_all().build();
        let Ok(runtime) = runtime else {
            eprintln!("eguidev: failed to start tokio runtime");
            return;
        };
        let server = Server::new(move || {
            tools::DevMcpServer::with_runtime(Arc::clone(&inner), Arc::clone(&runtime_state))
        });
        if let Err(error) = runtime.block_on(server.serve_stdio()) {
            eprintln!("eguidev: MCP server failed: {error}");
        }
    });
}

#[cfg(test)]
#[allow(dead_code)]
pub fn reset_start_server_calls() {
    START_SERVER_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
#[allow(dead_code)]
pub fn start_server_calls() -> usize {
    START_SERVER_CALLS.load(Ordering::Relaxed)
}
