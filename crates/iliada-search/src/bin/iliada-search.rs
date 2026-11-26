//! `iliada-search`: what the keys run, and a way to try the search from a
//! terminal.
//!
//!     iliada-search                 show or hide the search (Super)
//!     iliada-search homer           the search, asking Homer (Super+O)
//!     iliada-search clipboard       clipboard history (Super+V)
//!     iliada-search query <words>   print results and how long they took
//!     iliada-search stats           what the index holds
//!     iliada-search forget          forget what you opened (frecency)
//!     iliada-search install <id> <name>   install an agora app, with notifications
//!
//! The first three tell the overlay (already running, hidden) through its
//! socket: a few milliseconds, with nothing to load. If it is not running
//! it is started.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::process::{Command, ExitCode};

use iliada_search::proto::{Client, Request, Response};

/// Where the overlay listens.
fn overlay_socket() -> Option<std::path::PathBuf> {
    iliada_search::runtime_dir().map(|d| d.join("iliada-search-overlay.sock"))
}

fn tell_overlay(mode: &str) -> ExitCode {
    if let Some(p) = overlay_socket()
        && let Ok(mut s) = UnixStream::connect(p)
        && writeln!(s, "{mode}").is_ok()
    {
        return ExitCode::SUCCESS;
    }
    // Not running: start it, showing.
    match iliada_search::actions::spawn(&["iliada-search-overlay".into(), "--show".into(), mode.into()]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("iliada-search: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn notify(summary: &str, body: &str) {
    let _ = Command::new("notify-send").args(["--app-name=agora", "--icon=org.iliada.Agora", summary, body]).status();
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("toggle") => tell_overlay("toggle"),
        Some("homer") => tell_overlay("homer"),
        Some("clipboard") => tell_overlay("clipboard"),
        Some("query") => {
            let q = args[1..].join(" ");
            let mut c = match Client::connect() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("iliada-search: is iliada-searchd running? {e:#}");
                    return ExitCode::FAILURE;
                }
            };
            match c.search(&q, true, 12) {
                Ok((hits, micros)) => {
                    for h in hits {
                        println!("{:<12} {:<40} {}", h.kind.label(), h.title, h.subtitle);
                    }
                    println!("({:.2} ms)", micros as f64 / 1000.0);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("iliada-search: {e:#}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(cmd @ ("stats" | "forget")) => {
            let req = if cmd == "stats" { Request::Stats } else { Request::Forget };
            match Client::connect().and_then(|mut c| c.call(&req)) {
                Ok(Response::Stats { files, removed, apps, offers, memory, indexing }) => {
                    println!("files and folders: {files}{}", if indexing { " (indexing…)" } else { "" });
                    println!("removed since indexing: {removed}");
                    println!("apps: {apps}, agora offers: {offers}");
                    println!("index memory: {:.1} MB", memory as f64 / 1e6);
                    ExitCode::SUCCESS
                }
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("iliada-search: {e:#}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("install") if args.len() >= 2 => {
            let id = &args[1];
            let name = args.get(2).unwrap_or(id);
            notify(&format!("Installing {name}"), "From agora. You'll be told when it's ready.");
            let out = Command::new("iliada").args(["install", id]).output();
            match out {
                Ok(o) if o.status.success() => {
                    notify(&format!("{name} is installed"), "Find it in Applications, or search for it.");
                    ExitCode::SUCCESS
                }
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    notify(
                        &format!("{name} could not be installed"),
                        err.lines().last().unwrap_or("Try again in agora."),
                    );
                    ExitCode::FAILURE
                }
                Err(e) => {
                    notify(&format!("{name} could not be installed"), &e.to_string());
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!(
                "usage: iliada-search [toggle | homer | clipboard | query <words> | stats | forget | install <id> <name>]"
            );
            ExitCode::from(2)
        }
    }
}
