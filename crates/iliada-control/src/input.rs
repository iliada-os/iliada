//! Layer 2: the virtual mouse.
//!
//! The desktop portal's Remote Desktop session (COSMIC asks the person once;
//! the answer is remembered with a restore token) hands Homer an EI socket
//! (libei protocol), on which cosmic-comp gives it an absolute pointer, a
//! button and a scroll wheel. cosmic-comp takes EI sockets only from the
//! portal, so no sandboxed app can get one, and the portal is out of
//! sandboxed apps' reach (no session bus in the sandbox).
//!
//! The EI connection lives on a thread of its own (its event stream is not
//! `Send`); the service talks to it over a channel.

use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result};
use ashpd::desktop::PersistMode;
use ashpd::desktop::remote_desktop::{DeviceType, RemoteDesktop, SelectDevicesOptions};
use futures_util::StreamExt;
use reis::ei;
use reis::event::{DeviceCapability, EiEvent};
use tokio::sync::{mpsc, oneshot};

/// Linux input codes.
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

type Region = (u32, u32, u32, u32);

#[derive(Debug)]
enum Cmd {
    Click { x: f32, y: f32, button: u32, count: u32, done: oneshot::Sender<Result<()>> },
    Scroll { x: f32, y: f32, dx: i32, dy: i32, done: oneshot::Sender<Result<()>> },
    Release,
}

/// Homer's pointer: a handle to the EI thread.
#[derive(Debug)]
pub struct Pointer {
    tx: mpsc::UnboundedSender<Cmd>,
    regions: Vec<Region>,
}

fn token_path() -> Option<std::path::PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(x) => std::path::PathBuf::from(x),
        None => std::path::PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("iliada/homer-remote-desktop-token"))
}

fn now_us() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

/// The portal session (COSMIC asks the person the first time) and the EI
/// socket it hands out.
async fn portal() -> Result<(RemoteDesktop, ashpd::desktop::Session<RemoteDesktop>, UnixStream)> {
    let portal = RemoteDesktop::new().await.context("the desktop portal is not running")?;
    let session = portal.create_session(Default::default()).await?;
    let saved = token_path().and_then(|p| std::fs::read_to_string(p).ok());
    portal
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::POINTER)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(saved.as_deref().map(str::trim)),
        )
        .await?;
    let started = portal
        .start(&session, None, Default::default())
        .await?
        .response()
        .context("the person did not let Homer use the mouse (or the desktop cannot share it)")?;
    if let (Some(t), Some(p)) = (started.restore_token(), token_path()) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, t);
    }
    let fd = portal.connect_to_eis(&session, Default::default()).await.context("no input socket from the portal")?;
    Ok((portal, session, UnixStream::from(fd)))
}

struct Emu {
    ctx: ei::Context,
    device: Option<reis::event::Device>,
    serial: u32,
    sequence: u32,
    emulating: bool,
}

impl Emu {
    fn send(&mut self, f: impl FnOnce(&reis::event::Device, u32)) -> Result<()> {
        let d = self.device.clone().context("the pointer went away")?;
        if !self.emulating {
            self.sequence += 1;
            d.device().start_emulating(self.serial, self.sequence);
            self.emulating = true;
        }
        f(&d, self.serial);
        self.ctx.flush().map_err(|e| anyhow::anyhow!("sending input: {e}"))
    }

    fn motion(&mut self, x: f32, y: f32) -> Result<()> {
        self.send(|d, serial| {
            if let Some(p) = d.interface::<ei::PointerAbsolute>() {
                p.motion_absolute(x, y);
                d.device().frame(serial, now_us());
            }
        })
    }

    fn button(&mut self, button: u32, state: ei::button::ButtonState) -> Result<()> {
        self.send(|d, serial| {
            if let Some(b) = d.interface::<ei::Button>() {
                b.button(button, state);
                d.device().frame(serial, now_us());
            }
        })
    }

    fn release(&mut self) {
        if let Some(d) = &self.device
            && self.emulating
        {
            d.device().stop_emulating(self.serial);
            self.emulating = false;
            let _ = self.ctx.flush();
        }
    }
}

async fn run_cmd(emu: &mut Emu, cmd: Cmd) {
    match cmd {
        Cmd::Click { x, y, button, count, done } => {
            let r = async {
                emu.motion(x, y)?;
                tokio::time::sleep(Duration::from_millis(30)).await;
                for _ in 0..count.clamp(1, 3) {
                    emu.button(button, ei::button::ButtonState::Press)?;
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    emu.button(button, ei::button::ButtonState::Released)?;
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
                Ok(())
            }
            .await;
            let _ = done.send(r);
        }
        Cmd::Scroll { x, y, dx, dy, done } => {
            let r = emu.motion(x, y).and_then(|()| {
                emu.send(|d, serial| {
                    if let Some(s) = d.interface::<ei::Scroll>() {
                        s.scroll_discrete(dx * 120, dy * 120);
                        d.device().frame(serial, now_us());
                    }
                })
            });
            let _ = done.send(r);
        }
        Cmd::Release => emu.release(),
    }
}

impl Pointer {
    /// Starts the Remote Desktop session and waits for the pointer.
    pub async fn connect() -> Result<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Cmd>();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<Vec<Region>>>();
        std::thread::Builder::new().name("homer-pointer".into()).spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.into()));
                    return;
                }
            };
            rt.block_on(async move {
                let setup = async {
                    let (portal, session, sock) = portal().await?;
                    let ctx = ei::Context::new(sock)?;
                    let (_conn, events) = ctx
                        .handshake_tokio("Homer (Iliada)", ei::handshake::ContextType::Sender)
                        .await
                        .context("EI handshake")?;
                    anyhow::Ok((portal, session, ctx, events))
                };
                let (_portal, _session, ctx, mut events) = match setup.await {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let mut emu = Emu { ctx, device: None, serial: 0, sequence: 0, emulating: false };
                let mut ready = Some(ready_tx);
                let deadline = tokio::time::sleep(Duration::from_secs(5));
                tokio::pin!(deadline);
                loop {
                    tokio::select! {
                        ev = events.next() => {
                            let Some(Ok(ev)) = ev else { break };
                            match ev {
                                EiEvent::SeatAdded(a) => a.seat.bind_capabilities(
                                    DeviceCapability::PointerAbsolute | DeviceCapability::Button | DeviceCapability::Scroll,
                                ),
                                EiEvent::DeviceAdded(a) if a.device.interface::<ei::PointerAbsolute>().is_some() => {
                                    emu.device = Some(a.device);
                                }
                                EiEvent::DeviceResumed(r) => {
                                    emu.serial = r.serial;
                                    if emu.device.as_ref().is_some_and(|d| d.device() == r.device.device()) {
                                        emu.emulating = false;
                                        if let (Some(tx), Some(d)) = (ready.take(), &emu.device) {
                                            let regions = d.regions().iter().map(|r| (r.x, r.y, r.width, r.height)).collect();
                                            let _ = tx.send(Ok(regions));
                                        }
                                    }
                                }
                                EiEvent::DevicePaused(p) => emu.serial = p.serial,
                                EiEvent::DeviceRemoved(r) if emu.device.as_ref().is_some_and(|d| d.device() == r.device.device()) => {
                                    emu.device = None;
                                }
                                EiEvent::Disconnected(_) => break,
                                _ => {}
                            }
                        }
                        cmd = rx.recv() => {
                            let Some(cmd) = cmd else { break };
                            run_cmd(&mut emu, cmd).await;
                        }
                        _ = &mut deadline, if ready.is_some() => {
                            if let Some(tx) = ready.take() {
                                let _ = tx.send(Err(anyhow::anyhow!("the desktop did not give Homer a pointer")));
                            }
                            break;
                        }
                    }
                }
                emu.release();
            });
        })?;
        let regions = ready_rx.await.context("the pointer thread ended")??;
        Ok(Self { tx, regions })
    }

    pub fn alive(&self) -> bool {
        !self.tx.is_closed()
    }

    pub fn regions(&self) -> Vec<Region> {
        self.regions.clone()
    }

    pub async fn click(&mut self, x: f32, y: f32, button: u32, count: u32) -> Result<()> {
        let (done, rx) = oneshot::channel();
        self.tx.send(Cmd::Click { x, y, button, count, done }).map_err(|_| anyhow::anyhow!("the pointer went away"))?;
        rx.await.context("the pointer went away")?
    }

    /// Scrolls at a point by whole wheel clicks (positive: down / right).
    pub async fn scroll(&mut self, x: f32, y: f32, dx: i32, dy: i32) -> Result<()> {
        let (done, rx) = oneshot::channel();
        self.tx.send(Cmd::Scroll { x, y, dx, dy, done }).map_err(|_| anyhow::anyhow!("the pointer went away"))?;
        rx.await.context("the pointer went away")?
    }

    /// Lets go of the pointer (the person pressed Stop).
    pub fn release(&mut self) {
        let _ = self.tx.send(Cmd::Release);
    }
}

/// Is a point inside what the pointer covers?
pub fn inside(regions: &[Region], x: f64, y: f64) -> bool {
    regions.is_empty()
        || regions
            .iter()
            .any(|&(rx, ry, w, h)| x >= rx as f64 && y >= ry as f64 && x < (rx + w) as f64 && y < (ry + h) as f64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn points_outside_every_screen_are_refused() {
        let r = [(0, 0, 1920, 1080), (1920, 0, 1280, 1024)];
        assert!(super::inside(&r, 10.0, 10.0));
        assert!(super::inside(&r, 3000.0, 1000.0));
        assert!(!super::inside(&r, 3000.0, 1050.0));
        assert!(!super::inside(&r, -1.0, 0.0));
    }
}
