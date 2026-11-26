//! `iliada-searchd`: the search service (one per person, started with the
//! session). It indexes the home folder, keeps the index current, and
//! answers the overlay on `$XDG_RUNTIME_DIR/iliada-searchd.sock`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use iliada_search::files::{FileIndex, skipped};
use iliada_search::proto::{Request, Response, socket_path};
use iliada_search::usage::Usage;
use iliada_search::{Engine, actions, agora, apps};
use notify::{EventKind, RecursiveMode, Watcher};

const BUNDLES: [&str; 1] = ["/Applications"];

fn bundle_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = BUNDLES.iter().map(PathBuf::from).collect();
    v.extend(iliada_search::home().map(|h| h.join("Applications")));
    v
}

fn load_apps() -> Vec<apps::App> {
    apps::load(&apps::entry_dirs(), &bundle_dirs())
}

/// When the newest change to any app folder was (to reload apps then).
fn apps_changed() -> Option<SystemTime> {
    apps::entry_dirs().iter().chain(bundle_dirs().iter()).filter_map(|d| d.metadata().ok()?.modified().ok()).max()
}

fn serve(engine: Arc<Engine>, stream: UnixStream) {
    let Ok(read) = stream.try_clone() else { return };
    let mut writer = stream;
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else { return };
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Search { query, homer, limit }) => {
                let t = Instant::now();
                let hits = engine.search(&query, homer, limit);
                Response::Hits { hits, micros: t.elapsed().as_micros() as u64 }
            }
            Ok(Request::OpenWith { path }) => {
                let t = Instant::now();
                let mut hits = engine.open_with(&path);
                hits.extend(actions::extras(&iliada_search::engine::file_hit(&path, path.is_dir())));
                Response::Hits { hits, micros: t.elapsed().as_micros() as u64 }
            }
            Ok(Request::Used { hit }) => {
                engine.used(&hit);
                Response::Done
            }
            Ok(Request::Forget) => {
                let mut u = engine.usage.lock().unwrap_or_else(|e| e.into_inner());
                u.forget();
                let _ = u.save();
                Response::Done
            }
            Ok(Request::Stats) => {
                let f = engine.files.read().unwrap_or_else(|e| e.into_inner());
                Response::Stats {
                    files: f.len() - f.removed(),
                    removed: f.removed(),
                    apps: engine.apps.read().map_or(0, |a| a.len()),
                    offers: engine.offers.read().map_or(0, |o| o.len()),
                    memory: f.memory(),
                    indexing: engine.indexing.load(Ordering::Relaxed),
                }
            }
            Err(e) => Response::Error { message: format!("bad request: {e}") },
        };
        let Ok(mut out) = serde_json::to_vec(&reply) else { return };
        out.push(b'\n');
        if writer.write_all(&out).is_err() {
            return;
        }
    }
}

/// Index the home folder (again), and swap the new index in.
fn reindex(engine: &Engine, home: &Path) {
    engine.indexing.store(true, Ordering::Relaxed);
    let t = Instant::now();
    let index = FileIndex::build(home);
    tracing::info!(
        entries = index.len(),
        ms = t.elapsed().as_millis() as u64,
        mb = index.memory() / 1_000_000,
        "indexed the home folder"
    );
    *engine.files.write().unwrap_or_else(|e| e.into_inner()) = index;
    engine.indexing.store(false, Ordering::Relaxed);
}

/// Whether a changed path is one the index leaves out (hidden, or in a
/// hidden or skipped folder).
fn left_out(home: &Path, path: &Path) -> bool {
    path.strip_prefix(home).map_or(true, |rel| rel.components().any(|c| skipped(&c.as_os_str().to_string_lossy())))
}

/// Follow changes to the home folder. Returns when watching fails (too
/// many folders for inotify): the periodic re-index covers it then.
fn watch(engine: Arc<Engine>, home: PathBuf) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(&home, RecursiveMode::Recursive).context("watching the home folder")?;
    tracing::info!("following changes to the home folder");
    for event in rx {
        let Ok(event) = event else { continue };
        let paths: Vec<_> = event.paths.iter().filter(|p| !left_out(&home, p)).cloned().collect();
        if paths.is_empty() {
            continue;
        }
        use notify::event::{ModifyKind, RenameMode};
        let mut files = engine.files.write().unwrap_or_else(|e| e.into_inner());
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                for p in &paths {
                    add(&mut files, p);
                }
            }
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                for p in &paths {
                    files.remove(p);
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() == 2 => {
                files.remove(&paths[0]);
                add(&mut files, &paths[1]);
            }
            EventKind::Modify(ModifyKind::Name(_)) => {
                for p in &paths {
                    if p.exists() {
                        add(&mut files, p);
                    } else {
                        files.remove(p);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn add(files: &mut FileIndex, p: &Path) {
    let dir = p.is_dir();
    files.insert(p, dir);
    // A folder moved in brings what is in it.
    if dir && std::fs::read_dir(p).is_ok_and(|mut r| r.next().is_some()) {
        files.add_tree(p);
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let home = iliada_search::home().context("HOME is not set")?;
    let sock = socket_path().context("XDG_RUNTIME_DIR is not set")?;

    let offers_cache = agora::cache_path();
    let engine = Arc::new(Engine::new(
        FileIndex::new(&home),
        load_apps(),
        offers_cache.as_deref().map(agora::load_cache).unwrap_or_default(),
        Usage::default_path().as_deref().map(Usage::load).unwrap_or_default(),
        iliada_search::recent::default_path(),
    ));
    *engine.installed.write().unwrap_or_else(|e| e.into_inner()) = agora::installed(&bundle_dirs());

    // Answer at once (apps, Settings and answers work while the home folder
    // is indexed).
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).with_context(|| format!("listening on {}", sock.display()))?;

    // Files: index, then follow changes; index again every few hours, or
    // sooner when many entries were removed.
    {
        let (engine, home) = (engine.clone(), home.clone());
        std::thread::spawn(move || {
            reindex(&engine, &home);
            let watcher = {
                let (engine, home) = (engine.clone(), home.clone());
                std::thread::spawn(move || {
                    if let Err(e) = watch(engine, home) {
                        tracing::warn!("not following changes to the home folder: {e:#}");
                    }
                })
            };
            let mut last = Instant::now();
            loop {
                std::thread::sleep(Duration::from_secs(600));
                let (len, removed) = {
                    let f = engine.files.read().unwrap_or_else(|e| e.into_inner());
                    (f.len(), f.removed())
                };
                let stale = removed * 4 > len;
                let unwatched = watcher.is_finished() && last.elapsed() > Duration::from_secs(1800);
                if stale || unwatched || last.elapsed() > Duration::from_secs(6 * 3600) {
                    reindex(&engine, &home);
                    last = Instant::now();
                }
            }
        });
    }
    // Apps: reload when an app folder changes (an app installed or removed).
    {
        let engine = engine.clone();
        std::thread::spawn(move || {
            let mut seen = apps_changed();
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let now = apps_changed();
                if now != seen {
                    seen = now;
                    *engine.apps.write().unwrap_or_else(|e| e.into_inner()) = load_apps();
                    *engine.installed.write().unwrap_or_else(|e| e.into_inner()) = agora::installed(&bundle_dirs());
                    tracing::info!("apps reloaded");
                }
            }
        });
    }
    // agora: at start, then every six hours.
    {
        let engine = engine.clone();
        std::thread::spawn(move || {
            loop {
                match agora::fetch() {
                    Ok(offers) => {
                        if let Some(c) = &offers_cache {
                            agora::save_cache(c, &offers);
                        }
                        tracing::info!(apps = offers.len(), "agora catalogue");
                        *engine.offers.write().unwrap_or_else(|e| e.into_inner()) = offers;
                    }
                    Err(e) => tracing::warn!("agora catalogue: {e:#}"),
                }
                std::thread::sleep(Duration::from_secs(6 * 3600));
            }
        });
    }

    tracing::info!(socket = %sock.display(), "ready");
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let engine = engine.clone();
                std::thread::spawn(move || serve(engine, s));
            }
            Err(e) => tracing::warn!("accept: {e}"),
        }
    }
    Ok(())
}
