//! Compatibility re-exports for the shared Action Broker wire protocol.
//!
//! The protocol lives in `wayexpand-broker-protocol` so clients do not depend
//! on this server/executor crate.

pub use wayexpand_broker_protocol::*;
