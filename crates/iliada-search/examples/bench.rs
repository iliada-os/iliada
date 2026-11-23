//! Search speed on a large home folder: 1,000,000 made-up paths.
//!
//!     cargo run --release -p iliada-search --example bench [-- <paths>]
//!
//! Prints the time to index them, the memory used, and the time per
//! keystroke for typical queries (each typed one letter at a time, as the
//! overlay asks), file index alone and the whole engine.

use std::path::{Path, PathBuf};
use std::time::Instant;

use iliada_search::Engine;
use iliada_search::apps::parse_entry;
use iliada_search::files::FileIndex;
use iliada_search::usage::Usage;

const WORDS: [&str; 48] = [
    "report",
    "invoice",
    "photo",
    "holiday",
    "crete",
    "project",
    "notes",
    "draft",
    "final",
    "budget",
    "tax",
    "meeting",
    "slides",
    "thesis",
    "chapter",
    "iliad",
    "odyssey",
    "homer",
    "athens",
    "nicosia",
    "music",
    "mix",
    "video",
    "backup",
    "config",
    "src",
    "main",
    "test",
    "lib",
    "readme",
    "design",
    "logo",
    "icon",
    "contract",
    "letter",
    "cv",
    "resume",
    "receipt",
    "bank",
    "statement",
    "screenshot",
    "scan",
    "recipe",
    "plan",
    "data",
    "export",
    "summary",
    "archive",
];
const EXTS: [&str; 12] = ["pdf", "txt", "md", "jpg", "png", "docx", "xlsx", "rs", "py", "mp3", "mp4", "zip"];

/// A deterministic pseudo-random sequence (no dependency).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize
    }
}

fn fake_paths(n: usize) -> Vec<(PathBuf, bool)> {
    let root = Path::new("/home/bench");
    let mut rng = Rng(0x2545F4914F6CDD1D);
    let mut dirs: Vec<PathBuf> = vec![root.to_owned()];
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let parent = dirs[rng.next() % dirs.len()].clone();
        let depth = parent.components().count();
        let name =
            format!("{}-{}{}", WORDS[rng.next() % WORDS.len()], WORDS[rng.next() % WORDS.len()], rng.next() % 1000);
        // About one entry in twelve is a folder, and homes are rarely deep.
        if rng.next().is_multiple_of(12) && depth < 12 {
            let p = parent.join(&name);
            dirs.push(p.clone());
            out.push((p, true));
        } else {
            out.push((parent.join(format!("{name}.{}", EXTS[rng.next() % EXTS.len()])), false));
        }
    }
    out
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1_000_000);
    let paths = fake_paths(n);
    let t = Instant::now();
    let mut ix = FileIndex::new("/home/bench");
    for (p, d) in &paths {
        ix.insert(p, *d);
    }
    let build = t.elapsed();
    println!("indexed {} paths in {:.0} ms, {:.0} MB", ix.len(), build.as_secs_f64() * 1e3, ix.memory() as f64 / 1e6);

    let queries = ["report", "tax 2", "iliad", "photo crete", "readme.md", "zz", "nicosia-budget", "q"];
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("\nfile index alone, per keystroke from the 2nd letter ({threads} threads), ms:");
    println!("{:<16} {:>8} {:>8} {:>8}", "query", "median", "p99", "max");
    let mut all = Vec::new();
    for q in queries {
        let mut times = Vec::new();
        for _ in 0..20 {
            // From two letters, as the engine searches files.
            for end in 2.min(q.len())..=q.len() {
                let typed = &q[..end];
                let t = Instant::now();
                let r = ix.search(typed, 12);
                times.push(t.elapsed().as_secs_f64() * 1e3);
                std::hint::black_box(r);
            }
        }
        let (m, p99, max) = (percentile(&mut times, 0.5), percentile(&mut times, 0.99), percentile(&mut times, 1.0));
        all.extend(times);
        println!("{q:<16} {m:>8.2} {p99:>8.2} {max:>8.2}");
    }
    println!(
        "{:<16} {:>8.2} {:>8.2} {:>8.2}",
        "all",
        percentile(&mut all, 0.5),
        percentile(&mut all, 0.99),
        percentile(&mut all, 1.0)
    );

    // The whole engine: 150 apps, Settings, agora, answers and the files.
    let apps = (0..150)
        .map(|i| {
            let name = format!("{} {}", WORDS[i % WORDS.len()], WORDS[(i * 7) % WORDS.len()]);
            parse_entry(
                &format!("app{i}"),
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=app{i}\nComment=An app\n"),
            )
            .unwrap()
        })
        .collect();
    let dir = std::env::temp_dir().join("iliada-search-bench");
    let engine = Engine::new(ix, apps, Vec::new(), Usage::load(&dir.join("u.json")), None);
    println!("\nwhole engine (apps, Settings, answers, files), per keystroke, ms:");
    println!("{:<16} {:>8} {:>8} {:>8}", "letters typed", "median", "p99", "max");
    let mut by_len: Vec<Vec<f64>> = vec![Vec::new(); 5];
    for q in queries.iter().chain(&["2+2*3", "5 km in mi"]) {
        for _ in 0..20 {
            for end in 1..=q.len() {
                let t = Instant::now();
                std::hint::black_box(engine.search(&q[..end], true, 12));
                by_len[end.min(4)].push(t.elapsed().as_secs_f64() * 1e3);
            }
        }
    }
    let mut all = Vec::new();
    for (len, times) in by_len.iter_mut().enumerate().skip(1) {
        let label = if len == 4 { "4 or more".to_owned() } else { len.to_string() };
        println!(
            "{label:<16} {:>8.2} {:>8.2} {:>8.2}",
            percentile(times, 0.5),
            percentile(times, 0.99),
            percentile(times, 1.0)
        );
        all.extend(times.iter().copied());
    }
    println!(
        "{:<16} {:>8.2} {:>8.2} {:>8.2}",
        "all",
        percentile(&mut all, 0.5),
        percentile(&mut all, 0.99),
        percentile(&mut all, 1.0)
    );
}
