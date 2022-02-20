//! Protocol for communication between game clients and meta servers

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message<'a> {
    #[serde(borrow)]
    pub servers: Vec<Server<'a>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Server<'a> {
    pub id: u64,
    /// Information about the game server's state
    #[serde(borrow)]
    pub event: Event<'a>,
}

/// Change in a game server's state
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Event<'a> {
    Shutdown,
    /// The game server changed state
    Update(SocketAddr, &'a [u8]),
}

/// ALPN ID for client connections
pub const PROTOCOL: &[u8] = &[
    0xB6, 0x46, 0x55, 0x6E, 0x05, 0x65, 0xD0, 0x9C, 0xD2, 0xFA, 0xEE, 0x31, 0xFD, 0x8A, 0x0A, 0x95,
];
