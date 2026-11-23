//! Settings → Account (and the Setup Assistant's sign-in step): the Iliada
//! account on this computer, for the person using it. No administrator
//! needed: it is theirs, kept in their home folder.

use cosmic::iced::task::Handle;
use cosmic::iced::{Alignment, Length, Task};
use cosmic::widget::{self, settings};
use cosmic::{Element, theme};
use iliada_account::{self as account, DeviceCode, Files, KeyStep, Manifest, Status};

use crate::i18n::ago;
use crate::{fl, open_url};

#[derive(Debug, Clone)]
pub enum Msg {
    SignIn,
    Code(Result<DeviceCode, String>),
    OpenPage,
    CancelSignIn,
    SignedIn(Result<(String, KeyStep), String>),
    /// The person saved the recovery key shown to them.
    KeySaved,
    ShowKey,
    HideKey,
    CopyKey,
    RecoveryInput(String),
    UseRecoveryKey,
    KeyUsed(Result<(), String>),
    /// Get the key from a computer already signed in.
    AskOtherComputer,
    Asked(Result<account::KeyRequest, String>),
    CancelKeyWait,
    /// Look for new computers waiting for the key (while the page is open).
    PollWaiting,
    WaitingList(Result<Vec<account::Waiting>, String>),
    GiveKey(String),
    RefuseKey(String),
    Answered(Result<String, String>),
    Refused,
    BackUp,
    BackedUp(Result<account::Backup, String>),
    Latest(Result<Option<Manifest>, String>),
    /// Ask first (the page's owner shows a dialog), then `Restore`.
    Restore,
    /// The settings restored, and how bringing back the computer's apps and
    /// tools went (`None`: nothing to bring back).
    Restored(Result<(Manifest, Option<Result<String, String>>), String>),
    SignOut,
    SignedOut,
    Dismiss,
}

#[derive(Debug, Default)]
enum Step {
    #[default]
    Idle,
    Asking,
    Code(DeviceCode),
    /// Showing the recovery key: just made (`true`), or asked for again.
    Key(String, bool),
    /// Waiting for another computer to hand over the key; the code to compare.
    KeyWait(String),
}

#[derive(Debug)]
pub struct Account {
    files: Option<Files>,
    server: Option<String>,
    status: Status,
    step: Step,
    wait: Option<Handle>,
    recovery: String,
    /// Other computers waiting for the key (when this one has it).
    waiting: Vec<account::Waiting>,
    polling: bool,
    latest: Option<Result<Option<Manifest>, String>>,
    /// The account was made here just now (nothing to restore).
    fresh: bool,
    busy: Option<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
}

impl Default for Account {
    fn default() -> Self {
        let files = Files::user().ok();
        let status = files.as_ref().map(account::status).unwrap_or(Status::SignedOut);
        Account {
            files,
            server: account::default_server(),
            status,
            step: Step::Idle,
            wait: None,
            recovery: String::new(),
            waiting: Vec::new(),
            polling: false,
            latest: None,
            fresh: false,
            busy: None,
            error: None,
            notice: None,
        }
    }
}

/// Bring back the computer's apps and command-line tools the account keeps,
/// if they differ from this computer's (the system service asks for the
/// administrator's password). `Ok`: the computer they came from.
async fn restore_computer(files: &Files) -> Option<Result<String, String>> {
    let setup = match account::computer_setup(files).await {
        Ok(Some(s)) if !s.matches(&files.system) => s,
        Ok(_) => return None,
        Err(e) => return Some(Err(text_err(e))),
    };
    let from = setup.computer.clone();
    let request = iliada_system::protocol::Request::RestoreSettings { settings: setup.settings, apps: setup.apps };
    Some(iliada_system::client::call(&request, &mut |_| {}).await.map(|_| from).map_err(|e| format!("{e:#}")))
}

/// An error as a sentence for the page.
fn text_err(e: anyhow::Error) -> String {
    let s = format!("{e:#}");
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => s,
    }
}

impl Account {
    /// Whether this computer can use an Iliada account at all (sign in, or
    /// already signed in from before).
    pub fn available(&self) -> bool {
        self.server.is_some() || self.status != Status::SignedOut
    }

    /// Sign-in is off and this person isn't already signed in: show that
    /// accounts are coming soon, not a broken sign-in.
    pub fn coming_soon(&self) -> bool {
        !self.available()
    }

    pub fn signed_in(&self) -> bool {
        matches!(self.status, Status::Ready { .. })
    }

    /// Reads the saved state again; when signed in, looks for a backup.
    pub fn refresh(&mut self) -> Task<Msg> {
        if let Some(f) = &self.files {
            self.status = account::status(f);
        }
        let mut tasks = Vec::new();
        if self.signed_in() && self.latest.is_none() {
            let files = self.files.clone().expect("signed in");
            tasks.push(Task::perform(async move { account::latest(&files).await.map_err(text_err) }, Msg::Latest));
        }
        if self.signed_in() && !self.polling {
            self.polling = true;
            tasks.push(Task::done(Msg::PollWaiting));
        }
        Task::batch(tasks)
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        let Some(files) = self.files.clone() else {
            self.error = Some(fl!("account-no-home"));
            return Task::none();
        };
        match msg {
            Msg::SignIn => {
                let Some(server) = self.server.clone() else { return Task::none() };
                self.error = None;
                self.notice = None;
                self.step = Step::Asking;
                return Task::perform(
                    async move { account::start_sign_in(&server).await.map_err(text_err) },
                    Msg::Code,
                );
            }
            Msg::Code(Err(e)) => {
                self.step = Step::Idle;
                self.error = Some(e);
            }
            Msg::Code(Ok(code)) => {
                open_url(code.page());
                let server = self.server.clone().expect("asked it");
                let c = code.clone();
                self.step = Step::Code(code);
                let (task, handle) = Task::perform(
                    async move { account::finish_sign_in(&files, &server, &c).await.map_err(text_err) },
                    Msg::SignedIn,
                )
                .abortable();
                self.wait = Some(handle);
                return task;
            }
            Msg::OpenPage => {
                if let Step::Code(c) = &self.step {
                    open_url(c.page());
                }
            }
            Msg::CancelSignIn => {
                if let Some(h) = self.wait.take() {
                    h.abort();
                }
                self.step = Step::Idle;
            }
            Msg::SignedIn(r) => {
                self.wait = None;
                self.step = Step::Idle;
                self.latest = None;
                match r {
                    Err(e) => self.error = Some(e),
                    Ok((email, KeyStep::Created(key))) => {
                        self.notice = Some(fl!("signed-in-done", account = email));
                        self.step = Step::Key(key, true);
                        self.fresh = true;
                    }
                    Ok((email, KeyStep::Needed | KeyStep::Ready)) => {
                        self.notice = Some(fl!("signed-in-done", account = email))
                    }
                }
                return self.refresh();
            }
            Msg::KeySaved => {
                self.step = Step::Idle;
                // The first backup right away: the account isn't empty for long.
                return self.update(Msg::BackUp);
            }
            Msg::ShowKey => {
                if let Some(k) = account::recovery_key(&files) {
                    self.step = Step::Key(k, false);
                }
            }
            Msg::HideKey => self.step = Step::Idle,
            Msg::CopyKey => {
                if let Step::Key(k, _) = &self.step {
                    return cosmic::iced::clipboard::write(k.clone());
                }
            }
            Msg::RecoveryInput(s) => self.recovery = s,
            Msg::UseRecoveryKey => {
                let text = self.recovery.clone();
                self.busy = Some(fl!("account-busy-checking-key"));
                self.error = None;
                return Task::perform(
                    async move { account::use_recovery_key(&files, &text).await.map_err(text_err) },
                    Msg::KeyUsed,
                );
            }
            Msg::KeyUsed(r) => {
                self.busy = None;
                self.wait = None;
                self.step = Step::Idle;
                match r {
                    Ok(()) => {
                        self.recovery.clear();
                        self.notice = Some(fl!("account-can-read-backup"));
                        self.latest = None;
                        return self.refresh();
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Msg::AskOtherComputer => {
                self.busy = Some(fl!("account-busy-asking"));
                self.error = None;
                return Task::perform(async move { account::ask_for_key(&files).await.map_err(text_err) }, Msg::Asked);
            }
            Msg::Asked(Err(e)) => {
                self.busy = None;
                self.error = Some(e);
            }
            Msg::Asked(Ok(request)) => {
                self.busy = None;
                self.step = Step::KeyWait(request.code.clone());
                let (task, handle) = Task::perform(
                    async move { account::wait_for_key(&files, &request).await.map_err(text_err) },
                    Msg::KeyUsed,
                )
                .abortable();
                self.wait = Some(handle);
                return task;
            }
            Msg::CancelKeyWait => {
                if let Some(h) = self.wait.take() {
                    h.abort();
                }
                self.step = Step::Idle;
            }
            Msg::PollWaiting => {
                if !self.signed_in() {
                    self.polling = false;
                    return Task::none();
                }
                return Task::perform(
                    async move { account::waiting_for_key(&files).await.map_err(text_err) },
                    Msg::WaitingList,
                );
            }
            Msg::WaitingList(r) => {
                // A failed look (offline) just waits for the next one.
                if let Ok(w) = r {
                    self.waiting = w;
                }
                return Task::perform(tokio::time::sleep(std::time::Duration::from_secs(10)), |_| Msg::PollWaiting);
            }
            Msg::GiveKey(id) | Msg::RefuseKey(id) if !self.waiting.iter().any(|w| w.id == id) => {}
            Msg::GiveKey(id) => {
                let to = self.waiting.iter().find(|w| w.id == id).cloned().expect("checked");
                self.waiting.retain(|w| w.id != id);
                self.busy = Some(fl!("account-busy-handing-over"));
                self.error = None;
                return Task::perform(
                    async move { account::hand_over_key(&files, &to).await.map(|()| to.computer).map_err(text_err) },
                    Msg::Answered,
                );
            }
            Msg::RefuseKey(id) => {
                let to = self.waiting.iter().find(|w| w.id == id).cloned().expect("checked");
                self.waiting.retain(|w| w.id != id);
                return Task::perform(
                    async move {
                        let _ = account::refuse_key(&files, &to).await;
                    },
                    |_| Msg::Refused,
                );
            }
            Msg::Refused => {}
            Msg::Answered(r) => {
                self.busy = None;
                match r {
                    Ok(computer) => self.notice = Some(fl!("account-key-handed", computer = computer)),
                    Err(e) => self.error = Some(e),
                }
            }
            Msg::BackUp => {
                self.busy = Some(fl!("activity-backing-up"));
                self.error = None;
                return Task::perform(
                    async move { account::backup(&files, true).await.map_err(text_err) },
                    Msg::BackedUp,
                );
            }
            Msg::BackedUp(r) => {
                self.busy = None;
                match r {
                    Ok(_) => {
                        self.notice = Some(fl!("backed-up"));
                        self.latest = None;
                    }
                    Err(e) => self.error = Some(fl!("account-backup-failed", error = e)),
                }
                return self.refresh();
            }
            Msg::Latest(r) => self.latest = Some(r),
            Msg::Restore => {
                self.busy = Some(fl!("account-busy-restoring"));
                self.error = None;
                return Task::perform(
                    async move {
                        let (m, _) = account::restore(&files).await.map_err(text_err)?;
                        Ok((m, restore_computer(&files).await))
                    },
                    Msg::Restored,
                );
            }
            Msg::Restored(r) => {
                self.busy = None;
                match r {
                    Ok((m, apps)) => {
                        let mut notice = fl!("account-settings-back", computer = m.computer.as_str());
                        match apps {
                            Some(Ok(from)) => {
                                notice.push(' ');
                                notice.push_str(&fl!("account-apps-installing", computer = from));
                            }
                            Some(Err(e)) => self.error = Some(fl!("account-apps-failed", error = e)),
                            None => {}
                        }
                        self.notice = Some(notice);
                    }
                    Err(e) => self.error = Some(fl!("account-restore-failed", error = e)),
                }
            }
            Msg::SignOut => {
                self.busy = Some(fl!("activity-signing-out"));
                return Task::perform(
                    async move {
                        let _ = account::sign_out(&files).await;
                    },
                    |_| Msg::SignedOut,
                );
            }
            Msg::SignedOut => {
                self.busy = None;
                self.step = Step::Idle;
                self.latest = None;
                self.notice = Some(fl!("account-signed-out"));
                return self.refresh();
            }
            Msg::Dismiss => {
                self.error = None;
                self.notice = None;
            }
        }
        Task::none()
    }

    /// The account's latest backup came from another computer (or this one).
    pub fn latest(&self) -> Option<&Manifest> {
        self.latest.as_ref()?.as_ref().ok()?.as_ref()
    }

    /// Signing in or showing the key: the page shows only that.
    pub fn in_flow(&self) -> bool {
        !matches!(self.step, Step::Idle) || self.busy.is_some()
    }

    fn idle(&self) -> bool {
        self.busy.is_none() && matches!(self.step, Step::Idle)
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let mut items: Vec<Element<'_, Msg>> = Vec::new();
        if let Some(b) = self.banner() {
            items.push(b);
        }
        if self.coming_soon() {
            items.push(
                settings::section()
                    .title(fl!("page-account"))
                    .add(info(fl!("account-coming-soon"), fl!("account-coming-soon-desc")))
                    .into(),
            );
            return settings::view_column(items).into();
        }
        match &self.step {
            Step::Code(c) => {
                items.push(code_card(c));
                return settings::view_column(items).into();
            }
            Step::Key(k, new) => {
                items.push(key_card(k, *new));
                return settings::view_column(items).into();
            }
            Step::KeyWait(code) => {
                items.push(key_wait_card(code));
                return settings::view_column(items).into();
            }
            Step::Idle | Step::Asking => {}
        }
        let idle = self.idle();
        match &self.status {
            Status::SignedOut => items.push(
                settings::section()
                    .title(fl!("page-account"))
                    .add(
                        settings::item::builder(fl!("account-sign-in-title"))
                            .description(fl!("account-sign-in-desc"))
                            .control(
                                widget::button::suggested(fl!("sign-in")).on_press_maybe(idle.then_some(Msg::SignIn)),
                            ),
                    )
                    .into(),
            ),
            Status::NeedsKey { email } => {
                items.push(self.recovery_section().title(fl!("page-account")).add(signed_in_as(email, idle)).into())
            }
            Status::Ready { email, last_backup, last_error } => {
                if !self.waiting.is_empty() {
                    let mut s = settings::section().title(fl!("account-key-request-title"));
                    for w in &self.waiting {
                        let buttons = widget::row::with_capacity(2)
                            .push(
                                widget::button::standard(fl!("refuse"))
                                    .on_press_maybe(idle.then(|| Msg::RefuseKey(w.id.clone()))),
                            )
                            .push(
                                widget::button::suggested(fl!("approve"))
                                    .on_press_maybe(idle.then(|| Msg::GiveKey(w.id.clone()))),
                            )
                            .spacing(theme::spacing().space_xs);
                        s = s.add(
                            settings::item::builder(fl!("account-wants-backup", computer = w.computer.as_str()))
                                .description(fl!("account-approve-only-if", code = w.code.as_str()))
                                .control(buttons),
                        );
                    }
                    items.push(s.into());
                }
                let last = match (last_backup, last_error) {
                    (_, Some(e)) => fl!("account-last-backup-failed", error = e.as_str()),
                    (Some(t), None) => fl!("last-backup", ago = ago(*t)),
                    (None, None) => fl!("not-backed-up"),
                };
                items.push(
                    settings::section()
                        .title(fl!("page-account"))
                        .add(signed_in_as(email, idle))
                        .add(
                            settings::item::builder(fl!("page-backup"))
                                .description(fl!("account-backup-schedule", last = last))
                                .control(
                                    widget::button::standard(fl!("back-up-now"))
                                        .on_press_maybe(idle.then_some(Msg::BackUp)),
                                ),
                        )
                        .add(
                            settings::item::builder(fl!("recovery-key"))
                                .description(fl!("recovery-key-desc"))
                                .control(widget::button::standard(fl!("show")).on_press(Msg::ShowKey)),
                        )
                        .into(),
                );
                let mut what = settings::section().title(fl!("whats-backed-up"));
                for (path, label) in account::PARTS {
                    what = what.add(info(part_label(path, label), ""));
                }
                items.push(what.into());
                let restore = match &self.latest {
                    None => info(fl!("looking-for-backup"), ""),
                    Some(Err(e)) => info(fl!("backup-look-failed"), e.as_str()),
                    Some(Ok(None)) => info(fl!("account-no-backup"), ""),
                    Some(Ok(Some(m))) => restore_item(m, fl!("restore-ellipsis"), idle),
                };
                items.push(settings::section().title(fl!("restore")).add(restore).into());
            }
        }
        settings::view_column(items).into()
    }

    /// The Setup Assistant's shorter version: sign in, the recovery key, and
    /// bringing back the account's backup.
    pub fn view_setup(&self) -> Element<'_, Msg> {
        let mut col = widget::column::with_capacity(3).spacing(theme::spacing().space_m).align_x(Alignment::Center);
        if let Some(b) = self.banner() {
            col = col.push(b);
        }
        let idle = self.idle();
        let content: Element<'_, Msg> = match (&self.step, &self.status) {
            (Step::Code(c), _) => code_card(c),
            (Step::Key(k, new), _) => key_card(k, *new),
            (Step::KeyWait(code), _) => key_wait_card(code),
            (_, Status::SignedOut) => {
                widget::button::suggested(fl!("sign-in")).on_press_maybe(idle.then_some(Msg::SignIn)).into()
            }
            (_, Status::NeedsKey { .. }) => self.recovery_section().into(),
            (_, Status::Ready { email, .. }) => {
                let mut s = settings::section().add(settings::item_row(vec![
                    widget::text::body(fl!("signed-in-as", account = email.as_str())).width(Length::Fill).into(),
                ]));
                // A new account has nothing to bring back yet.
                if let (Some(m), false) = (self.latest(), self.fresh) {
                    s = s.add(restore_item(m, fl!("restore"), idle));
                }
                s.into()
            }
        };
        col = col.push(content);
        col.into()
    }

    fn recovery_section(&self) -> settings::Section<'_, Msg> {
        let idle = self.idle();
        let input = widget::text_input("ABCD-EFGH-…", &self.recovery)
            .on_input(Msg::RecoveryInput)
            .on_submit(|_| Msg::UseRecoveryKey)
            .font(cosmic::font::mono())
            .width(Length::Fill);
        let go = widget::button::standard(fl!("continue"))
            .on_press_maybe((idle && !self.recovery.trim().is_empty()).then_some(Msg::UseRecoveryKey));
        let row = widget::row::with_capacity(2)
            .push(input)
            .push(go)
            .spacing(theme::spacing().space_xs)
            .align_y(Alignment::Center);
        settings::section()
            .add(
                settings::item::builder(fl!("get-from-other-computer"))
                    .description(fl!("get-from-other-computer-desc"))
                    .control(
                        widget::button::suggested(fl!("ask-other-computer"))
                            .on_press_maybe(idle.then_some(Msg::AskOtherComputer)),
                    ),
            )
            .add(info(fl!("type-recovery-key"), fl!("type-recovery-key-desc")))
            .add(settings::item_row(vec![row.into()]))
    }

    fn banner(&self) -> Option<Element<'_, Msg>> {
        let spacing = theme::spacing();
        let asking = matches!(self.step, Step::Asking).then(|| fl!("account-busy-asking-code"));
        let row = if let Some(what) = self.busy.clone().or(asking) {
            widget::row::with_capacity(2)
                .push(widget::progress_bar::circular::Circular::new().size(20.0))
                .push(widget::text::heading(format!("{what}…")).width(Length::Fill))
        } else {
            let (text, icon) = match (&self.error, &self.notice) {
                (Some(e), _) => (e.as_str(), "dialog-error-symbolic"),
                (None, Some(n)) => (n.as_str(), "emblem-ok-symbolic"),
                _ => return None,
            };
            widget::row::with_capacity(3)
                .push(widget::icon::from_name(icon).size(20).icon())
                .push(widget::text::body(text).width(Length::Fill))
                .push(widget::button::icon(widget::icon::from_name("window-close-symbolic")).on_press(Msg::Dismiss))
        };
        Some(card(row.spacing(spacing.space_s).align_y(Alignment::Center).into()))
    }
}

fn signed_in_as(email: &str, idle: bool) -> Element<'_, Msg> {
    settings::item::builder(fl!("signed-in-as", account = email))
        .control(widget::button::standard(fl!("sign-out")).on_press_maybe(idle.then_some(Msg::SignOut)))
        .into()
}

fn code_card(c: &DeviceCode) -> Element<'_, Msg> {
    let spacing = theme::spacing();
    let buttons = widget::row::with_capacity(2)
        .push(widget::button::standard(fl!("cancel")).on_press(Msg::CancelSignIn))
        .push(widget::button::suggested(fl!("open-sign-in-page")).on_press(Msg::OpenPage))
        .spacing(spacing.space_s);
    let col = widget::column::with_capacity(5)
        .push(widget::text::body(fl!("sign-in-page-code")))
        .push(widget::text::title1(c.user_code.as_str()))
        .push(buttons)
        .push(widget::text::caption(fl!("waiting-approval", url = c.verification_uri.as_str())))
        .spacing(spacing.space_s)
        .align_x(Alignment::Center);
    card(col.width(Length::Fill).into())
}

fn key_card(key: &str, new: bool) -> Element<'_, Msg> {
    let spacing = theme::spacing();
    let text = if new { fl!("recovery-key-new") } else { fl!("recovery-key-again") };
    let done = if new { fl!("saved-it") } else { fl!("done") };
    let buttons = widget::row::with_capacity(2)
        .push(widget::button::standard(fl!("copy")).on_press(Msg::CopyKey))
        .push(widget::button::suggested(done).on_press(if new { Msg::KeySaved } else { Msg::HideKey }))
        .spacing(spacing.space_s);
    let col = widget::column::with_capacity(4)
        .push(widget::text::title4(fl!("your-recovery-key")))
        .push(widget::text::title4(key).font(cosmic::font::mono()))
        .push(widget::text::body(text))
        .push(buttons)
        .spacing(spacing.space_m)
        .align_x(Alignment::Center);
    card(col.width(Length::Fill).into())
}

fn info<'a>(title: impl Into<String>, description: impl Into<String>) -> Element<'a, Msg> {
    let description = description.into();
    let mut col = widget::column::with_capacity(2).push(widget::text::body(title.into()));
    if !description.is_empty() {
        col = col.push(widget::text::caption(description));
    }
    settings::item_row(vec![col.spacing(2).width(Length::Fill).into()]).into()
}

fn card(content: Element<'_, Msg>) -> Element<'_, Msg> {
    widget::container(content)
        .padding(theme::spacing().space_s)
        .class(theme::Container::Card)
        .width(Length::Fill)
        .into()
}

/// What a part of the backup is, in the person's language.
fn part_label(path: &str, english: &str) -> String {
    let id = match path {
        ".config/cosmic" => "part-desktop",
        ".local/share/iliada/permissions.toml" => "part-privacy",
        ".local/share/iliada/conversations" => "part-conversations",
        _ => return english.to_owned(),
    };
    crate::i18n::lookup(id).unwrap_or_else(|| english.to_owned())
}

fn restore_item<'a>(m: &'a Manifest, button: String, idle: bool) -> Element<'a, Msg> {
    settings::item::builder(fl!("backup-from", computer = m.computer.as_str()))
        .description(fl!("backup-made", ago = ago(m.created)))
        .control(widget::button::standard(button).on_press_maybe(idle.then_some(Msg::Restore)))
        .into()
}

fn key_wait_card(code: &str) -> Element<'_, Msg> {
    let spacing = theme::spacing();
    let col = widget::column::with_capacity(4)
        .push(widget::text::body(fl!("key-wait")))
        .push(widget::text::title1(code).font(cosmic::font::mono()))
        .push(widget::button::standard(fl!("cancel")).on_press(Msg::CancelKeyWait))
        .push(widget::text::caption(fl!("waiting-other-computer")))
        .spacing(spacing.space_s)
        .align_x(Alignment::Center);
    card(col.width(Length::Fill).into())
}
