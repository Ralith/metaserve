extern crate failure;
extern crate futures;
extern crate quinn;
extern crate tokio;
extern crate tokio_current_thread;
#[macro_use]
extern crate structopt;
#[macro_use]
extern crate slog;
extern crate bincode;
extern crate masterserve_proto as ms;
extern crate rustls;
extern crate slog_term;

use std::{
    cell::RefCell, collections::HashMap, fmt, fs, io, net::SocketAddr, path::PathBuf, rc::Rc,
    time::{Instant, Duration},
};

use failure::{err_msg, Error, Fail, ResultExt};
use futures::{Future, Stream, future::{self, Loop}};
use rustls::internal::pemfile;
use slog::{Drain, Logger};
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

    let mut builder = quinn::EndpointBuilder::from_config(quinn::Config {
        max_remote_uni_streams: 1,
        ..Default::default()
    });
    builder
        .set_protocols(&[
            ms::HEARTBEAT_PROTOCOL,
            ms::CLIENT_PROTOCOL,
        ]).logger(log.clone())
        .listen();

    let keys = {
        let mut reader =
            io::BufReader::new(fs::File::open(&options.key).context("failed to read private key")?);
        pemfile::rsa_private_keys(&mut reader).map_err(|_| err_msg("failed to read private key"))?
    };
    let cert_chain = {
        let mut reader = io::BufReader::new(
            fs::File::open(&options.cert).context("failed to read private key")?,
        );
        pemfile::certs(&mut reader).map_err(|_| err_msg("failed to read certificates"))?
    };
    builder.set_certificate(cert_chain, keys[0].clone())?;

    let (_, driver, incoming) = builder.bind(options.listen)?;

    let state = Rc::new(RefCell::new(State::new()));

    runtime.spawn(incoming.for_each(move |conn| {
        let remote = conn.connection.remote_address();
        let log = log.new(o!("peer" => remote));
        match conn.connection.protocol().as_ref().map(|x| -> &[u8] { &x }) {
            Some(ms::HEARTBEAT_PROTOCOL) => tokio_current_thread::spawn(do_heartbeat(
                log.clone(),
                remote,
                state.clone(),
                conn.incoming,
            )),
            Some(ms::CLIENT_PROTOCOL) => tokio_current_thread::spawn(do_client(log.clone(), state.clone(), conn.connection)),
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
    addr: SocketAddr,
    state: Rc<RefCell<State>>,
    incoming: impl Stream<Item = quinn::NewStream, Error = quinn::ConnectionError>,
) -> impl Future<Item = (), Error = ()> {
    info!(log, "heartbeat established");
    incoming.map_err(|e| -> Error { e.context("connection lost").into() })
        .and_then(|stream| {
            let stream = match stream {
                quinn::NewStream::Uni(stream) => stream,
                quinn::NewStream::Bi(_) => unreachable!(), // config.max_remote_bi_streams is defaulted to 0
            };
            quinn::read_to_end(stream, 8 * 1024)
                .map_err(Into::into)
        })
        .and_then({ let state = state.clone(); move |(_, buf)| {
            let heartbeat = bincode::deserialize::<ms::Heartbeat>(&buf).context("malformed heartbeat")?;
            state.borrow_mut().servers.insert(addr, heartbeat.info.into());
            Ok(())
        }})
    // Process at most one update per second
        .for_each(|()| tokio::timer::Delay::new(Instant::now() + Duration::from_secs(1)).map_err(|_| unreachable!()))
        .map_err({ let state = state.clone(); move |e| {
            info!(log, "heartbeat lost: {reason}", reason=e.pretty().to_string());
            state.borrow_mut().servers.remove(&addr);
        }})
}

fn do_client(log: Logger, state: Rc<RefCell<State>>, conn: quinn::Connection) -> impl Future<Item = (), Error = ()> {
    info!(log, "client connected");
    future::loop_fn((), move |()| {
        let state = state.clone();
        conn.open_uni()
            .map_err(Into::into)
            .and_then(move |stream| {
                let state = state.borrow();
                let state = ms::State {
                    servers: state.servers.iter().map(|(&address, info)| ms::Server { address, info: &info }).collect(),
                };
                tokio::io::write_all(stream, bincode::serialize(&state).unwrap())
                    .and_then(|(stream, _)| tokio::io::shutdown(stream))
                    .map(|_| ())
                    .map_err(Into::into)
            })
        // Send at most one update per second
            .and_then(|()| tokio::timer::Delay::new(Instant::now() + Duration::from_secs(1)).map_err(|_| unreachable!()))
            .map(|()| Loop::Continue(()))
    }).map_err(move |e: Error| {
        info!(log, "client lost: {reason}", reason=e.pretty().to_string());
    })
}
