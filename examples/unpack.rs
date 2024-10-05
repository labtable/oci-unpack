use std::path::PathBuf;

use clap::Parser;
use oci_unpack::{download, EventHandler, Reference};

#[derive(Parser, Debug)]
struct Args {
    /// CPU architecture to download.
    #[arg(short, long)]
    arch: Option<String>,

    /// Operating system to download.
    #[arg(short, long)]
    os: Option<String>,

    /// Image reference.
    image: String,

    /// Target directory to write the image.
    target: PathBuf,
}

struct Logger;

impl EventHandler for Logger {
    fn registry_request(&self, url: &str) {
        println!("GET {url}");
    }

    fn registry_auth(&self, url: &str) {
        println!("AUTH {url}");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    download(
        &Reference::parse(&args.image)?,
        args.arch.as_deref(),
        args.os.as_deref(),
        Logger,
        &args.target,
    )?;

    Ok(())
}
