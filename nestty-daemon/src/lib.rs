//! Headless host: socket transport, plugin supervisor, action registry.
//! Wire protocol: see `docs/gui-daemon-protocol.md`.

pub mod service_supervisor;
pub mod socket;
pub mod trigger_sink;
