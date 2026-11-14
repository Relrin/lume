//! `lume-host --endpoint <name>`: a session host in its own process.
//!
//! It serves one client on the named per-user endpoint and exits when that client
//! disconnects. The sessions the client started end with it.

use std::process::ExitCode;

use lume_host::{Host, HostConfig};
use lume_ipc::{Endpoint, Listener};

const USAGE: &str = "usage: lume-host --endpoint <name>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [flag, name] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if flag != "--endpoint" {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    match run(name) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lume-host: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let endpoint = Endpoint::named(name)?;
        let mut listener = Listener::bind(&endpoint)?;
        let client = listener.accept().await?;
        // One client only: stop listening so a second cannot wait for ever.
        drop(listener);
        Host::new(HostConfig::default()).serve(client).await?;
        Ok(())
    })
}
