use anyhow::Result;
use clap::Parser;

mod ui;
mod cw;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Config {
    log_group_name: String,
    #[arg(short, long, default_value = "84")]
    snip: Option<usize>
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse();

    let mut cw = cw::Logs::new(config.log_group_name).await;
    for (x, y) in cw.get_more_logs().await {
        let msg = if let Some(snip) = config.snip {
            y.chars().skip(snip).collect::<String>()
        } else {
            y
        };
        println!("{}: {}", x, msg);
    }

    //ui::run().await?;

    Ok(())
}
