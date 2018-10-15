#[macro_use]
extern crate serde_derive;
extern crate serde;

use std::net::SocketAddr;

#[derive(Serialize, Deserialize)]
pub struct Server<'a> {
    pub address: SocketAddr,
    pub info: &'a [u8],
}

#[derive(Serialize, Deserialize)]
pub struct Heartbeat<'a> {
    pub info: &'a [u8],
}

pub const HEARTBEAT_PROTOCOL: &[u8] = b"masterserve-heartbeat";
pub const CLIENT_PROTOCOL: &[u8] = b"masterserve-client";
