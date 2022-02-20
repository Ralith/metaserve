use tokio::time::{Duration, Instant};

pub use metaserve_proto::game as proto;

pub struct Heartbeat {
    connection: quinn::Connection,
    prev_update: Instant,
}

impl Heartbeat {
    pub async fn new(
        connection: quinn::NewConnection,
        port: u16,
    ) -> Result<Self, quinn::WriteError> {
        let mut stream = connection.connection.open_uni().await?;
        let msg = bincode::serialize(&proto::Hello { port }).unwrap();
        stream.write_all(&msg).await?;

        Ok(Self {
            connection: connection.connection,
            prev_update: Instant::now() - Duration::from_secs(1),
        })
    }

    pub async fn send(&mut self, state: &[u8]) -> Result<(), quinn::WriteError> {
        // Send at most once per second
        tokio::time::sleep_until(self.prev_update + Duration::from_secs(1)).await;
        let mut stream = self.connection.open_uni().await?;
        stream.write_all(state).await?;
        Ok(())
    }
}
