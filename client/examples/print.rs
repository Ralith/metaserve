use std::{
    io::{self, Write},
    net::ToSocketAddrs,
};

use failure::{err_msg, Error};
use futures::{Future, Stream};
use masterserve_client as client;
use structopt::StructOpt;

type Result<T> = ::std::result::Result<T, Error>;

#[derive(StructOpt, Debug)]
#[structopt(name = "print")]
struct Opt {
    /// Master server to connect to
    #[structopt(default_value = "localhost:4433")]
    master: String,
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

    let (endpoint, driver, _) = quinn::EndpointBuilder::new(quinn::Config {
        stream_window_bidi: 0,
        stream_window_uni: 1,
        ..Default::default()
    })
    .bind("[::]:0")?;
    runtime.spawn(driver.map_err(|e| eprintln!("IO error: {}", e)));

    let hostname = options.master.split(':').next().unwrap();

    let addr = options
        .master
        .to_socket_addrs()
        .map_err(|_| err_msg("invalid master server address -- did you forget a port number?"))?
        .next()
        .map_or_else(|| Err(err_msg("no such hostname")), Ok)?;

    let mut config = quinn::ClientConfigBuilder::new();
    config.set_protocols(&[client::PROTOCOL]);
    let config = config.build();

    print!("connecting to {}...", addr);
    io::stdout().flush()?;

    runtime.block_on(
        endpoint
            .connect_with(&config, &addr, hostname)?
            .map_err(|e| -> Error { e.into() })
            .and_then(|conn| {
                println!(" connected");
                client::run(conn)
                    .for_each(|state| {
                        println!("state:");
                        state.for_each(|server| {
                            println!(
                                "\t{} {}",
                                server.address,
                                String::from_utf8_lossy(&server.info)
                            );
                            Ok(())
                        })
                    })
                    .map_err(Into::into)
            }),
    )?;

    Ok(())
}
