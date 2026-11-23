//! Every file and folder name in the home folder, in memory.
//!
//! Names are kept in one block, lower-cased and separated by newlines, so a
//! search is a single substring scan over a few megabytes (memchr's SIMD
//! `memmem`), split across the processor's cores for large homes. Each
//! entry knows its folder, so paths are rebuilt only for the few results
//! shown, and words after the first may match a folder on the way ("tax
//! 2025" finds `~/Taxes/2025/return.pdf`).
//!
//! Hidden files and folders (`.config`, `.cache`…), `node_modules` and
//! whatever a Git repository ignores are left out, as Spotlight leaves out
//! system files. Changes arrive through inotify ([`FileIndex::insert`],
//! [`FileIndex::remove`]); removed entries are marked, and the index is
//! rebuilt when many are.

use std::collections::{BinaryHeap, HashMap};
use std::path::{Path, PathBuf};

use memchr::memmem;

const ROOT: u32 = u32::MAX;
const DIR: u8 = 1;
const GONE: u8 = 2;

/// Folder names never indexed (besides hidden ones).
const SKIPPED: [&str; 4] = ["node_modules", "__pycache__", "lost+found", "$RECYCLE.BIN"];

/// Whether a name is left out of the index.
pub fn skipped(name: &str) -> bool {
    name.starts_with('.') || SKIPPED.contains(&name) || name.contains('\n')
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    /// Start of the lower-cased name in `lower` (ascending).
    lower: u32,
    /// Start and length of the name as written, in `names`.
    name: u32,
    name_len: u32,
    parent: u32,
    depth: u8,
    flags: u8,
}

/// See the module docs.
#[derive(Debug)]
pub struct FileIndex {
    root: PathBuf,
    lower: Vec<u8>,
    names: Vec<u8>,
    entries: Vec<Entry>,
    dirs: HashMap<PathBuf, u32>,
    children: HashMap<(u32, u64), u32>,
    gone: usize,
}

fn fnv(s: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// One result: the entry and how well it matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub score: i64,
    pub entry: u32,
}

impl FileIndex {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FileIndex {
            root: root.into(),
            lower: Vec::new(),
            names: Vec::new(),
            entries: Vec::new(),
            dirs: HashMap::new(),
            children: HashMap::new(),
            gone: 0,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Entries in the index (removed ones included until the next rebuild).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries removed since the index was built.
    pub fn removed(&self) -> usize {
        self.gone
    }

    /// Bytes of memory the index holds, roughly.
    pub fn memory(&self) -> usize {
        self.lower.capacity()
            + self.names.capacity()
            + self.entries.capacity() * std::mem::size_of::<Entry>()
            + self.children.capacity() * 24
            + self.dirs.keys().map(|k| k.as_os_str().len() + 40).sum::<usize>()
    }

    /// Add a file or folder (and any folders above it not yet known).
    /// Returns its entry; `None` when it is outside the root or left out.
    pub fn insert(&mut self, path: &Path, is_dir: bool) -> Option<u32> {
        let rel = path.strip_prefix(&self.root).ok()?;
        if rel.as_os_str().is_empty() {
            return None;
        }
        let name = path.file_name()?.to_string_lossy();
        if skipped(&name) {
            return None;
        }
        let parent_path = path.parent()?;
        let parent = if parent_path == self.root {
            ROOT
        } else {
            match self.dirs.get(parent_path) {
                Some(&i) => i,
                None => self.insert(parent_path, true)?,
            }
        };
        let key = (parent, fnv(name.as_bytes()));
        if let Some(&i) = self.children.get(&key) {
            return Some(i);
        }
        let depth = if parent == ROOT { 0 } else { self.entries[parent as usize].depth.saturating_add(1) };
        let i = u32::try_from(self.entries.len()).ok()?;
        let entry = Entry {
            lower: u32::try_from(self.lower.len()).ok()?,
            name: u32::try_from(self.names.len()).ok()?,
            name_len: name.len() as u32,
            parent,
            depth,
            flags: if is_dir { DIR } else { 0 },
        };
        self.lower.extend(name.to_lowercase().bytes());
        self.lower.push(b'\n');
        self.names.extend_from_slice(name.as_bytes());
        self.entries.push(entry);
        self.children.insert(key, i);
        if is_dir {
            self.dirs.insert(path.to_owned(), i);
        }
        Some(i)
    }

    /// Mark a file or folder as removed (a folder's contents with it).
    pub fn remove(&mut self, path: &Path) {
        let (Some(parent_path), Some(name)) = (path.parent(), path.file_name()) else { return };
        let parent = if parent_path == self.root {
            ROOT
        } else {
            match self.dirs.get(parent_path) {
                Some(&i) => i,
                None => return,
            }
        };
        let Some(i) = self.children.remove(&(parent, fnv(name.to_string_lossy().as_bytes()))) else { return };
        let e = &mut self.entries[i as usize];
        if e.flags & GONE == 0 {
            e.flags |= GONE;
            self.gone += 1;
        }
        if e.flags & DIR != 0 {
            self.dirs.retain(|p, _| !p.starts_with(path));
        }
    }

    fn lower_of(&self, i: u32) -> &[u8] {
        let start = self.entries[i as usize].lower as usize;
        let end = self.entries.get(i as usize + 1).map_or(self.lower.len(), |e| e.lower as usize) - 1;
        &self.lower[start..end]
    }

    /// The name as written.
    pub fn name(&self, i: u32) -> &str {
        let e = &self.entries[i as usize];
        std::str::from_utf8(&self.names[e.name as usize..(e.name + e.name_len) as usize]).unwrap_or("")
    }

    pub fn is_dir(&self, i: u32) -> bool {
        self.entries[i as usize].flags & DIR != 0
    }

    /// Present: neither it nor a folder above it was removed.
    pub fn alive(&self, mut i: u32) -> bool {
        while i != ROOT {
            let e = &self.entries[i as usize];
            if e.flags & GONE != 0 {
                return false;
            }
            i = e.parent;
        }
        true
    }

    /// The entry's full path.
    pub fn path(&self, i: u32) -> PathBuf {
        let mut parts = Vec::new();
        let mut j = i;
        while j != ROOT {
            parts.push(self.name(j));
            j = self.entries[j as usize].parent;
        }
        let mut p = self.root.clone();
        for part in parts.iter().rev() {
            p.push(part);
        }
        p
    }

    /// The best `limit` matches for `query` (words, any case). Every word
    /// must appear in the name, or (after the first) in a folder above it.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Found> {
        let q = query.to_lowercase();
        let mut words: Vec<&str> = q.split_whitespace().collect();
        if words.is_empty() || limit == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        // The longest word is the rarest, usually: it drives the scan.
        words.sort_by_key(|w| std::cmp::Reverse(w.len()));
        let (first, rest) = (words[0], &words[1..]);
        let whole = q.trim();
        let threads = if self.entries.len() > 150_000 {
            std::thread::available_parallelism().map_or(1, |n| n.get()).min(16)
        } else {
            1
        };
        let keep = limit * 3;
        let chunk = self.entries.len().div_ceil(threads);
        let mut best: Vec<Found> = if threads == 1 {
            self.scan(0, self.entries.len(), first, rest, whole, keep)
        } else {
            std::thread::scope(|s| {
                let parts: Vec<_> = (0..threads)
                    .map(|t| {
                        let (from, to) = (t * chunk, ((t + 1) * chunk).min(self.entries.len()));
                        s.spawn(move || self.scan(from, to, first, rest, whole, keep))
                    })
                    .collect();
                parts.into_iter().flat_map(|p| p.join().unwrap_or_default()).collect()
            })
        };
        best.sort_unstable_by(|a, b| b.cmp(a));
        best.retain(|f| self.alive(f.entry));
        best.truncate(limit);
        best
    }

    /// Matches among entries `from..to`: the best `keep`.
    fn scan(&self, from: usize, to: usize, first: &str, rest: &[&str], whole: &str, keep: usize) -> Vec<Found> {
        if from >= to {
            return Vec::new();
        }
        let start = self.entries[from].lower as usize;
        let end = self.entries.get(to).map_or(self.lower.len(), |e| e.lower as usize);
        let hay = &self.lower[start..end];
        let mut heap: BinaryHeap<std::cmp::Reverse<Found>> = BinaryHeap::with_capacity(keep + 1);
        let mut offer = |f: Found| {
            if heap.len() < keep {
                heap.push(std::cmp::Reverse(f));
            } else if heap.peek().is_some_and(|m| m.0 < f) {
                heap.pop();
                heap.push(std::cmp::Reverse(f));
            }
        };
        let finder = memmem::Finder::new(first.as_bytes());
        let mut i = from;
        let mut last = usize::MAX;
        for pos in finder.find_iter(hay) {
            let pos = (pos + start) as u32;
            // The entry this position is in: the cursor only moves forward.
            if i + 1 < to && self.entries[i + 1].lower <= pos {
                let ahead = &self.entries[i + 1..to];
                i += ahead.partition_point(|e| e.lower <= pos);
            }
            if i == last {
                continue;
            }
            last = i;
            let e = &self.entries[i];
            if e.flags & GONE != 0 {
                continue;
            }
            let name = self.lower_of(i as u32);
            let mut at = pos as usize - e.lower as usize;
            let word_start = |at: usize| at == 0 || !name[at - 1].is_ascii_alphanumeric();
            if !word_start(at) {
                // A later place may start a word ("store-report"): the best.
                match memmem::find_iter(&name[at + 1..], first.as_bytes()).map(|k| k + at + 1).find(|&k| word_start(k))
                {
                    Some(k) => at = k,
                    // One or two letters match only at the start of a word
                    // ("re" finds "report", not "store"): the rest is noise.
                    None if first.len() <= 2 => continue,
                    None => {}
                }
            }
            let Some(score) = self.score(i as u32, name, at, rest, whole) else { continue };
            offer(Found { score, entry: i as u32 });
        }
        heap.into_iter().map(|r| r.0).collect()
    }

    fn score(&self, i: u32, name: &[u8], at: usize, rest: &[&str], whole: &str) -> Option<i64> {
        let e = &self.entries[i as usize];
        let mut s: i64 = 0;
        let stem = name.iter().rposition(|&b| b == b'.').filter(|&d| d > 0).map_or(name, |d| &name[..d]);
        if name == whole.as_bytes() || stem == whole.as_bytes() {
            s += 600;
        }
        s += if at == 0 {
            300
        } else if !name[at - 1].is_ascii_alphanumeric() {
            200
        } else if name[at - 1].is_ascii_lowercase() && self.upper_at(i, at) {
            150 // camelCase
        } else {
            40
        };
        for w in rest {
            if memmem::find(name, w.as_bytes()).is_some() {
                s += 60;
                continue;
            }
            // In a folder above it?
            let mut j = e.parent;
            let mut found = false;
            while j != ROOT {
                if memmem::find(self.lower_of(j), w.as_bytes()).is_some() {
                    found = true;
                    break;
                }
                j = self.entries[j as usize].parent;
            }
            if !found {
                return None;
            }
            s += 15;
        }
        s -= (name.len() as i64).min(80);
        s -= 12 * i64::from(e.depth);
        if e.flags & DIR != 0 {
            s += 30;
        }
        Some(s)
    }

    /// Whether the name as written has a capital letter at `at` (when the
    /// lower-cased name has the same length).
    fn upper_at(&self, i: u32, at: usize) -> bool {
        let e = &self.entries[i as usize];
        let n = &self.names[e.name as usize..(e.name + e.name_len) as usize];
        n.get(at).is_some_and(|b| b.is_ascii_uppercase())
    }

    /// Index `root`'s tree (in parallel): the whole home folder.
    pub fn build(root: impl Into<PathBuf>) -> Self {
        let mut index = FileIndex::new(root);
        let root = index.root.clone();
        index.add_tree(&root);
        index.lower.shrink_to_fit();
        index.names.shrink_to_fit();
        index.entries.shrink_to_fit();
        index
    }

    /// Add a folder and everything in it (a folder moved into the home).
    pub fn add_tree(&mut self, dir: &Path) {
        let (tx, rx) = std::sync::mpsc::sync_channel::<(PathBuf, bool)>(4096);
        let walker = ignore::WalkBuilder::new(dir)
            .hidden(true)
            .parents(false)
            .git_global(false)
            .git_exclude(false)
            .ignore(false)
            .follow_links(false)
            .filter_entry(|e| e.depth() == 0 || !skipped(&e.file_name().to_string_lossy()))
            .threads(std::thread::available_parallelism().map_or(2, |n| n.get()).min(8))
            .build_parallel();
        std::thread::scope(|s| {
            s.spawn(move || {
                walker.run(|| {
                    let tx = tx.clone();
                    Box::new(move |r| {
                        if let Ok(e) = r
                            && e.depth() > 0
                        {
                            let dir = e.file_type().is_some_and(|t| t.is_dir());
                            if tx.send((e.into_path(), dir)).is_err() {
                                return ignore::WalkState::Quit;
                            }
                        }
                        ignore::WalkState::Continue
                    })
                });
            });
            if dir != self.root {
                self.insert(dir, true);
            }
            for (path, is_dir) in rx {
                self.insert(&path, is_dir);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(paths: &[&str]) -> FileIndex {
        let mut ix = FileIndex::new("/home/a");
        for p in paths {
            let dir = p.ends_with('/');
            ix.insert(Path::new(&format!("/home/a/{}", p.trim_end_matches('/'))), dir);
        }
        ix
    }

    fn names(ix: &FileIndex, q: &str) -> Vec<String> {
        ix.search(q, 10).into_iter().map(|f| ix.path(f.entry).display().to_string()).collect()
    }

    #[test]
    fn finds_by_substring_and_ranks_prefixes_first() {
        let ix = index(&["Documents/", "Documents/report.pdf", "Documents/old-report.txt", "Music/reporters.mp3"]);
        let r = names(&ix, "report");
        assert_eq!(r[0], "/home/a/Documents/report.pdf");
        assert_eq!(r.len(), 3);
        assert!(names(&ix, "REPORT.PDF")[0].ends_with("report.pdf"));
        assert!(names(&ix, "zzz").is_empty());
        // One or two letters: word starts only, and a later word start counts.
        let ix = index(&["store-report.txt", "restore.txt", "rest.txt"]);
        assert_eq!(names(&ix, "re").len(), 3);
        let ix = index(&["store.txt", "store-report.txt"]);
        assert_eq!(names(&ix, "re"), vec!["/home/a/store-report.txt"]);
    }

    #[test]
    fn later_words_may_match_folders() {
        let ix = index(&["Taxes/2025/return.pdf", "Taxes/2024/return.pdf", "return.txt"]);
        assert_eq!(names(&ix, "return 2025"), vec!["/home/a/Taxes/2025/return.pdf"]);
    }

    #[test]
    fn parents_are_created_and_hidden_left_out() {
        let ix = index(&["a/b/c.txt", ".secret/key", "x/node_modules/y.js"]);
        assert!(names(&ix, "b").iter().any(|p| p == "/home/a/a/b"));
        assert!(names(&ix, "key").is_empty());
        assert!(names(&ix, "y.js").is_empty());
    }

    #[test]
    fn removing_a_folder_hides_what_is_in_it() {
        let mut ix = index(&["Projects/", "Projects/notes.md", "notes-2.md"]);
        ix.remove(Path::new("/home/a/Projects"));
        assert_eq!(names(&ix, "notes"), vec!["/home/a/notes-2.md"]);
        // Back again.
        ix.insert(Path::new("/home/a/Projects/notes.md"), false);
        assert_eq!(names(&ix, "notes").len(), 2);
    }

    #[test]
    fn unicode_names() {
        let ix = index(&["Ιλιάδα.txt", "ΟΔΥΣΣΕΙΑ.md"]);
        assert_eq!(names(&ix, "ιλιάδα").len(), 1);
        assert_eq!(names(&ix, "οδυσσεια").len(), 1);
    }

    #[test]
    fn builds_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Photos/Crete")).unwrap();
        std::fs::write(dir.path().join("Photos/Crete/beach.jpg"), b"").unwrap();
        std::fs::create_dir_all(dir.path().join(".cache")).unwrap();
        std::fs::write(dir.path().join(".cache/beach-thumb.jpg"), b"").unwrap();
        let ix = FileIndex::build(dir.path());
        let hits = ix.search("beach", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(ix.path(hits[0].entry), dir.path().join("Photos/Crete/beach.jpg"));
        assert_eq!(ix.len(), 3);
    }
}
