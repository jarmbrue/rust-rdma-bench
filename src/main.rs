mod bench;
mod cli;
mod client;
mod comm;
mod device;
mod error;
mod report;
mod server;
mod transport;

use clap::Parser;
use cli::Cli;

fn main() {
    let cli = Cli::parse();

    let result = match cli {
        Cli::Server(args) => server::run(args),
        Cli::Client(args) => client::run(args),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
