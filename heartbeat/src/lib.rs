use std::{
    io,
    time::{Duration, Instant},
};

use err_derive::Error;
use futures::{Future, Stream};
pub use masterserve_proto::{HEARTBEAT_PROTOCOL as PROTOCOL, MAX_HEARTBEAT_SIZE};

#[derive(Debug, Error)]
pub enum Error {
    #[error(display = "{}", _0)]
    Connect(quinn::ConnectError),
    #[error(display = "{}", _0)]
    Io(io::Error),
}

/// Transmit a stream of heartbeats to the master server on `connection`.
///
/// `connection` *must* be configured to use the protocol ID `PROTOCOL` alone.
/// `heartbeat` will be polled at most every two seconds.
///
/// A 2-second delay is inserted after each hearbeat transmit. To transmit heartbeats less frequently--for example, only
/// when changed--supply a stream that yields heartbeats at the desired rate.
pub fn run<S: Stream<Item = T, Error = ()>, T: AsRef<[u8]>>(
    connection: quinn::NewClientConnection,
    heartbeats: S,
) -> impl Future<Item = (), Error = Error> {
    heartbeats
        .map_err(|()| None)
        .for_each(move |heartbeat| {
            connection
                .connection
                .open_uni()
                .map_err(|x| Error::Io(x.into()))
                .and_then(move |stream| {
                    tokio::io::write_all(stream, heartbeat)
                        .and_then(|(stream, _)| tokio::io::shutdown(stream))
                        .map_err(|x| Error::Io(x.into()))
                })
                .map_err(Some)
                .and_then(|_| {
                    tokio::timer::Delay::new(Instant::now() + Duration::from_secs(2))
                        .then(|_| Ok(()))
                })
        })
        .or_else(|e| e.map_or(Ok(()), Err))
}
