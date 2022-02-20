use std::{
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use clap::Parser;
use futures_util::StreamExt;
use indexmap::IndexSet;
use metaserve_proto as ms;
use slab::Slab;
use tokio::sync::Notify;
use tracing::{debug, error, info, Instrument};

#[derive(Parser, Debug)]
#[clap(name = "metaserve")]
struct Opt {
    /// TLS private key in DER format
    #[clap(parse(from_os_str), short = 'k', long = "key")]
    private_key: PathBuf,
    /// TLS certificate in DER format
    #[clap(parse(from_os_str), short = 'c', long = "cert")]
    certificate: PathBuf,

    /// Maximum size of server state to accept
    #[clap(short = 's', long = "state-size", default_value = "8192")]
    state_size: usize,

    /// Address to listen on
    #[clap(long = "listen", default_value = "[::]:4433")]
    listen: SocketAddr,
}

#[tokio::main]
async fn run(options: Opt) -> Result<()> {
    let key =
        rustls::PrivateKey(fs::read(&options.private_key).context("failed to read private key")?);
    let cert_chain = vec![rustls::Certificate(
        fs::read(&options.certificate).context("failed to read certificate")?,
    )];
    let mut server_crypto = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;
    server_crypto.alpn_protocols = vec![ms::client::PROTOCOL.into(), ms::game::PROTOCOL.into()];
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(server_crypto));
    server_config.use_retry(true);
    Arc::get_mut(&mut server_config.transport)
        .unwrap()
        .max_concurrent_uni_streams(1u32.into())
        .max_concurrent_bidi_streams(0u32.into())
        .stream_receive_window(
            options
                .state_size
                .try_into()
                .context("failed to set stream window size")?,
        );
    let (endpoint, incoming) = quinn::Endpoint::server(server_config, options.listen)?;
    debug!("listening on {}", endpoint.local_addr()?);

    let state = Arc::new(State::new(options));
    state.run(incoming).await
}

fn main() {
    use tracing_subscriber::{
        filter, fmt, layer::SubscriberExt, registry, util::SubscriberInitExt,
    };

    let opt = Opt::parse();
    let journald = tracing_journald::layer();
    let (stdout, stdout_guard) = tracing_appender::non_blocking(std::io::stdout());
    let fmt = if journald.is_err() || std::env::var_os("INVOCATION_ID").is_none() {
        Some(fmt::layer().with_target(false).with_writer(stdout))
    } else {
        None
    };
    let registry = registry().with(fmt).with(
        filter::EnvFilter::from_default_env()
            .add_directive(tracing_subscriber::filter::LevelFilter::INFO.into()),
    );
    match journald {
        Ok(layer) => {
            registry.with(layer).init();
        }
        Err(_e) => {
            registry.init();
            info!("couldn't connect to journald: {}", _e);
        }
    }
    let code = match run(opt) {
        Err(e) => {
            error!("{}", e);
            1
        }
        Ok(()) => 0,
    };
    drop(stdout_guard);
    ::std::process::exit(code);
}

struct State {
    options: Opt,
    dirty: Notify,
    inner: Mutex<Inner>,
}

impl State {
    fn new(options: Opt) -> Self {
        Self {
            options,
            dirty: Notify::new(),
            inner: Mutex::new(Inner {
                clients: Slab::new(),
                servers: Slab::new(),
            }),
        }
    }

    async fn run(self: Arc<Self>, mut incoming: quinn::Incoming) -> Result<()> {
        while let Some(conn) = incoming.next().await {
            tokio::spawn(self.clone().dispatch(conn));
        }
        Ok(())
    }

    async fn dispatch(self: Arc<Self>, conn: quinn::Connecting) {
        match conn.await {
            Ok(conn) => {
                let hs = conn
                    .connection
                    .handshake_data()
                    .unwrap()
                    .downcast::<quinn::crypto::rustls::HandshakeData>()
                    .unwrap();
                match hs.protocol.as_ref().map(|x| &x[..]).unwrap() {
                    ms::game::PROTOCOL => self.handle_server(conn).await,
                    ms::client::PROTOCOL => self.handle_client(conn).await,
                    _ => unreachable!(),
                }
            }
            Err(e) => {
                info!("handshake failed: {}", e);
            }
        }
    }

    async fn handle_server(self: Arc<Self>, conn: quinn::NewConnection) {
        let id = self.inner.lock().unwrap().servers.insert(Server {
            state: Vec::new(),
            address: None,
        });
        let span = tracing::error_span!("server", id);
        async move {
            info!(address = %conn.connection.remote_address(), "connected");
            if let Err(e) = self.server_inner(conn, id).await {
                info!("connection lost: {}", e);
                {
                    let mut inner = self.inner.lock().unwrap();
                    inner.servers.remove(id);
                    for (_, client) in &mut inner.clients {
                        client.dirty.remove(&id);
                        client.lost.push(id);
                    }
                }
                self.dirty.notify_waiters();
            }
        }
        .instrument(span)
        .await;
    }

    async fn server_inner(&self, mut conn: quinn::NewConnection, id: usize) -> Result<()> {
        let hello = match conn.uni_streams.next().await {
            Some(x) => x?,
            None => return Ok(()),
        };
        let hello = hello.read_to_end(self.options.state_size).await?;
        let hello = bincode::deserialize::<ms::game::Hello>(&hello).context("decoding hello")?;

        while let Some(stream) = conn.uni_streams.next().await {
            let stream = stream?;
            let state = stream.read_to_end(self.options.state_size).await?;
            let addr = SocketAddr::new(conn.connection.remote_address().ip(), hello.port);
            let dirty = {
                let mut inner = self.inner.lock().unwrap();
                let server = &mut inner.servers[id];
                let dirty = state != server.state || Some(addr) != server.address;
                if dirty {
                    server.state = state;
                    server.address = Some(addr);
                    for (_, client) in &mut inner.clients {
                        client.dirty.insert(id);
                    }
                }
                dirty
            };
            if dirty {
                self.dirty.notify_waiters();
            }
            // Read at most one heartbeat per second
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        Ok(())
    }

    async fn handle_client(self: Arc<Self>, conn: quinn::NewConnection) {
        let id = {
            let mut inner = self.inner.lock().unwrap();
            let client = Client {
                dirty: inner.servers.iter().map(|(id, _)| id).collect(),
                lost: Vec::new(),
            };
            inner.clients.insert(client)
        };
        let span = tracing::error_span!("client", id);
        async move {
            info!(address = %conn.connection.remote_address(), "connected");
            if let Err(e) = self.client_inner(conn, id).await {
                info!("connection lost: {}", e);
                {
                    let mut inner = self.inner.lock().unwrap();
                    inner.clients.remove(id);
                }
            }
        }
        .instrument(span)
        .await;
    }

    async fn client_inner(&self, mut conn: quinn::NewConnection, id: usize) -> Result<()> {
        loop {
            let mut stream = conn.connection.open_uni().await?;
            let msg = {
                let inner = &mut *self.inner.lock().unwrap();
                let client = &mut inner.clients[id];
                let msg = ms::client::Message {
                    servers: client
                        .lost
                        .drain(..)
                        .map(|id| ms::client::Server {
                            id: id as u64,
                            event: ms::client::Event::Shutdown,
                        })
                        .chain(client.dirty.drain(..).map(|id| {
                            let x = &inner.servers[id];
                            ms::client::Server {
                                id: id as u64,
                                event: ms::client::Event::Update(
                                    x.address.expect("dirty server without addr"),
                                    &x.state,
                                ),
                            }
                        }))
                        .collect(),
                };
                bincode::serialize(&msg).unwrap()
            };
            stream.write_all(&msg).await?;
            drop(stream);

            // Update at most once per second
            let dirty = self.dirty.notified();
            let should_transmit = async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                dirty.await;
            };
            tokio::select! {
                _ = should_transmit => {}
                Some(Err(e)) = conn.bi_streams.next() => {
                    return Err(e.into());
                }
            }
        }
    }
}

struct Inner {
    servers: Slab<Server>,
    clients: Slab<Client>,
}

struct Server {
    address: Option<SocketAddr>,
    state: Vec<u8>,
}

struct Client {
    dirty: IndexSet<usize>,
    lost: Vec<usize>,
}
