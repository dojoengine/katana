use clap::Parser;

fn main() {
    if let Err(err) = katana::cli::Cli::parse().run() {
        eprintln!("\x1b[31merror:\x1b[0m {err:?}");
        std::process::exit(1);
    }
}
