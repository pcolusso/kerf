use anyhow::Result;
use clap::Parser;
use aws_sdk_cloudwatch as cw;

mod ui;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Config {
    log_group_name: String
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = aws_config::load_from_env().await;
    let client = aws_sdk_cloudwatch::Client::new(&config);
    let config = Config::parse();

    ui::run().await?;

    Ok(())
}
