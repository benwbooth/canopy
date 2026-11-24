//! canopyd: canopy's sync daemon (no display needed). The window and the
//! `canopy` command start it when it isn't running; a systemd user unit
//! can too.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("canopyd {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("canopyd: keeps Google Drive and your folders in sync.\n\nUsually started by canopy. Talk to it with `canopy status`, `canopy pause`...");
        return;
    }
    if let Err(e) = canopy_core::daemon::run() {
        eprintln!("canopyd: {e:#}");
        std::process::exit(1);
    }
}
