use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    katana::cli::Cli::parse().run()?;
    Ok(())
}
