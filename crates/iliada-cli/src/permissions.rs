//! `iliada permissions`: what every app may use (ADR 0007).
//!
//!   iliada permissions [list] [--json]          every app, at a glance
//!   iliada permissions show <app> [--json]      one app, every capability
//!   iliada permissions set <app> <capability> <value>
//!   iliada permissions reset [<app>]            forget answers (asked again)
//!   iliada permissions check                    is the file readable?
//!   iliada permissions limit <app|all> <capability> deny|read-only|none
//!                                               the computer's limits (administrators)
//!
//! `--json` follows docs/permissions-schema.md, for Homer and other agents.

use anyhow::{Context, Result, bail};
use iliada_bundle::policy::{self, AppFacts, AppReport, Key, Policy, Rule, SCHEMA, Value};
use iliada_system::protocol::Request;
use serde_json::json;

#[derive(clap::Args)]
pub struct Args {
    /// Machine-readable output (docs/permissions-schema.md).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Option<Sub>,
}

#[derive(clap::Subcommand)]
enum Sub {
    /// Every app, what it can touch and its worst case.
    List {
        /// Also list built-in programs and command-line tools (not sandboxed).
        #[arg(long)]
        all: bool,
    },
    /// One app: every capability, what it reaches, and who decided it.
    Show { app: String },
    /// Change one capability (applies the next time the app opens).
    /// Values: allow, deny, ask; folders: read-write, read-only, deny, ask.
    Set { app: String, capability: String, value: String },
    /// Forget answers so the app asks again: one app's, or everyone's.
    Reset { app: Option<String> },
    /// Check the permissions files for mistakes (exit status 1 if any).
    Check,
    /// The computer's limits, for everyone on it (administrators): deny,
    /// read-only (folders), or none to remove. `all` is every app.
    Limit { app: String, capability: String, limit: String },
}

/// The document `--json` prints.
pub fn document(policy: &Policy, reports: &[AppReport]) -> serde_json::Value {
    json!({
        "schema": SCHEMA,
        "files": {
            "user": policy.user_path,
            "system": policy.system_path,
        },
        "readable": policy.readable(),
        "problems": policy.problems(),
        "apps": reports,
    })
}

/// Keep the file's summaries current (never over a file with a mistake,
/// so people see their mistake where they made it).
fn sync(policy: &mut Policy, apps: &[AppFacts]) {
    if policy.readable() && policy.user.problems.is_empty() {
        let _ = policy.edit(apps, |_| Ok(()));
    }
}

fn print_card(r: &AppReport) {
    let how = match (r.sandboxed, r.isolation) {
        (false, _) => "not sandboxed".to_owned(),
        (true, Some(iliada_bundle::IsolationTier::MicroVm)) => "micro-VM".to_owned(),
        (true, _) => "sandbox".to_owned(),
    };
    println!("{} ({}), {}, {how}", r.name, r.id, r.source.name());
    println!("  Can touch:  {}", r.summary.join(", "));
    println!("  Worst case: {}", r.worst_case);
    if let Some(s) = &r.search {
        let line = s.sentence();
        let mut chars = line.chars();
        let line: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
        println!("  Search:     {line}");
    }
    if !r.asks.is_empty() {
        let asks: Vec<String> = r.asks.iter().map(|k| k.label().to_lowercase()).collect();
        println!("  Asks next time it opens: {}", asks.join(", "));
    }
}

fn problems(policy: &Policy) {
    for p in policy.problems() {
        eprintln!("Problem: {p}");
    }
}

pub async fn run(args: Args) -> Result<()> {
    let apps = policy::installed_programs();
    let mut policy = Policy::load();
    match args.cmd.unwrap_or(Sub::List { all: false }) {
        Sub::List { all } => {
            sync(&mut policy, &apps);
            let reports: Vec<AppReport> = apps.iter().map(|a| policy.report(a)).collect();
            if args.json {
                println!("{}", serde_json::to_string_pretty(&document(&policy, &reports))?);
                return Ok(());
            }
            problems(&policy);
            let shown: Vec<&AppReport> = reports.iter().filter(|r| all || r.sandboxed).collect();
            if shown.is_empty() {
                println!("No apps are installed.");
            }
            for r in shown {
                print_card(r);
                println!();
            }
            if let Some(p) = &policy.user_path {
                println!("Your answers: {}", p.display());
            }
            println!("The computer's limits: {}", policy.system_path.display());
            if !all {
                println!("Built-in programs and command-line tools are not sandboxed (`--all` lists them).");
            }
        }
        Sub::Show { app } => {
            let a = policy::find(&apps, &app).with_context(|| format!("{app:?} is not installed"))?.clone();
            sync(&mut policy, &apps);
            let r = policy.report(&a);
            if args.json {
                println!("{}", serde_json::to_string_pretty(&document(&policy, std::slice::from_ref(&r)))?);
                return Ok(());
            }
            problems(&policy);
            print_card(&r);
            println!();
            for line in &r.risk {
                println!("  • {line}");
            }
            println!("\n  Always: {}.", r.always.join("; "));
            println!();
            for k in Key::ALL {
                let e = r.get(k);
                let mut extra = Vec::new();
                if e.value != e.setting {
                    extra.push(format!("your setting: {}", e.setting));
                }
                if let Some(s) = &e.scope {
                    extra.push(s.clone());
                }
                if let Some(n) = &e.note {
                    extra.push(n.clone());
                }
                let extra = if extra.is_empty() { String::new() } else { format!("  ({})", extra.join("; ")) };
                println!("  {:<18} {:<14}{extra}", k.name(), e.value.name());
            }
        }
        Sub::Set { app, capability, value } => {
            let a = policy::find(&apps, &app).with_context(|| format!("{app:?} is not installed"))?.clone();
            let key = Key::parse(&capability).with_context(|| {
                let known: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
                format!("{capability:?} is not a capability (one of: {})", known.join(", "))
            })?;
            let v = policy::parse_choice(key, &value)
                .with_context(|| format!("{value:?} is not a value (use one of: {})", choice_list(key)))?;
            policy.edit(&apps, |p| p.set(&a, key, v)).map_err(anyhow::Error::msg)?;
            let now = policy.report(&a).get(key).clone();
            let mut msg = format!("{}: {} is now {}", a.name, key.name(), v);
            if now.value != v {
                msg.push_str(&format!(" (in effect: {}: {})", now.value, now.note.unwrap_or_default()));
            }
            println!("{msg}. This applies the next time {} opens.", a.name);
        }
        Sub::Reset { app } => {
            let target =
                app.as_ref().map(|n| policy::find(&apps, n).map(|a| a.id.clone()).unwrap_or_else(|| n.clone()));
            if target.is_none()
                && policy.user.broken
                && let Some(path) = &policy.user_path
            {
                let kept = path.with_extension("toml.broken");
                std::fs::copy(path, &kept)?;
                eprintln!("Kept the file with the mistake as {}", kept.display());
            }
            if target.is_some() && policy.user.broken {
                bail!(
                    "the permissions file has a mistake: fix it, or reset everything with `iliada permissions reset`"
                );
            }
            policy
                .edit(&apps, |p| {
                    p.reset(target.as_deref());
                    Ok(())
                })
                .map_err(anyhow::Error::msg)?;
            match target {
                Some(id) => println!("Forgot the answers for {id}: it asks again the next time it opens."),
                None => println!("Forgot every answer: apps ask again the next time they open."),
            }
        }
        Sub::Check => {
            let found = policy.problems();
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "readable": policy.readable(), "problems": found }))?
                );
            } else if found.is_empty() {
                println!("No problems.");
            } else {
                problems(&policy);
            }
            if !found.is_empty() {
                std::process::exit(1);
            }
        }
        Sub::Limit { app, capability, limit } => {
            let app = if app == "all" { app } else { policy::find(&apps, &app).map(|a| a.id.clone()).unwrap_or(app) };
            let limit = (limit != "none").then_some(limit);
            let r = iliada_system::client::call(&Request::SetPermissionLimit { app, capability, limit }, &mut |_| {})
                .await?;
            match r["change"].as_str() {
                Some(c) => println!("{c}. This applies the next time the app opens."),
                None => println!("Nothing changed."),
            }
        }
    }
    Ok(())
}

fn choice_list(key: Key) -> String {
    match key.rule() {
        Rule::Decided { .. } => policy::choices(key).iter().map(|v| v.name()).collect::<Vec<_>>().join(", "),
        _ => format!("{} (fixed)", policy::choices(key).first().copied().unwrap_or(Value::Deny)),
    }
}
