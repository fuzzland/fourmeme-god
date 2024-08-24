mod meme;
mod run;
mod search;
mod start;
mod strategy;

extern crate core;

use clap::Parser;

#[derive(Debug, clap::Parser)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    Run(run::Args),
    Start(start::Args),
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    match args.command {
        Command::Run(sub_args) => run::run(sub_args).await,
        Command::Start(sub_args) => start::run(sub_args).await,
    }
}
