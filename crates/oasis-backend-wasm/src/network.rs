//! `NetworkBackend` stub for WASM.
//!
//! TCP sockets are not available in browsers. This stub returns appropriate
//! errors for all operations.

use oasis_types::backend::{NetworkBackend, NetworkStream};
use oasis_types::error::{OasisError, Result};

pub struct WasmNetworkBackend;

impl WasmNetworkBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WasmNetworkBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkBackend for WasmNetworkBackend {
    fn listen(&mut self, _port: u16) -> Result<()> {
        Err(OasisError::Backend(
            "TCP listen not available in browser".into(),
        ))
    }

    fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>> {
        Err(OasisError::Backend(
            "TCP accept not available in browser".into(),
        ))
    }

    fn connect(&mut self, _address: &str, _port: u16) -> Result<Box<dyn NetworkStream>> {
        Err(OasisError::Backend(
            "TCP connect not available in browser".into(),
        ))
    }
}
