mod player;
#[cfg(target_os = "linux")]
#[path = "gtk.rs"]
mod window;
#[cfg(target_os = "macos")]
#[path = "appkit.rs"]
mod window;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("Sanim Presenter supports Linux and macOS");

use player::Player;
use std::{path::Path, process::ExitCode};

const NAME: &str = "Sanim Presenter";
const WINDOW_SIZE: (i32, i32) = (960, 540);
const USAGE: &str = "Usage: sanim-presenter <deck.sanim>";

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let arg = args.next().ok_or(USAGE)?;
    if args.next().is_some() {
        return Err(USAGE.into());
    }
    if arg == "--help" || arg == "-h" {
        println!("{USAGE}");
        println!(
            "Right, Page Down, or Space go forward. Left or Page Up go back. F11 toggles fullscreen."
        );
    } else if arg == "--version" || arg == "-V" {
        println!("sanim-presenter {}", env!("CARGO_PKG_VERSION"));
    } else {
        let path = Path::new(&arg);
        let (name, player) = Player::open(path)?;
        window::run(&format!("{name} — {NAME}"), player)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
