extern crate quinn;
extern crate futures;
extern crate masterserve_proto as ms;
extern crate bincode;
extern crate tokio;
#[macro_use]
extern crate failure;

use std::{io, time::{Instant, Duration}};

use futures::{Future, Stream};

/// ALPN protcol identifier that must be used for heartbeat connections.
pub use ms::HEARTBEAT_PROTOCOL as PROTOCOL;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "{}", _0)]
    Connect(quinn::ConnectError),
    #[fail(display = "{}", _0)]
    Io(io::Error),
}

/// Transmit a stream of heartbeats to the master server on `connection`.
///
/// `connection` *must* be configured to use the protocol ID `PROTOCOL` alone.
/// `heartbeat` will be polled at most every two seconds.
pub fn run<S: Stream<Item=T, Error=()>, T: AsRef<[u8]>>(
    connection: quinn::NewClientConnection,
    heartbeats: S,
) -> impl Future<Item=(), Error=Error> {
    heartbeats
        .map_err(|()| None)
        .for_each(move |x| {
            connection.connection.open_uni()
                .map_err(|x| Error::Io(x.into()))
                .and_then(move |stream| {
                    tokio::io::write_all(stream, bincode::serialize(&ms::Heartbeat { info: x.as_ref() }).unwrap())
                        .and_then(|(stream, _)| tokio::io::shutdown(stream))
                        .map_err(|x| Error::Io(x.into()))
                })
                .map_err(Some)
                .and_then(|_| tokio::timer::Delay::new(Instant::now() + Duration::from_secs(2)).then(|_| Ok(())))
        })
        .or_else(|e| e.map_or(Ok(()), Err))
}
