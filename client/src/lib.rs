extern crate quinn;
extern crate futures;
extern crate masterserve_proto as ms;
extern crate bincode;
extern crate tokio;
#[macro_use]
extern crate failure;

use std::net::SocketAddr;

use futures::{Future, Stream};

/// ALPN protcol identifier that must be used for client connections.
pub use ms::CLIENT_PROTOCOL as PROTOCOL;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "{}", _0)]
    Connection(quinn::ConnectionError),
    #[fail(display = "{}", _0)]
    Io(quinn::ReadError),
    #[fail(display = "malformed data: {}", _0)]
    Parse(bincode::Error),
}

#[derive(Debug, Clone)]
pub struct State {
    pub servers: Vec<(SocketAddr, Vec<u8>)>,
}

/// Read a stream of state updates from the master server on `connection`.
///
/// `connection` *must* be configured to use the protocol ID `PROTOCOL` alone.
pub fn run(connection: quinn::NewClientConnection) -> impl Stream<Item=State, Error=Error> {
    connection.incoming
        .map_err(Error::Connection)
        .filter_map(|stream| match stream {
            quinn::NewStream::Uni(x) => Some(x),
            _ => None,
        })
        .and_then(|stream| quinn::read_to_end(stream, 64 * 1024).map_err(Error::Io))
        .and_then(|(_, buffer)| {
            let state = bincode::deserialize::<ms::State>(&buffer).map_err(Error::Parse)?;
            Ok(State {
                servers: state.servers.into_iter().map(|x| (x.address, x.info.into())).collect(),
            })
        })
}
