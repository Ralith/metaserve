use futures_util::StreamExt;
use thiserror::Error;

pub use metaserve_proto::client as proto;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Connection(#[from] quinn::ConnectionError),
    #[error(transparent)]
    Read(#[from] quinn::ReadError),
    #[error("server sent malformed data: {0}")]
    Parse(#[from] bincode::Error),
}

pub struct Client {
    inner: quinn::IncomingUniStreams,
    buffer: Vec<u8>,
}

impl Client {
    pub fn new(connection: quinn::NewConnection) -> Self {
        Self {
            inner: connection.uni_streams,
            buffer: Vec::new(),
        }
    }

    pub async fn recv(&mut self) -> Result<proto::Message<'_>, Error> {
        let stream = self
            .inner
            .next()
            .await
            .expect("connection locally closed unexpectedly")?;
        self.buffer = match stream.read_to_end(usize::MAX).await {
            Ok(x) => x,
            Err(quinn::ReadToEndError::TooLong) => unreachable!(),
            Err(quinn::ReadToEndError::Read(x)) => return Err(x.into()),
        };
        Ok(bincode::deserialize(&self.buffer)?)
    }
}
