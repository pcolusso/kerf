use anyhow::Result;
use clap::Parser;

mod ui;
mod cw;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Config {
    log_group: String,
    #[arg(short, long)]
    snip: Option<usize>,
    #[arg(short, long)]
    log_stream: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse();

    let cw = cw::Logs::new(config.log_group, config.log_stream, config.snip).await?;
    ui::run(cw).await?;

    Ok(())
}
