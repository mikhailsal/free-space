use clap::Parser;

#[derive(Parser)]
#[command(
    name = "freespace",
    version,
    about = "Disk usage snapshot that is always ready"
)]
struct Cli {}

fn main() {
    let _ = Cli::parse();
    println!("freespace {}", freespace_core::VERSION);
}
