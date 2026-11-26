//! Homer's hands: how the assistant reads and uses other apps.
//!
//! One user service (`iliada-control`) is the only way Homer touches another
//! app. It has four ways in, from the most exact to the most general:
//!
//! 1. **Accessibility** (`a11y.rs`): the app's own description of its
//!    window (AT-SPI): buttons, fields, menus, with ids Homer acts on.
//! 2. **Screen and input** (`screen.rs`, `input.rs`): a screenshot the model
//!    looks at, and a virtual mouse and keyboard, for apps that describe
//!    nothing.
//! 3. **Chrome** (`chrome.rs`): Homer's own Chrome window over the DevTools
//!    protocol on a private pipe: tabs, pages, forms, Google Docs.
//! 4. **App intents** live in the assistant (they are commands an app
//!    declares, not control of its window).
//!
//! Every request passes the same gate (`server.rs`): the person's per-app
//! answers and the global switch (`iliada_bundle::control`), a per-
//! conversation yes in Ask mode, a yes before anything that sends, submits,
//! deletes, buys or posts (`risk.rs`), the Stop key, a banner on screen while
//! Homer is in control, and a log of every action. What it reads from apps
//! goes back to the model fenced as untrusted (`untrusted.rs`).

pub mod a11y;
pub mod chrome;
pub mod input;
pub mod note;
pub mod risk;
pub mod screen;
pub mod server;
pub mod untrusted;

/// `$XDG_RUNTIME_DIR/iliada-control.sock`.
pub fn socket_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| std::path::PathBuf::from(d).join("iliada-control.sock"))
}
