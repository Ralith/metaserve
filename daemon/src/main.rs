use std::{
    cell::RefCell,
    collections::HashMap,
    fmt, fs,
    net::SocketAddr,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use byteorder::{ByteOrder, LE};
use failure::{Error, Fail, ResultExt};
use futures::{
    future::{self, Loop},
    Future, Stream,
};
use masterserve_proto as ms;
use slog::{info, o, Drain, Logger};
use structopt::StructOpt;

type Result<T> = std::result::Result<T, Error>;

pub struct PrettyErr<'a>(&'a Fail);
impl<'a> fmt::Display for PrettyErr<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)?;
        let mut x: &Fail = self.0;
        while let Some(cause) = x.cause() {
            f.write_str(": ")?;
            fmt::Display::fmt(&cause, f)?;
            x = cause;
        }
        Ok(())
    }
}

pub trait ErrorExt {
    fn pretty(&self) -> PrettyErr;
}

impl ErrorExt for Error {
    fn pretty(&self) -> PrettyErr {
        PrettyErr(self.as_fail())
    }
}

#[derive(StructOpt, Debug)]
#[structopt(name = "masterserve")]
struct Opt {
    /// TLS private key in PEM format
    #[structopt(parse(from_os_str), short = "k", long = "key")]
    key: PathBuf,
    /// TLS certificate in PEM format
    #[structopt(parse(from_os_str), short = "c", long = "cert")]
    cert: PathBuf,

    /// Address to listen on
    #[structopt(long = "listen", default_value = "[::]:4433")]
    listen: SocketAddr,
}

fn main() {
    let opt = Opt::from_args();
    let code = {
        let decorator = slog_term::PlainSyncDecorator::new(std::io::stderr());
        let drain = slog_term::FullFormat::new(decorator)
            .use_original_order()
            .build()
            .fuse();
        if let Err(e) = run(Logger::root(drain, o!()), opt) {
            eprintln!("ERROR: {}", e.pretty());
            1
        } else {
            0
        }
    };
    ::std::process::exit(code);
}

struct State {
    servers: HashMap<SocketAddr, Vec<u8>>,
}

impl State {
    fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }
}

fn run(log: Logger, options: Opt) -> Result<()> {
    let mut runtime = tokio::runtime::current_thread::Runtime::new()?;

    let key = {
        let key = fs::read(&options.key).context("failed to read private key")?;
        quinn::PrivateKey::from_pem(&key).context("failed to parse private key")?
    };
    let cert_chain = {
        let cert_chain = fs::read(&options.cert).context("failed to read certificates")?;
        quinn::CertificateChain::from_pem(&cert_chain).context("failed to parse certificates")?
    };
    let mut server = quinn::ServerConfigBuilder::default();
    server.set_protocols(&[ms::HEARTBEAT_PROTOCOL, ms::CLIENT_PROTOCOL]);
    server.set_certificate(cert_chain, key)?;

    let mut builder = quinn::EndpointBuilder::new(quinn::Config {
        stream_window_uni: 1,
        stream_window_bidi: 0,
        stream_receive_window: ms::MAX_HEARTBEAT_SIZE as u64,
        ..Default::default()
    });
    builder.logger(log.clone()).listen(server.build());

    let (_, driver, incoming) = builder.bind(options.listen)?;

    let state = Rc::new(RefCell::new(State::new()));

    runtime.spawn(incoming.for_each(move |conn| {
        let remote = conn.connection.remote_address();
        let log = log.new(o!("peer" => remote));
        match conn.connection.protocol().as_ref().map(|x| &x[..]) {
            Some(ms::HEARTBEAT_PROTOCOL) => {
                tokio_current_thread::spawn(do_heartbeat(log.clone(), state.clone(), conn))
            }
            Some(ms::CLIENT_PROTOCOL) => {
                tokio_current_thread::spawn(do_client(log.clone(), state.clone(), conn))
            }
            None => {
                info!(log, "attempted connection with missing ALPN");
            }
            Some(_) => unreachable!(),
        }
        Ok(())
    }));
    runtime.block_on(driver)?;

    Ok(())
}

fn do_heartbeat(
    log: Logger,
    state: Rc<RefCell<State>>,
    conn: quinn::NewConnection,
) -> impl Future<Item = (), Error = ()> {
    info!(log, "heartbeat established");
    let addr = conn.connection.remote_address();
    let connection = conn.connection;
    conn.incoming
        .map_err(|e| -> Option<Error> { Some(e.context("connection lost").into()) })
        .and_then(|stream| {
            let stream = match stream {
                quinn::NewStream::Uni(stream) => stream,
                quinn::NewStream::Bi(_) => unreachable!(),
            };
            quinn::read_to_end(stream, ms::MAX_HEARTBEAT_SIZE).map_err(|e| match e {
                quinn::ReadError::Finished => None,
                _ => Some(e.into()),
            })
        })
        .and_then({
            let state = state.clone();
            move |(_, buf)| {
                state.borrow_mut().servers.insert(addr, buf.to_vec());
                Ok(())
            }
        })
        // Process at most one update per second
        .for_each(|()| {
            tokio::timer::Delay::new(Instant::now() + Duration::from_secs(1))
                .map_err(|_| unreachable!())
        })
        .map_err({
            let state = state.clone();
            move |e| {
                state.borrow_mut().servers.remove(&addr);
                match e {
                    Some(e) => {
                        info!(
                            log,
                            "heartbeat lost: {reason}",
                            reason = e.pretty().to_string()
                        );
                    }
                    None => {
                        info!(log, "closing connection due to oversized heartbeat");
                        connection.close(1, b"oversized heartbeat");
                    }
                }
            }
        })
}

fn do_client(
    log: Logger,
    state: Rc<RefCell<State>>,
    conn: quinn::NewConnection,
) -> impl Future<Item = (), Error = ()> {
    info!(log, "client connected");
    future::loop_fn((), move |()| {
        let state = state.clone();
        conn.connection
            .open_uni()
            .map_err(Into::into)
            .and_then(move |stream| {
                let state = state.borrow();
                let mut buf = Vec::new();
                for (&address, info) in &state.servers {
                    let start = buf.len();
                    buf.resize(start + 2, 0);
                    bincode::serialize_into(
                        &mut buf,
                        &ms::Server {
                            address,
                            info: &info,
                        },
                    )
                    .unwrap();
                    let end = buf.len();
                    // We know this subtraction won't underflow because `MAX_HEARTBEAT_SIZE` is much smaller than 2^16-1
                    LE::write_u16(&mut buf[start..start + 2], (end - start - 2) as u16)
                }
                tokio::io::write_all(stream, buf)
                    .and_then(|(stream, _)| tokio::io::shutdown(stream))
                    .map(|_| ())
                    .map_err(Into::into)
            })
            // Send at most one update per second
            .and_then(|()| {
                tokio::timer::Delay::new(Instant::now() + Duration::from_secs(1))
                    .map_err(|_| unreachable!())
            })
            .map(|()| Loop::Continue(()))
    })
    .map_err(move |e: Error| {
        info!(
            log,
            "client lost: {reason}",
            reason = e.pretty().to_string()
        );
    })
}
