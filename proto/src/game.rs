//! Protocol for communication between game servers and meta servers

use serde::{Deserialize, Serialize};

/// Message sent by the game server on connect
#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct Hello {
    /// The port game clients should connect to
    pub port: u16,
}

pub struct Update {
}

/// ALPN ID for a game server's heartbeat connection
pub const PROTOCOL: &[u8] = &[
    0x72, 0x7F, 0x4A, 0x53, 0x03, 0xDF, 0xDD, 0xB3, 0xAC, 0x79, 0x9E, 0x0F, 0x49, 0xB1, 0xE3, 0x60,
];
