//! `sdeck ctl`: a read-only control API of the running sdeck, over loopback TCP.

pub mod endpoint;
pub mod protocol;
pub mod server;

pub use endpoint::{endpoint_path, CtlClient, Endpoint};
pub use protocol::{CtlRequest, CtlResponse};
