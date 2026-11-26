//! Homer mode in the Terminal (`iliada ask --here`, `homer`, or Ctrl+O at
//! a shell prompt): Homer working in the folder the terminal is in.
//!
//! Commands Homer runs start in that folder, and his answers are written
//! in the terminal as they come. Actions follow the same rules as anywhere
//! else (Ask mode asks first, commands need full access). The conversation
//! is kept under Terminal, with its folder, in Homer's history; started
//! again in the same folder soon after, it continues.
//!
//! Ctrl+O at Homer's prompt (or Ctrl+D, or `exit`) goes back to the shell:
//! while Homer mode runs, the terminal ends a line at Ctrl+O as well as at
//! Enter (termios VEOL), so the key works without a line editor.

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use iliada_assistant::Assistant;
use iliada_assistant::history::{Conversation, ConversationStore, Line, LineKind, Source, short_path};
use iliada_system::client;
use iliada_system::protocol::AssistantMode;

/// Ctrl+O.
const CTRL_O: u8 = 0x0f;

/// A folder's conversation is continued if it was this recent (seconds).
const RESUME: u64 = 6 * 60 * 60;

/// The terminal as it was, put back on the way out.
static SAVED: std::sync::OnceLock<rustix::termios::Termios> = std::sync::OnceLock::new();

/// Ctrl+O ends a line too, until [`restore_terminal`].
fn ctrl_o_ends_lines() {
    use rustix::termios::{OptionalActions, SpecialCodeIndex, tcgetattr, tcsetattr};
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return;
    }
    let Ok(t) = tcgetattr(&stdin) else { return };
    let mut ours = t.clone();
    ours.special_codes[SpecialCodeIndex::VEOL] = CTRL_O;
    if SAVED.set(t).is_ok() {
        let _ = tcsetattr(&stdin, OptionalActions::Now, &ours);
    }
}

fn restore_terminal() {
    if let Some(t) = SAVED.get() {
        let _ = rustix::termios::tcsetattr(std::io::stdin(), rustix::termios::OptionalActions::Now, t);
    }
}

/// Puts the terminal back however Homer mode ends.
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// What was typed at a prompt.
#[derive(Debug, PartialEq)]
enum Typed {
    Line(String),
    /// Ctrl+O or Ctrl+D: back to the shell.
    Leave,
}

fn typed(read: usize, line: &str) -> Typed {
    if read == 0 || line.as_bytes().contains(&CTRL_O) {
        return Typed::Leave;
    }
    Typed::Line(line.trim().to_owned())
}

fn read_line() -> Typed {
    use std::io::Read;
    // Not `read_line`: a line ended by Ctrl+O has no newline, and it would
    // wait for one. Each read of a terminal is one line.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut stdin = std::io::stdin();
    while let Ok(n @ 1..) = stdin.read(&mut chunk) {
        buf.extend_from_slice(&chunk[..n]);
        if buf.ends_with(b"\n") || buf.contains(&CTRL_O) {
            break;
        }
    }
    let line = String::from_utf8_lossy(&buf);
    if !buf.is_empty() && !line.ends_with('\n') {
        // Ended by Ctrl+O: the terminal did not move to the next line.
        eprintln!();
    }
    typed(buf.len(), &line)
}

/// Colours, when writing to a terminal.
struct Style {
    on: bool,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.on { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_owned() }
    }
    /// Homer's gold.
    fn gold(&self, text: &str) -> String {
        self.paint("1;38;5;178", text)
    }
    fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }
}

/// The terminal as Homer's frontend.
struct Term {
    style: Style,
    /// Notes for the transcript, as the app keeps them.
    notes: Mutex<Vec<Line>>,
    /// Something of the current answer was written.
    writing: AtomicBool,
    /// The last answer was written out as it came.
    wrote: AtomicBool,
    /// "Thinking…" is showing.
    thinking: AtomicBool,
}

impl Term {
    fn clear_thinking(&self) {
        if self.thinking.swap(false, Ordering::SeqCst) {
            eprint!("\r\x1b[K");
        }
    }

    fn note(&self, text: &str) {
        let mut notes = self.notes.lock().expect("notes");
        // "✓ Install Firefox" replaces "Install Firefox…".
        if let Some(last) = notes.last()
            && let Some(action) = last.text.strip_suffix('…')
            && (text.strip_prefix("✓ ") == Some(action)
                || text.strip_prefix("✗ ").is_some_and(|rest| rest.starts_with(action)))
        {
            notes.pop();
        }
        notes.push(Line::new(LineKind::Note, text));
    }
}

#[async_trait::async_trait]
impl iliada_assistant::Frontend for Term {
    async fn approve(&self, action: &str) -> bool {
        self.clear_thinking();
        eprint!("  {} {action}. Allow? [y/N] ", self.style.gold("Homer wants to:"));
        let _ = std::io::stderr().flush();
        let yes = matches!(read_line(), Typed::Line(l) if matches!(l.as_str(), "y" | "Y" | "yes" | "Yes"));
        if !yes {
            self.note(&format!("You didn't allow: {action}"));
        }
        yes
    }
    fn progress(&self, text: &str) {
        self.clear_thinking();
        if !text.ends_with('…') || !self.style.on {
            eprintln!("  {}", self.style.dim(text));
        }
        self.note(text);
    }
    fn text(&self, piece: &str) {
        self.clear_thinking();
        let first = !self.writing.swap(true, Ordering::SeqCst);
        print!("{}", if first { piece.trim_start() } else { piece });
        let _ = std::io::stdout().flush();
    }
    fn text_end(&self) {
        let wrote = self.writing.swap(false, Ordering::SeqCst);
        if wrote {
            println!();
        }
        self.wrote.store(wrote, Ordering::SeqCst);
    }
}

fn ago(t: u64) -> String {
    let s = iliada_assistant::history::now().saturating_sub(t);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..7200 => "an hour ago".into(),
        _ => format!("{} hours ago", s / 3600),
    }
}

fn fresh(model: &str, cwd: &str) -> Conversation {
    Conversation::from_source(model, Source::Terminal { cwd: cwd.to_owned() })
}

/// Homer mode: `first` is asked first (text typed before Ctrl+O); `new`
/// starts a fresh conversation instead of continuing this folder's.
pub async fn run(first: String, new: bool) -> Result<()> {
    let settings = client::status().await?.assistant;
    let cwd = std::env::current_dir()?;
    let here = cwd.to_string_lossy().into_owned();
    let store = ConversationStore::open();
    let style = Style { on: std::io::stderr().is_terminal() };
    let mut a = start(&settings, &cwd)?;
    let mut c =
        if new { None } else { store.latest_in(&here, RESUME) }.unwrap_or_else(|| fresh(&settings.model, &here));
    if !c.messages.is_empty() {
        a.restore(c.messages.clone(), &c.model);
    }
    a.set_conversation(&c.id);

    let mode = match a.mode() {
        AssistantMode::Ask => "asks first",
        AssistantMode::Auto => "acts on its own",
    };
    eprintln!(
        "{} in {} {}",
        style.gold("Homer"),
        short_path(&here),
        style.dim(&format!("({mode} · Ctrl+O or Ctrl+D: back to the shell · /new: start over)"))
    );
    if !c.lines.is_empty() {
        eprintln!("{}", style.dim(&format!("Continuing \"{}\" from {}.", c.title, ago(c.updated))));
    }
    if !a.full_access() {
        eprintln!("{}", style.dim("Homer can't run commands here until you give him full access (Settings → Homer)."));
    }
    if !store.keeps() {
        eprintln!("{}", style.dim("History is off: this conversation won't be kept."));
    }

    ctrl_o_ends_lines();
    let _restore = Restore;
    tokio::spawn(async {
        // Ctrl+C: back to the shell, with the terminal as it was.
        if tokio::signal::ctrl_c().await.is_ok() {
            restore_terminal();
            eprintln!();
            std::process::exit(130);
        }
    });

    let prompt = format!("{} {} ", style.gold("Homer"), style.gold(&format!("{} ❯", short_path(&here))));
    let mut next = Some(first).filter(|f| !f.trim().is_empty());
    loop {
        let line = match next.take() {
            Some(l) => {
                eprintln!("{prompt}{l}");
                l
            }
            None => {
                eprint!("{prompt}");
                let _ = std::io::stderr().flush();
                match read_line() {
                    Typed::Line(l) => l,
                    Typed::Leave => break,
                }
            }
        };
        match line.as_str() {
            "" => continue,
            "exit" | "/exit" | "quit" | "/quit" => break,
            "/new" => {
                c = fresh(&settings.model, &here);
                a = start(&settings, &cwd)?;
                a.set_conversation(&c.id);
                eprintln!("{}", style.dim("A fresh conversation."));
                continue;
            }
            _ => {}
        }
        let ui = Term {
            style: Style { on: style.on },
            notes: Mutex::new(Vec::new()),
            writing: AtomicBool::new(false),
            wrote: AtomicBool::new(false),
            thinking: AtomicBool::new(false),
        };
        c.lines.push(Line::new(LineKind::You, line.clone()));
        let _ = store.save(&mut c);
        if style.on {
            eprint!("{}", style.dim("  Thinking…"));
            ui.thinking.store(true, Ordering::SeqCst);
        }
        let spent_before = a.spent();
        let r = a.send(&line, &ui).await;
        ui.clear_thinking();
        c.lines.append(&mut ui.notes.lock().expect("notes"));
        match r {
            Ok(answer) => {
                if !ui.wrote.load(Ordering::SeqCst) && !answer.is_empty() {
                    println!("{answer}");
                }
                if !answer.is_empty() {
                    c.lines.push(Line::new(LineKind::Assistant, answer));
                }
            }
            Err(e) => {
                eprintln!("{}", style.paint("31", &format!("{e:#}")));
                c.lines.push(Line::new(LineKind::Error, format!("{e:#}")));
            }
        }
        println!();
        c.messages = a.messages().to_vec();
        c.spent += a.spent() - spent_before;
        c.model = settings.model.clone();
        if let Err(e) = store.save(&mut c) {
            eprintln!("{}", style.dim(&format!("(This conversation could not be saved: {e:#})")));
        }
    }
    Ok(())
}

fn start(settings: &iliada_system::protocol::AssistantSettings, cwd: &Path) -> Result<Assistant> {
    let mut a = Assistant::new(settings)?;
    a.set_workdir(cwd);
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_o_and_ctrl_d_leave() {
        assert_eq!(typed(0, ""), Typed::Leave);
        assert_eq!(typed(4, "abc\x0f"), Typed::Leave);
        assert_eq!(typed(1, "\x0f"), Typed::Leave);
        assert_eq!(typed(8, "  ls -la\n"), Typed::Line("ls -la".into()));
        assert_eq!(typed(1, "\n"), Typed::Line(String::new()));
    }

    #[test]
    fn notes_read_like_the_app() {
        let t = Term {
            style: Style { on: false },
            notes: Mutex::new(Vec::new()),
            writing: AtomicBool::new(false),
            wrote: AtomicBool::new(false),
            thinking: AtomicBool::new(false),
        };
        t.note("Run the command: make…");
        t.note("✓ Run the command: make");
        t.note("Install VLC…");
        t.note("✗ Install VLC: offline");
        let notes: Vec<_> = t.notes.lock().unwrap().iter().map(|l| l.text.clone()).collect();
        assert_eq!(notes, ["✓ Run the command: make", "✗ Install VLC: offline"]);
    }
}
