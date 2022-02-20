use std::{
    fs,
    io::{self, Write},
    net::ToSocketAddrs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use metaserve_heartbeat::Heartbeat;

#[derive(Parser, Debug)]
#[clap(name = "print")]
struct Opt {
    /// Meta server to connect to
    #[clap(default_value = "localhost:4433")]
    meta: String,
    /// Additional certificate authority to trust, in DER format
    #[clap(parse(from_os_str), long = "ca")]
    ca: Option<PathBuf>,
}

fn main() {
    let opt = Opt::parse();
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

#[tokio::main(flavor = "current_thread")]
async fn run(options: Opt) -> Result<()> {
    let mut roots = rustls::RootCertStore::empty();
    if let Some(ca_path) = options.ca {
        roots.add(&rustls::Certificate(
            fs::read(&ca_path).context("reading CA")?,
        ))?;
    }
    let mut client_crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(roots)
        .with_no_client_auth();

    client_crypto.alpn_protocols = vec![metaserve_heartbeat::proto::PROTOCOL.into()];
    let mut client_config = quinn::ClientConfig::new(Arc::new(client_crypto));
    Arc::get_mut(&mut client_config.transport)
        .unwrap()
        .keep_alive_interval(Some(std::time::Duration::from_secs(5)))
        .max_concurrent_bidi_streams(0u32.into())
        .max_concurrent_uni_streams(1u32.into());

    let endpoint = quinn::Endpoint::client("[::]:0".parse().unwrap())?;

    let hostname = options.meta.split(':').next().unwrap();

    let addr = options
        .meta
        .to_socket_addrs()
        .map_err(|_| anyhow!("invalid meta server address -- did you forget a port number?"))?
        .next()
        .map_or_else(|| Err(anyhow!("no such hostname")), Ok)?;

    print!("connecting to {}...", addr);
    io::stdout().flush()?;

    let conn = endpoint
        .connect_with(client_config, addr, hostname)?
        .await?;
    println!(" connected");

    let mut heartbeat = Heartbeat::new(conn, 1234).await?;
    let mut i = 0;
    loop {
        let msg = format!("heartbeat #{}", i);
        i += 1;
        heartbeat.send(msg.as_bytes()).await?;
    }
}
