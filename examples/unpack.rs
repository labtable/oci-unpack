use std::{
    fmt,
    io::{IsTerminal, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicU64, Ordering},
        RwLock,
    },
    time::{Duration, Instant},
};

use clap::Parser;
use oci_unpack::{EventHandler, Reference, Unpacker};

#[derive(Parser, Debug)]
struct Args {
    /// CPU architecture to download.
    #[arg(short, long)]
    arch: Option<String>,

    /// Operating system to download.
    #[arg(short, long)]
    os: Option<String>,

    /// Skip sandbox if it can't be created.
    #[arg(short, long)]
    can_skip_sandbox: bool,

    /// Show debug messages.
    #[arg(short, long)]
    debug: bool,

    /// Image reference.
    image: String,

    /// Target directory to write the image.
    target: PathBuf,
}

#[derive(Default)]
struct PrinterData {
    layers_total: usize,
    bytes_total: u64,
    last_update: Option<Instant>,
}

#[derive(Default)]
struct Logger {
    debug: bool,
    is_terminal: bool,
    layers_received: AtomicU64,
    bytes_received: AtomicU64,
    current_layer_len: AtomicU64,
    current_layer_position: AtomicU64,
    printer: RwLock<PrinterData>,
}

impl Logger {
    const PRINT_INTERVAL: Duration = Duration::from_millis(100);

    fn show_progress(&self, force: bool) {
        if !force && !self.need_update() {
            return;
        }

        let Ok(mut printer) = self.printer.try_write() else {
            return;
        };

        if printer.bytes_total == 0 {
            return;
        }

        let first_print = printer.last_update.is_none();

        printer.last_update = Some(Instant::now());

        let stdout = std::io::stdout();
        let prefix = if !first_print && self.is_terminal {
            "\x1B[A\r\x1B[K"
        } else {
            ""
        };

        let mut output = std::io::BufWriter::new(stdout);

        let _ = write!(
            output,
            "{}Downloading: {}% of {}",
            prefix,
            100 * self.bytes_received.load(Ordering::SeqCst) / printer.bytes_total,
            HumanSize(printer.bytes_total),
        );

        let layer_len = self.current_layer_len.load(Ordering::SeqCst);
        let layer_pos = self.current_layer_position.load(Ordering::SeqCst);

        if layer_pos < layer_len && layer_len > 0 {
            let _ = write!(
                output,
                "  |  Extracting layer {} of {}: {}% of {}",
                self.layers_received.load(Ordering::SeqCst),
                printer.layers_total,
                100 * layer_pos / layer_len,
                HumanSize(layer_len),
            );
        }

        let _ = output.write_all(b"\n");
    }

    fn need_update(&self) -> bool {
        let printer = self.printer.read().unwrap();
        match &printer.last_update {
            Some(lu) => lu.elapsed() > Self::PRINT_INTERVAL,
            None => true,
        }
    }
}

impl EventHandler for Logger {
    fn registry_request(&self, url: &str) {
        if self.debug {
            println!("GET {url}");
        }
    }

    fn registry_auth(&self, url: &str) {
        if self.debug {
            println!("AUTH {url}");
        }
    }

    fn download_start(&self, layers: usize, bytes: usize) {
        let mut printer = self.printer.write().unwrap();
        printer.layers_total = layers;
        printer.bytes_total = bytes as u64;
    }

    fn download_progress_bytes(&self, bytes: usize) {
        self.bytes_received
            .fetch_add(bytes as u64, Ordering::SeqCst);
        self.show_progress(false);
    }

    fn layer_start(&self, archive_len: u64) {
        self.layers_received.fetch_add(1, Ordering::SeqCst);
        self.current_layer_position.store(0, Ordering::SeqCst);
        self.current_layer_len.store(archive_len, Ordering::SeqCst);
        self.show_progress(false);
    }

    fn layer_progress(&self, position: usize) {
        self.current_layer_position
            .store(position as u64, Ordering::Relaxed);
        self.show_progress(false);
    }

    fn layer_entry_skipped(&self, path: &std::path::Path, cause: &dyn fmt::Display) {
        println!("{path:?}: {cause}");
    }

    #[cfg(feature = "sandbox")]
    fn sandbox_status(&self, status: landlock::RestrictionStatus) {
        if self.debug {
            println!("SANDBOX {status:?}");
        }
    }

    fn finished(&self) {
        self.show_progress(true);
    }
}

struct HumanSize<T>(T);

impl<T: Into<u64> + Copy> fmt::Display for HumanSize<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MB: u64 = 1 << 20;
        let n = self.0.into();
        if n > MB {
            write!(f, "{} M", n / MB)
        } else {
            write!(f, "{}", n)
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let event_handler = Logger {
        debug: args.debug,
        is_terminal: std::io::stdout().is_terminal(),
        ..Logger::default()
    };

    let mut unpacker = Unpacker::new(Reference::try_from(args.image.as_str())?)
        .event_handler(event_handler)
        .require_sandbox(!args.can_skip_sandbox);

    if let Some(arch) = &args.arch {
        unpacker = unpacker.architecture(arch);
    }

    if let Some(os) = &args.os {
        unpacker = unpacker.os(os);
    }

    unpacker.unpack(args.target)?;

    Ok(())
}

fn main() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("{}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
