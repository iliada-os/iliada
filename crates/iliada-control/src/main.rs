//! `iliada-control`: Homer's control of other apps (the user service), and
//! the person's switches for it.
//!
//! ```text
//! iliada-control serve            the service (a systemd user unit)
//! iliada-control stop             stop Homer now (Super+Escape, the banner's Stop)
//! iliada-control status           what Homer may use
//! iliada-control on | off         the global switch
//! iliada-control allow <app>      let Homer use an app (bundle id, host:<program> or screen)
//! iliada-control deny <app>
//! iliada-control forget <app>     ask again next time
//! iliada-control note <title> <text>   a new note in Text Editor (an intent)
//! ```

use anyhow::{Context, Result};
use iliada_bundle::control::{Control, ControlPolicy};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn request(tool: &str) -> Result<serde_json::Value> {
    let path = iliada_control::socket_path().context("XDG_RUNTIME_DIR is not set")?;
    let stream = tokio::net::UnixStream::connect(&path).await.context("Homer's app control is not running")?;
    let (r, mut w) = stream.into_split();
    w.write_all(format!("{}\n", serde_json::json!({ "tool": tool })).as_bytes()).await?;
    let line = BufReader::new(r).lines().next_line().await?.unwrap_or_default();
    Ok(serde_json::from_str(&line)?)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("serve");
    let path = ControlPolicy::default_path().context("HOME is not set")?;
    match cmd {
        "serve" => {
            tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
            let sock = iliada_control::socket_path().context("XDG_RUNTIME_DIR is not set")?;
            let server = iliada_control::server::Server::new(iliada_control::server::Config::user());
            iliada_control::server::serve(server, &sock).await?;
        }
        "stop" => {
            let r = request("stop").await;
            // Shown even when the service was not running.
            let _ = std::process::Command::new("notify-send")
                .args(["--app-name=Homer", "Homer stopped", "Homer won't touch your apps until you ask again."])
                .status();
            r?;
        }
        "status" => {
            let p = ControlPolicy::load(&path);
            println!("Homer may use other apps: {}", if p.enabled { "yes" } else { "no" });
            for (app, c) in &p.apps {
                println!("  {app}: {c:?}");
            }
            if let Ok(r) = request("status").await
                && r["stopped"] == true
            {
                println!("Stopped until the next message to Homer.");
            }
        }
        "on" | "off" => {
            let mut p = ControlPolicy::load(&path);
            p.enabled = cmd == "on";
            p.save(&path)?;
        }
        "note" => {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from).context("HOME is not set")?;
            let title = args.get(1).map(String::as_str).unwrap_or("Note");
            let text = args.get(2).map(String::as_str).unwrap_or("");
            let note = iliada_control::note::write(&home, title, text)?;
            println!("{}", note.display());
            let _ = std::process::Command::new("cosmic-edit")
                .arg(&note)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        "allow" | "deny" | "forget" => {
            let app = args.get(1).context("which app? (its id, host:<program>, or screen)")?;
            let mut p = ControlPolicy::load(&path);
            p.set(
                app,
                match cmd {
                    "allow" => Control::Allow,
                    "deny" => Control::Deny,
                    _ => Control::Ask,
                },
            );
            p.save(&path)?;
        }
        _ => {
            eprintln!(
                "usage: iliada-control [serve | stop | status | on | off | allow <app> | deny <app> | forget <app> | note <title> <text>]"
            );
            std::process::exit(2);
        }
    }
    Ok(())
}
