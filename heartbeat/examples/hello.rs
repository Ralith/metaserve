use masterserve_heartbeat as heartbeat;

use std::{
    fs,
    io::{self, Write},
    net::ToSocketAddrs,
    path::PathBuf,
};

use err_ctx::ResultExt;
use futures::{stream, Future, Stream};
use structopt::StructOpt;

type Error = Box<std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

#[derive(StructOpt, Debug)]
#[structopt(name = "hello")]
struct Opt {
    /// Master server to connect to
    #[structopt(default_value = "localhost:4433")]
    master: String,
    /// Additional certificate authority to trust, in DER format
    #[structopt(parse(from_os_str), long = "ca")]
    ca: Option<PathBuf>,
}

fn main() {
    let opt = Opt::from_args();
    let code = {
        if let Err(e) = run(opt) {
            eprintln!("ERROR: {}", e);
            1
        } else {
            0
        }
    };
    ::std::process::exit(code);
}

fn run(options: Opt) -> Result<()> {
    let mut runtime = tokio::runtime::current_thread::Runtime::new()?;

    let (endpoint, driver, _) = quinn::Endpoint::new().bind("[::]:0")?;
    runtime.spawn(driver.map_err(|e| eprintln!("IO error: {}", e)));

    let hostname = options.master.split(':').next().unwrap();

    let addr = options
        .master
        .to_socket_addrs()
        .map_err(|_| "invalid master server address -- did you forget a port number?")?
        .next()
        .map_or_else(|| Err("no such hostname"), Ok)?;

    let mut config = quinn::ClientConfigBuilder::new();
    config.set_protocols(&[heartbeat::PROTOCOL]);
    if let Some(ref ca) = options.ca {
        let ca = fs::read(ca).ctx("reading CA cert")?;
        let ca = quinn::Certificate::from_der(&ca).ctx("parsing CA cert")?;
        config.add_certificate_authority(ca).ctx("using CA cert")?;
    }
    let config = config.build();

    print!("connecting to {}...", addr);
    io::stdout().flush()?;

    runtime.block_on(
        endpoint
            .connect_with(&config, &addr, hostname)?
            .map_err(|e| -> Error { e.into() })
            .and_then(|conn| {
                println!(" connected");
                let beats =
                    stream::unfold(0, |n| Some(Ok((format!("hello, world #{}", n), n + 1))))
                        .inspect(|_| {
                            print!(".");
                            io::stdout().flush().unwrap();
                        });
                heartbeat::run(conn, beats).map_err(Into::into)
            }),
    )?;

    Ok(())
}
