#[macro_use]
extern crate serde_derive;
extern crate serde;

use std::net::SocketAddr;

#[derive(Serialize, Deserialize)]
pub struct Server<'a> {
    pub address: SocketAddr,
    pub info: &'a [u8],
}

/// ALPN protcol identifier that must be used for heartbeat connections.
pub const HEARTBEAT_PROTOCOL: &[u8] = b"masterserve-heartbeat";

/// ALPN protcol identifier that must be used for client connections.
pub const CLIENT_PROTOCOL: &[u8] = b"masterserve-client";

/// Maximum number of bytes consumed by a heartbeat.
pub const MAX_HEARTBEAT_SIZE: usize = 8 * 1024;
