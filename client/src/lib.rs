extern crate quinn;
extern crate futures;
extern crate masterserve_proto as ms;
extern crate bincode;
extern crate tokio;
#[macro_use]
extern crate failure;
extern crate byteorder;
extern crate bytes;

use std::net::SocketAddr;

use futures::{Stream, Poll, Async};
use byteorder::{ByteOrder, LE};
use quinn::Read;
use bytes::{Bytes, BytesMut};

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

/// Read a stream of state updates from the master server on `connection`.
///
/// `connection` *must* be configured to use the protocol ID `PROTOCOL` alone.
pub fn run(connection: quinn::NewClientConnection) -> impl Stream<Item=impl Stream<Item=Server, Error=Error>, Error=Error> {
    connection.incoming
        .map_err(Error::Connection)
        .filter_map(|stream| match stream {
            quinn::NewStream::Uni(x) => Some(x),
            _ => None,
        })
        .map(|stream| Decoder::new(stream))
}

/// Information about a live server.
#[derive(Debug, Clone)]
pub struct Server {
    /// The address of the server.
    pub address: SocketAddr,
    /// The server's heartbeat information.
    pub info: Bytes,
}

struct Decoder {
    io: quinn::RecvStream,
    buffer: BytesMut,
}

impl Decoder {
    pub fn new(io: quinn::RecvStream) -> Self { Self { io, buffer: BytesMut::with_capacity(2048) } }
}

impl Stream for Decoder {
    type Item = Server;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Server>, Error> {
        loop {
            if self.buffer.len() >= 2 {
                let len = LE::read_u16(&self.buffer[0..2]) as usize;
                if self.buffer.len() >= 2 + len {
                    let buf = self.buffer.split_to(2 + len).freeze();
                    let server = bincode::deserialize::<ms::Server>(&buf[2..]).map_err(Error::Parse)?;
                    return Ok(Async::Ready(Some(Server { address: server.address, info: buf.slice_ref(server.info) })))
                }
            }
            let len = self.buffer.len();
            self.buffer.resize(len + 1024, 0);
            match self.io.poll_read(&mut self.buffer[len..]) {
                Ok(Async::Ready(n)) => { self.buffer.truncate(len + n); }
                Ok(Async::NotReady) => { return Ok(Async::NotReady); }
                Err(quinn::ReadError::Finished) => { return Ok(Async::Ready(None)); }
                Err(e) => { return Err(Error::Io(e)); }
            }
        }
    }
}
