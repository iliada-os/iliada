//! Iliada's settings pages (the account, updates, backup, privacy, Homer,
//! command-line tools, services, about) as one panel: state, update and
//! views, without a window of its own. COSMIC Settings shows these pages
//! among its own (Wi-Fi, displays, sound…), so there is one Settings app;
//! `iliada-settings --standalone` shows them alone (development).

use crate::account::{self, Account};
use crate::i18n::{LANGUAGE_NAMES, ago, describe, language_name, session_has_language};
use crate::{Progress, fl, open_url, run};
use cosmic::Element;
use cosmic::app::Task;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, settings};
use futures_util::StreamExt;
use iliada_bundle::policy::{self, AppFacts, Entry, Key, Policy, Rule, Value};
use iliada_system::client;
use iliada_system::protocol::{AssistantMode, AssistantSettings, Backup, Event, Provider, Request, Status, model_name};

/// Models offered in Settings (the first is the default). Claude models use
/// an Anthropic key; "vendor/model" ones an OpenRouter key.
const MODELS: [&str; 5] =
    ["claude-opus-5-5", "claude-fable-5-1", "claude-sonnet-5-5", "claude-haiku-5-5", "deepseek/deepseek-v4-flash"];
/// The models as people read them, in the same order.
static MODEL_NAMES: std::sync::LazyLock<Vec<String>> =
    std::sync::LazyLock::new(|| MODELS.iter().map(|m| model_name(m)).collect());
use iliada_assistant::email::{EmailAccount, Security};

static SECURITY_NAMES: std::sync::LazyLock<[String; 2]> =
    std::sync::LazyLock::new(|| [fl!("security-tls"), fl!("security-starttls")]);

fn security_index(s: Security) -> usize {
    usize::from(s == Security::Starttls)
}

/// The email account being edited in Settings → Assistant.
#[derive(Debug, Default)]
struct EmailForm {
    address: String,
    server: String,
    security: Security,
    password: String,
    /// The server was filled in from the address (so it follows it).
    suggested: bool,
}

impl EmailForm {
    fn load() -> Self {
        match iliada_assistant::email::load() {
            Some(a) => EmailForm { address: a.address, server: a.server, security: a.security, ..Default::default() },
            None => EmailForm::default(),
        }
    }
}

/// One of Iliada's settings pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Account,
    Updates,
    Backup,
    Tools,
    Services,
    Privacy,
    Notifications,
    Assistant,
    Language,
    About,
}

impl Page {
    pub const ALL: [Page; 10] = [
        Page::Account,
        Page::Privacy,
        Page::Notifications,
        Page::Assistant,
        Page::Updates,
        Page::Backup,
        Page::Services,
        Page::Tools,
        Page::Language,
        Page::About,
    ];

    /// Page id. COSMIC Settings opens it as two arguments: `iliada <id>`
    /// (`cosmic-settings iliada updates`), not one glued name.
    pub fn id(self) -> &'static str {
        match self {
            Page::Account => "account",
            Page::Updates => "updates",
            Page::Backup => "backup",
            Page::Tools => "tools",
            Page::Services => "services",
            Page::Privacy => "privacy",
            Page::Notifications => "notifications",
            Page::Assistant => "homer",
            Page::Language => "language",
            Page::About => "about",
        }
    }

    pub fn from_id(id: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn title(self) -> String {
        match self {
            Page::Account => fl!("page-account"),
            Page::Updates => fl!("page-updates"),
            Page::Backup => fl!("page-backup"),
            Page::Tools => fl!("page-tools"),
            Page::Services => fl!("page-services"),
            Page::Privacy => fl!("page-privacy"),
            Page::Notifications => fl!("page-notifications"),
            Page::Assistant => fl!("page-homer"),
            Page::Language => fl!("page-language"),
            Page::About => fl!("page-about"),
        }
    }

    pub fn description(self) -> String {
        match self {
            Page::Account => fl!("page-account-desc"),
            Page::Updates => fl!("page-updates-desc"),
            Page::Backup => fl!("page-backup-desc"),
            Page::Tools => fl!("page-tools-desc"),
            Page::Services => fl!("page-services-desc"),
            Page::Privacy => fl!("page-privacy-desc"),
            Page::Notifications => fl!("page-notifications-desc"),
            Page::Assistant => fl!("page-homer-desc"),
            Page::Language => fl!("page-language-desc"),
            Page::About => fl!("page-about-desc"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Page::Account => "avatar-default-symbolic",
            Page::Updates => "software-update-available-symbolic",
            Page::Backup => "document-save-symbolic",
            Page::Tools => "utilities-terminal-symbolic",
            Page::Services => "network-server-symbolic",
            Page::Privacy => "security-high-symbolic",
            Page::Notifications => "preferences-system-notifications-symbolic",
            Page::Assistant => "user-available-symbolic",
            Page::Language => "preferences-desktop-locale-symbolic",
            Page::About => "help-about-symbolic",
        }
    }
}

/// An app's permissions that are asked for (and can be changed here).
/// How Settings → Privacy lists permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrivacyBy {
    #[default]
    App,
    Capability,
}

/// A folder's choices, as Settings says them, and their values in the file.
static FOLDER_CHOICES: std::sync::LazyLock<[String; 4]> = std::sync::LazyLock::new(|| {
    [fl!("folder-read-write"), fl!("folder-read-only"), fl!("folder-not-allowed"), fl!("folder-ask-next")]
});
const FOLDER_VALUES: [Value; 4] = [Value::ReadWrite, Value::ReadOnly, Value::Deny, Value::Ask];

/// Whether Homer may read notifications, as Settings says it.
static HOMER_NOTIFICATION_CHOICES: std::sync::LazyLock<[String; 3]> = std::sync::LazyLock::new(|| {
    [fl!("homer-notifications-ask"), fl!("homer-notifications-allow"), fl!("homer-notifications-deny")]
});
const HOMER_NOTIFICATION_VALUES: [Value; 3] = [Value::Ask, Value::Allow, Value::Deny];

/// One of an app's notification switches (Settings → Notifications).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppSwitch {
    Allow,
    Banners,
    Sound,
    History,
}

/// One capability's control: a dropdown for folders, a switch otherwise.
fn permission_control<'a>(app: &str, e: &Entry, title: String) -> Element<'a, Message> {
    let mut item = settings::item::builder(title);
    let mut said = Vec::new();
    if e.setting == Value::Ask {
        said.push(fl!("asks-next-time"));
    }
    if let Some(s) = &e.scope {
        said.push(s.clone());
    }
    if let Some(n) = &e.note {
        said.push(n.clone());
    }
    if !said.is_empty() {
        item = item.description(said.join(". "));
    }
    let (id, key) = (app.to_owned(), e.key);
    if key.folder().is_some() {
        let selected = FOLDER_VALUES.iter().position(|v| *v == e.setting);
        item.control(widget::dropdown(&FOLDER_CHOICES[..], selected, move |i| {
            Message::Permission(id.clone(), key, FOLDER_VALUES[i])
        }))
        .into()
    } else {
        item.control(
            widget::toggler(e.setting == Value::Allow)
                .width(Length::Shrink)
                .on_toggle(move |on| Message::Permission(id.clone(), key, if on { Value::Allow } else { Value::Deny })),
        )
        .into()
    }
}

fn extra_isolation_path() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"));
    Some(cfg.join("iliada/extra-isolation"))
}

/// Apps the user put in their own micro-VM (`iliada-run` reads the same file).
fn load_extra_isolation() -> Vec<String> {
    extra_isolation_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn save_extra_isolation(ids: &[String]) -> std::io::Result<()> {
    let Some(p) = extra_isolation_path() else { return Ok(()) };
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, ids.iter().map(|i| format!("{i}\n")).collect::<String>())
}

/// What the provider said about the saved API key.
#[derive(Debug, Clone)]
enum KeyCheck {
    Checking,
    Works(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum Confirm {
    Restore(String),
    SignOut,
    AccountRestore(String),
    AccountSignOut,
    RemoveKey(Provider),
    TestingUpdates,
    ResetPermissions,
}

// Messages are moved around once each; boxing the large one buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Account(account::Msg),
    Status(Result<Status, String>),
    Run(Request),
    Progress(Progress),
    ToolInput(String),
    AddTool,
    BackupName(String),
    OpenSignIn,
    LoadBackups,
    Backups(Result<Vec<Backup>, String>),
    Ask(Confirm),
    Permission(String, Key, Value),
    HomerApps(bool),
    PrivacyBy(PrivacyBy),
    OpenPermissionsFile,
    ExtraIsolation(String, bool),
    AssistantSet(AssistantSettings),
    AssistantKey(String),
    SaveKey,
    ReplaceKey,
    CancelReplaceKey,
    RemoveKey,
    KeyChecked(Result<String, String>),
    EmailAddress(String),
    EmailServer(String),
    EmailSecurity(usize),
    EmailPassword(String),
    SaveEmail,
    RemoveEmail,
    SaveReport,
    ReportSaved(Result<String, String>),
    OpenReport,
    /// Open Homer's voice file (made from the default the first time).
    EditVoice,
    /// Homer keeps conversations, or not.
    KeepHistory(bool),
    ClearHistory,
    ClearHistoryConfirmed,
    ClipHistory(bool),
    ClipExclude(String, bool),
    ClipClear,
    ClipClearConfirmed,
    /// Search lists open windows, or not.
    ShowWindows(bool),
    /// Turn a Search extension on or off.
    ExtensionOn(String, bool),
    /// Start choosing another keyword for an extension.
    ChangeKeyword(String),
    KeywordInput(String),
    SaveKeyword,
    CancelKeyword,
    /// Settings → Notifications: Do Not Disturb now, its schedule
    /// (on or off, and the times being typed), critical notifications.
    Dnd(bool),
    DndSchedule(bool),
    DndFrom(String),
    DndTo(String),
    DndSaveTimes,
    DndCritical(bool),
    /// Whether Homer may read notifications (an index into the choices).
    HomerNotifications(usize),
    NotifyApp(String, AppSwitch, bool),
    /// Log out (to see a new language).
    LogOut,
    /// COSMIC Settings' keyboard page (layouts, shortcuts).
    OpenKeyboard,
    Confirmed,
    Cancel,
    Dismiss,
}

struct Job {
    request: Request,
    line: String,
}

pub struct Panel {
    page: Page,
    status: Option<Result<Status, String>>,
    job: Option<Job>,
    sign_in: Option<(String, String)>,
    error: Option<String>,
    notice: Option<String>,
    tool_input: String,
    backup_name: String,
    backups: Option<Result<Vec<Backup>, String>>,
    confirm: Option<Confirm>,
    /// Every program the permissions file describes, and the file itself.
    programs: Vec<AppFacts>,
    policy: Policy,
    privacy_by: PrivacyBy,
    extra_isolation: Vec<String>,
    key_input: String,
    replacing_key: bool,
    key_check: Option<KeyCheck>,
    /// The hardware report: None before one is asked for, Some(None) while it is made.
    report: Option<Option<Result<String, String>>>,
    email: EmailForm,
    account: Account,
    /// Homer keeps conversations (`~/.config/iliada/homer.toml`).
    keep_history: bool,
    /// "Delete all of Homer's conversations?" is showing.
    confirm_clear_history: bool,
    /// Clipboard history (Super+V), Settings → Privacy.
    clip_enabled: bool,
    clip_exclude: Vec<String>,
    confirm_clear_clipboard: bool,
    /// Search shows open windows (`~/.config/iliada/search.toml`).
    show_windows: bool,
    /// Choosing a keyword for this extension: its id, and what is typed.
    keyword_edit: Option<(String, String)>,
    /// Settings → Notifications (`~/.config/iliada/notifications.toml`),
    /// and the schedule's times as typed.
    notify: iliada_notify::settings::Settings,
    dnd_from: String,
    dnd_to: String,
}

fn load_status() -> Task<Message> {
    cosmic::task::future(async { Message::Status(client::status().await.map_err(|e| format!("{e:#}"))) })
}

fn load_backups() -> Task<Message> {
    cosmic::task::future(async {
        Message::Backups(client::call_as(&Request::BackupList, &mut |_| {}).await.map_err(|e| format!("{e:#}")))
    })
}

/// What to tell the user when a request succeeds.
fn success(request: &Request, result: &serde_json::Value) -> Option<String> {
    let changed = result.get("changed").and_then(|c| c.as_bool()).unwrap_or(true);
    Some(match request {
        Request::AddTools { packages } if changed => fl!("installed-tools-done", names = packages.join(", ")),
        Request::RemoveTools { packages } if changed => fl!("removed-tools-done", names = packages.join(", ")),
        Request::SetService { .. } | Request::Apply if changed => fl!("settings-applied"),
        Request::UpdateApps => fl!("apps-up-to-date"),
        Request::UpdateSystem => match result["updated"].as_bool() {
            Some(true) => {
                fl!("iliada-installed-restart", version = describe(result["version"].as_str().unwrap_or("")))
            }
            _ => fl!("iliada-up-to-date"),
        },
        Request::SetTestingUpdates { on } if changed => match (on, result["newer"].as_bool()) {
            (true, Some(true)) => fl!("testing-on-ready"),
            (true, _) => fl!("testing-on"),
            (false, _) => fl!("testing-off"),
        },
        Request::CheckSystemUpdate => match result["newer"].as_bool() {
            Some(true) => fl!("iliada-available", version = describe(result["latest"].as_str().unwrap_or(""))),
            _ => fl!("iliada-up-to-date"),
        },
        Request::BackupSignIn => fl!("signed-in-done", account = result["account"].as_str().unwrap_or("")),
        Request::BackupEnable { .. } | Request::BackupNow => fl!("backed-up"),
        Request::Restore { .. } => fl!("restored-installing"),
        Request::BackupSignOut => fl!("signed-out-backups-off"),
        Request::SetAssistant { .. } if changed => fl!("saved"),
        Request::SetLanguage { locale } if changed => fl!("language-changed", language = language_name(locale)),
        _ => return None,
    })
}

impl Panel {
    fn status(&self) -> Option<&Status> {
        self.status.as_ref().and_then(|s| s.as_ref().ok())
    }

    /// Changes are possible: an administrator, and nothing running.
    fn can_change(&self) -> bool {
        self.job.is_none() && self.status().is_some_and(|s| s.admin && s.busy.is_none())
    }

    fn start(&mut self, request: Request) -> Task<Message> {
        if self.job.is_some() {
            return Task::none();
        }
        self.error = None;
        self.notice = None;
        self.job = Some(Job { request: request.clone(), line: String::new() });
        cosmic::task::stream(run(request).map(Message::Progress))
    }

    fn banner(&self) -> Option<Element<'_, Message>> {
        let spacing = cosmic::theme::spacing();
        if let Some(job) = &self.job {
            let what = crate::i18n::activity(job.request.activity());
            let mut col = widget::column::with_capacity(2).push(widget::text::heading(format!("{what}…")));
            if !job.line.is_empty() {
                col = col.push(widget::text::caption(job.line.as_str()));
            }
            let row = widget::row::with_capacity(2)
                .push(widget::progress_bar::circular::Circular::new().size(20.0))
                .push(col.spacing(spacing.space_xxxs).width(Length::Fill))
                .spacing(spacing.space_s)
                .align_y(Alignment::Center);
            return Some(card(row.into()));
        }
        let (text, is_error) = match (&self.error, &self.notice) {
            (Some(e), _) => (e.as_str(), true),
            (None, Some(n)) => (n.as_str(), false),
            _ => return None,
        };
        let icon = if is_error { "dialog-error-symbolic" } else { "emblem-ok-symbolic" };
        let row = widget::row::with_capacity(3)
            .push(widget::icon::from_name(icon).size(20).icon())
            .push(widget::text::body(text).width(Length::Fill))
            .push(widget::button::icon(widget::icon::from_name("window-close-symbolic")).on_press(Message::Dismiss))
            .spacing(spacing.space_s)
            .align_y(Alignment::Center);
        Some(card(row.into()))
    }

    fn updates(&self, s: &Status) -> Element<'_, Message> {
        let ok = self.can_change();
        let u = &s.system_update;
        let newer = u.latest.as_ref().filter(|l| iliada_system::update::newer(l, &s.version));
        let state = match (newer, u.last_check, &u.last_error) {
            (Some(l), _, _) => fl!("iliada-available", version = describe(l)),
            (None, _, Some(e)) => fl!("update-check-failed", error = e.as_str()),
            (None, Some(t), None) => fl!("up-to-date-checked", ago = ago(t)),
            (None, None, None) => fl!("updates-daily"),
        };
        let action = match newer {
            Some(_) => widget::button::suggested(fl!("update-now"))
                .on_press_maybe(ok.then_some(Message::Run(Request::UpdateSystem))),
            None => widget::button::standard(fl!("check-now"))
                .on_press_maybe(ok.then_some(Message::Run(Request::CheckSystemUpdate))),
        };
        let system = settings::section()
            .title(fl!("iliada"))
            .add(
                settings::item::builder(fl!("iliada-version", version = describe(&s.version)))
                    .description(state)
                    .control(action),
            )
            .add(settings::item::builder(fl!("test-updates")).description(fl!("test-updates-desc")).toggler_maybe(
                u.testing,
                ok.then_some(|on| {
                    if on {
                        Message::Ask(Confirm::TestingUpdates)
                    } else {
                        Message::Run(Request::SetTestingUpdates { on: false })
                    }
                }),
            ));
        let apps = settings::section().title(fl!("apps")).add(
            settings::item::builder(fl!("app-updates")).description(fl!("app-updates-desc")).control(
                widget::button::standard(fl!("update-apps-now"))
                    .on_press_maybe(ok.then_some(Message::Run(Request::UpdateApps))),
            ),
        );
        settings::view_column(vec![system.into(), apps.into()]).into()
    }

    fn backup(&self, s: &Status) -> Element<'_, Message> {
        let ok = self.can_change();
        let b = &s.backup;
        let intro = fl!("backup-intro");
        let mut items: Vec<Element<_>> = Vec::new();
        if !b.available {
            items.push(
                settings::section()
                    .title(fl!("page-backup"))
                    .add(info(fl!("backup-unavailable"), Some(fl!("backup-unavailable-desc"))))
                    .into(),
            );
            return settings::view_column(items).into();
        }
        if let Some((code, url)) = &self.sign_in {
            let spacing = cosmic::theme::spacing();
            let col = widget::column::with_capacity(4)
                .push(widget::text::body(fl!("github-enter-code")))
                .push(widget::text::title1(code.as_str()))
                .push(widget::button::suggested(fl!("open-github")).on_press(Message::OpenSignIn))
                .push(widget::text::caption(fl!("github-waiting", url = url.as_str())))
                .spacing(spacing.space_s)
                .align_x(Alignment::Center);
            items.push(card(col.width(Length::Fill).into()));
            return settings::view_column(items).into();
        }
        match (&b.account, &b.repo) {
            (None, _) => {
                items.push(
                    settings::section()
                        .title(fl!("page-backup"))
                        .add(
                            settings::item::builder(fl!("back-up-to-github")).description(intro).control(
                                widget::button::suggested(fl!("sign-in-github"))
                                    .on_press_maybe(ok.then_some(Message::Run(Request::BackupSignIn))),
                            ),
                        )
                        .into(),
                );
            }
            (Some(account), None) => {
                let name_ok = iliada_system::github::valid_name(&self.backup_name);
                let input = widget::text_input("laptop", &self.backup_name)
                    .on_input(Message::BackupName)
                    .on_submit(|_| Message::Run(Request::BackupEnable { name: String::new() }))
                    .width(Length::Fixed(220.0));
                let start = widget::button::suggested(fl!("back-up")).on_press_maybe(
                    (ok && name_ok).then(|| Message::Run(Request::BackupEnable { name: self.backup_name.clone() })),
                );
                let row = widget::row::with_capacity(3)
                    .push(widget::text::body(github_prefix(account)))
                    .push(input)
                    .push(start)
                    .spacing(cosmic::theme::spacing().space_xs)
                    .align_y(Alignment::Center);
                items.push(
                    settings::section()
                        .title(fl!("page-backup"))
                        .add(info(fl!("name-this-backup"), Some(intro)))
                        .add(settings::item_row(vec![row.into()]))
                        .add(
                            settings::item::builder(fl!("signed-in-as", account = account.as_str())).control(
                                widget::button::standard(fl!("sign-out"))
                                    .on_press_maybe(ok.then_some(Message::Ask(Confirm::SignOut))),
                            ),
                        )
                        .into(),
                );
                items.push(self.restore_list(ok));
            }
            (Some(account), Some(repo)) => {
                let last = match (b.last_backup, &b.last_error) {
                    (_, Some(e)) => fl!("last-backup-failed", error = e.as_str()),
                    (Some(t), None) => fl!("last-backup", ago = ago(t)),
                    (None, None) => fl!("not-backed-up"),
                };
                items.push(
                    settings::section()
                        .title(fl!("page-backup"))
                        .add(
                            settings::item::builder(fl!("backing-up-to", repo = repo.as_str()))
                                .description(fl!("backup-schedule", last = last))
                                .control(
                                    widget::button::standard(fl!("back-up-now"))
                                        .on_press_maybe(ok.then_some(Message::Run(Request::BackupNow))),
                                ),
                        )
                        .add(
                            settings::item::builder(fl!("signed-in-as", account = account.as_str())).control(
                                widget::button::standard(fl!("sign-out"))
                                    .on_press_maybe(ok.then_some(Message::Ask(Confirm::SignOut))),
                            ),
                        )
                        .into(),
                );
                items.push(self.restore_list(ok));
            }
        }
        settings::view_column(items).into()
    }

    fn restore_list(&self, ok: bool) -> Element<'_, Message> {
        let mut section = settings::section().title(fl!("restore"));
        match &self.backups {
            None => {
                section = section.add(
                    settings::item::builder(fl!("restore-from-backup"))
                        .description(fl!("restore-from-backup-desc"))
                        .control(widget::button::standard(fl!("show-backups")).on_press(Message::LoadBackups)),
                )
            }
            Some(Err(e)) => {
                section = section.add(
                    settings::item::builder(fl!("backups-list-failed"))
                        .description(e.as_str())
                        .control(widget::button::standard(fl!("retry")).on_press(Message::LoadBackups)),
                )
            }
            Some(Ok(list)) if list.is_empty() => section = section.add(info(fl!("no-backups"), None)),
            Some(Ok(list)) => {
                for b in list {
                    let updated = b.updated.split('T').next().unwrap_or("").to_owned();
                    section = section.add(
                        settings::item::builder(b.name.clone())
                            .description(fl!("backup-item", repo = b.repo.as_str(), date = updated))
                            .control(
                                widget::button::standard(fl!("restore"))
                                    .on_press_maybe(ok.then(|| Message::Ask(Confirm::Restore(b.repo.clone())))),
                            ),
                    );
                }
            }
        }
        section.into()
    }

    fn tools(&self, s: &Status) -> Element<'_, Message> {
        let ok = self.can_change();
        let mut installed = settings::section().title(fl!("installed-tools"));
        if s.tools.is_empty() {
            installed = installed.add(info(fl!("no-tools"), None));
        }
        for t in &s.tools {
            installed = installed.add(
                settings::item::builder(t.clone()).control(
                    widget::button::standard(fl!("remove"))
                        .on_press_maybe(ok.then(|| Message::Run(Request::RemoveTools { packages: vec![t.clone()] }))),
                ),
            );
        }
        let valid = iliada_system::settings::valid_tool_name(self.tool_input.trim());
        let input = widget::text_input(fl!("tool-placeholder"), &self.tool_input)
            .on_input(Message::ToolInput)
            .on_submit(|_| Message::AddTool)
            .width(Length::Fill);
        let add = widget::row::with_capacity(2)
            .push(input)
            .push(widget::button::suggested(fl!("install")).on_press_maybe((ok && valid).then_some(Message::AddTool)))
            .spacing(cosmic::theme::spacing().space_xs)
            .align_y(Alignment::Center);
        let add = settings::section()
            .title(fl!("add-a-tool"))
            .add(info(fl!("page-tools"), Some(fl!("tools-desc"))))
            .add(settings::item_row(vec![add.into()]));
        settings::view_column(vec![installed.into(), add.into()]).into()
    }

    fn services(&self, s: &Status) -> Element<'_, Message> {
        let ok = self.can_change();
        let mut section = settings::section().title(fl!("page-services"));
        for svc in &s.services {
            let name = svc.name.clone();
            let toggle = ok.then_some(move |on| Message::Run(Request::SetService { name: name.clone(), on }));
            section = section.add(
                settings::item::builder(
                    crate::i18n::lookup(&format!("service-{}", svc.name)).unwrap_or_else(|| svc.description.clone()),
                )
                .toggler_maybe(svc.on, toggle),
            );
            // Installed: whether it starts by itself at boot (developers
            // sometimes want a database only while they work).
            if svc.on {
                let name = svc.name.clone();
                let boot = ok.then_some(move |boot| Message::Run(Request::SetServiceBoot { name: name.clone(), boot }));
                let unit = iliada_system::settings::unit_of_service(&svc.name).unwrap_or(&svc.name);
                section = section.add(
                    settings::item::builder(format!("    {}", fl!("start-automatically")))
                        .description(if svc.boot {
                            fl!("starts-at-boot")
                        } else {
                            fl!("starts-when-asked", unit = unit)
                        })
                        .toggler_maybe(svc.boot, boot),
                );
            }
        }
        settings::view_column(vec![section.into()]).into()
    }

    /// Read the permissions file again, and keep its summaries current
    /// (never over a file with a mistake: people fix it where they made it).
    fn load_permissions(&mut self) {
        self.programs = policy::installed_programs();
        self.policy = Policy::load();
        if self.policy.readable() && self.policy.user.problems.is_empty() {
            let programs = self.programs.clone();
            let _ = self.policy.edit(&programs, |_| Ok(()));
        }
    }

    fn privacy(&self) -> Element<'_, Message> {
        let mut items: Vec<Element<_>> = vec![info(fl!("privacy-intro-title"), Some(fl!("privacy-intro")))];
        items.push(self.clipboard_section());
        items.push(self.search_section());
        let by = |label: String, v: PrivacyBy| {
            if self.privacy_by == v {
                widget::button::suggested(label).on_press(Message::PrivacyBy(v))
            } else {
                widget::button::standard(label).on_press(Message::PrivacyBy(v))
            }
        };
        let bar = widget::row::with_capacity(4)
            .push(by(fl!("privacy-by-app"), PrivacyBy::App))
            .push(by(fl!("privacy-by-permission"), PrivacyBy::Capability))
            .push(widget::space::horizontal())
            .push(widget::button::standard(fl!("open-the-file")).on_press(Message::OpenPermissionsFile))
            .spacing(cosmic::theme::spacing().space_xs)
            .align_y(Alignment::Center);
        items.push(settings::item_row(vec![bar.into()]).into());

        let problems = self.policy.problems();
        if !problems.is_empty() {
            let mut section = settings::section().title(fl!("permissions-file-mistake"));
            for p in problems.iter().take(5) {
                section = section.add(info(p.message.clone(), Some(p.to_string())));
            }
            if !self.policy.readable() {
                section = section.add(settings::item::builder(fl!("until-fixed-none")).control(
                    widget::button::destructive(fl!("reset")).on_press(Message::Ask(Confirm::ResetPermissions)),
                ));
            }
            items.push(section.into());
        }

        let sandboxed: Vec<&AppFacts> = self.programs.iter().filter(|a| a.source.sandboxed()).collect();
        if sandboxed.is_empty() {
            items.push(info(fl!("no-apps"), None));
        }
        match self.privacy_by {
            PrivacyBy::App => {
                for app in &sandboxed {
                    let r = self.policy.report(app);
                    let mut section = settings::section().title(app.name.clone());
                    section = section.add(info(fl!("can-touch"), Some(r.summary.join(", "))));
                    section = section.add(info(fl!("worst-case"), Some(r.worst_case.clone())));
                    for key in Key::ALL {
                        let e = r.get(key);
                        if e.changeable {
                            section = section.add(permission_control(&app.id, e, key.label()));
                        }
                    }
                    let loose: Vec<String> =
                        Key::ALL.into_iter().filter(|k| !r.get(*k).enforced).map(|k| k.phrase()).collect();
                    if !loose.is_empty() {
                        section = section.add(info(
                            fl!("not-restricted-yet"),
                            Some(fl!("not-restricted-yet-list", list = loose.join(", "))),
                        ));
                    }
                    let id = app.id.clone();
                    section = section.add(
                        settings::item::builder(fl!("extra-isolation"))
                            .description(fl!("extra-isolation-desc"))
                            .toggler(self.extra_isolation.contains(&app.id), move |on| {
                                Message::ExtraIsolation(id.clone(), on)
                            }),
                    );
                    items.push(section.into());
                }
            }
            PrivacyBy::Capability => {
                let reports: Vec<_> = sandboxed.iter().map(|a| self.policy.report(a)).collect();
                for key in Key::ALL {
                    match key.rule() {
                        Rule::Decided { .. } => {
                            let mut section = settings::section().title(key.label());
                            let mut any = false;
                            for r in &reports {
                                let e = r.get(key);
                                if e.changeable {
                                    any = true;
                                    section = section.add(permission_control(&r.id, e, r.name.clone()));
                                }
                            }
                            if any {
                                items.push(section.into());
                            }
                        }
                        Rule::NotRestrictedYet { why } => {
                            items.push(info(
                                fl!("cap-not-restricted-yet", name = key.label()),
                                Some(fl!("every-app-why", why = why)),
                            ));
                        }
                        Rule::Never { why } => {
                            items.push(info(fl!("cap-never", name = key.label()), Some(fl!("no-app-why", why = why))));
                        }
                    }
                }
            }
        }
        let open: Vec<&AppFacts> = self.programs.iter().filter(|a| !a.source.sandboxed()).collect();
        if !open.is_empty() {
            let names: Vec<&str> = open.iter().map(|a| a.name.as_str()).collect();
            let mut section = settings::section().title(fl!("not-sandboxed"));
            section = section.add(info(fl!("not-sandboxed"), Some(fl!("not-sandboxed-list", list = names.join(", ")))));
            section = section.add(info(fl!("not-sandboxed-homer"), None));
            for app in &open {
                let r = self.policy.report(app);
                let e = r.get(Key::ControlledByHomer);
                if e.changeable {
                    section = section.add(permission_control(&app.id, e, app.name.clone()));
                }
            }
            items.push(section.into());
        }
        settings::view_column(items).into()
    }

    fn assistant(&self, s: &Status) -> Element<'_, Message> {
        let ok = self.can_change();
        let a = s.assistant.clone();
        let on = {
            let a = a.clone();
            ok.then_some(move |enabled| Message::AssistantSet(AssistantSettings { enabled, ..a.clone() }))
        };
        let main = settings::section().title(fl!("homer")).add(
            settings::item::builder(fl!("use-homer")).description(fl!("use-homer-desc")).toggler_maybe(a.enabled, on),
        );
        let mut items: Vec<Element<_>> = vec![main.into()];
        if a.enabled {
            let mode_item = |mode: AssistantMode, title: String, desc: String| {
                let next = AssistantSettings { mode, ..a.clone() };
                settings::item::builder(title).description(desc).control(widget::radio(
                    widget::text::body(""),
                    mode,
                    Some(a.mode),
                    move |_| Message::AssistantSet(next.clone()),
                ))
            };
            let mut modes = settings::section().title(fl!("what-it-may-do"));
            if self.policy.homer_forced_off() {
                modes = modes.add(info(fl!("use-other-apps"), Some(fl!("use-other-apps-off-by-admin"))));
            } else {
                let on = self.policy.homer_enabled();
                let set = ok.then_some(Message::HomerApps);
                modes = modes.add(
                    settings::item::builder(fl!("use-other-apps"))
                        .description(fl!("use-other-apps-desc"))
                        .toggler_maybe(on, set),
                );
            }
            modes = modes
                .add(mode_item(AssistantMode::Ask, fl!("ask-me-first"), fl!("ask-me-first-desc")))
                .add(mode_item(AssistantMode::Auto, fl!("act-on-its-own"), fl!("act-on-its-own-desc")));
            if a.mode == AssistantMode::Auto {
                for (kind, what) in iliada_assistant::tools::KINDS {
                    let allowed = a.on_its_own.iter().any(|k| k == kind);
                    let set = {
                        let a = a.clone();
                        ok.then_some(move |allow: bool| {
                            let mut next = a.clone();
                            next.on_its_own.retain(|k| k != kind);
                            if allow {
                                next.on_its_own.push(kind.to_owned());
                            }
                            Message::AssistantSet(next)
                        })
                    };
                    modes = modes.add(
                        settings::item::builder(fl!(
                            "kind-without-asking",
                            what = crate::i18n::lookup(&format!("kind-{kind}")).unwrap_or_else(|| (*what).to_owned())
                        ))
                        .description(if allowed { fl!("kind-allowed") } else { fl!("kind-asks") })
                        .toggler_maybe(allowed, set),
                    );
                }
            }
            if a.mode == AssistantMode::Auto && !a.recommended_for_auto() {
                modes = modes
                    .add(info(fl!("model-more-mistakes", model = model_name(&a.model)), Some(fl!("model-careful"))));
            }
            let full = {
                let a = a.clone();
                ok.then_some(move |full_access| Message::AssistantSet(AssistantSettings { full_access, ..a.clone() }))
            };
            modes = modes.add(
                settings::item::builder(fl!("full-access"))
                    .description(fl!("full-access-desc"))
                    .toggler_maybe(a.full_access, full),
            );
            items.push(modes.into());
            let selected = MODELS.iter().position(|m| *m == a.model);
            let model_a = a.clone();
            let model = settings::section().title(fl!("model")).add(
                settings::item::builder(fl!("model")).description(fl!("model-desc")).control(widget::dropdown(
                    &MODEL_NAMES[..],
                    selected,
                    move |i| {
                        Message::AssistantSet(AssistantSettings { model: MODELS[i].to_owned(), ..model_a.clone() })
                    },
                )),
            );
            items.push(model.into());
            items.push(
                settings::section()
                    .title(fl!("voice"))
                    .add(
                        settings::item::builder(fl!("homers-voice"))
                            .description(fl!("homers-voice-desc"))
                            .control(widget::button::standard(fl!("edit")).on_press(Message::EditVoice)),
                    )
                    .into(),
            );
            items.push(self.history_section());
            items.push(self.key_section(&a.model));
            items.push(self.email_section());
        }
        settings::view_column(items).into()
    }

    /// Whether Homer keeps conversations, and deleting the ones he has.
    fn history_section(&self) -> Element<'_, Message> {
        let space = cosmic::theme::spacing();
        let mut section = settings::section().title(fl!("history")).add(
            settings::item::builder(fl!("keep-conversations"))
                .description(if self.keep_history {
                    fl!("keep-conversations-on")
                } else {
                    fl!("keep-conversations-off")
                })
                .toggler(self.keep_history, Message::KeepHistory),
        );
        let clear: Element<_> = if self.confirm_clear_history {
            widget::row::with_capacity(2)
                .push(widget::button::standard(fl!("cancel")).on_press(Message::Cancel))
                .push(widget::button::destructive(fl!("delete-all")).on_press(Message::ClearHistoryConfirmed))
                .spacing(space.space_xs)
                .into()
        } else {
            widget::button::standard(fl!("clear-history-ellipsis")).on_press(Message::ClearHistory).into()
        };
        section = section
            .add(settings::item::builder(fl!("clear-history")).description(fl!("clear-history-desc")).control(clear));
        section.into()
    }

    fn clipboard_section(&self) -> Element<'_, Message> {
        let space = cosmic::theme::spacing();
        let mut section = settings::section().title(fl!("clipboard-history")).add(
            settings::item::builder(fl!("clipboard-history"))
                .description(if self.clip_enabled { fl!("clipboard-history-on") } else { fl!("clipboard-history-off") })
                .toggler(self.clip_enabled, Message::ClipHistory),
        );
        section = section.add(info(fl!("clipboard-history"), Some(fl!("clipboard-history-desc"))));
        let clear: Element<_> = if self.confirm_clear_clipboard {
            widget::row::with_capacity(2)
                .push(widget::button::standard(fl!("cancel")).on_press(Message::Cancel))
                .push(widget::button::destructive(fl!("delete-all")).on_press(Message::ClipClearConfirmed))
                .spacing(space.space_xs)
                .into()
        } else {
            widget::button::standard(fl!("clipboard-history-clear-ellipsis")).on_press(Message::ClipClear).into()
        };
        section = section.add(
            settings::item::builder(fl!("clipboard-history-clear"))
                .description(fl!("clipboard-history-clear-desc"))
                .control(clear),
        );
        section = section.add(info(fl!("clipboard-exclude"), Some(fl!("clipboard-exclude-desc"))));
        for app in &self.programs {
            let id = app.id.clone();
            let on = self.clip_exclude.iter().any(|e| clip_excludes(e, &app.id));
            section = section.add(
                settings::item::builder(app.name.clone())
                    .description(app.id.clone())
                    .toggler(on, move |v| Message::ClipExclude(id.clone(), v)),
            );
        }
        section.into()
    }

    fn search_section(&self) -> Element<'_, Message> {
        let space = cosmic::theme::spacing();
        let mut section = settings::section().title(fl!("search-section")).add(
            settings::item::builder(fl!("show-open-windows"))
                .description(fl!("show-open-windows-desc"))
                .toggler(self.show_windows, Message::ShowWindows),
        );
        // Extensions from agora: each one's keyword, whether it can use the
        // internet, on or off, and another keyword when two want the same.
        section = section.add(info(fl!("search-extensions"), Some(fl!("search-extensions-desc"))));
        let extensions = self.extensions();
        if extensions.is_empty() {
            section = section.add(info(fl!("search-extensions-none"), None));
        }
        for (ext, internet) in extensions {
            let id = ext.id.clone();
            let mut desc = fl!("search-extension-keyword", keyword = ext.keyword.as_str());
            desc.push_str(" · ");
            desc.push_str(&if internet {
                fl!("search-extension-internet")
            } else {
                fl!("search-extension-no-internet")
            });
            if let Some(by) = &ext.clashes_with {
                desc.push_str(" · ");
                desc.push_str(&fl!("search-extension-clash", keyword = ext.keyword.as_str(), other = by.as_str()));
            }
            let change =
                widget::button::standard(fl!("search-extension-change")).on_press(Message::ChangeKeyword(id.clone()));
            let toggle = widget::toggler(ext.enabled).on_toggle(move |on| Message::ExtensionOn(id.clone(), on));
            let controls = widget::row::with_capacity(2)
                .push(change)
                .push(toggle)
                .spacing(space.space_xs)
                .align_y(Alignment::Center);
            section = section.add(settings::item::builder(ext.title.clone()).description(desc).control(controls));
            if let Some((editing, text)) = &self.keyword_edit
                && *editing == ext.id
            {
                let problem = keyword_problem(&self.programs, &ext.id, text.trim());
                let input = widget::text_input(ext.declared_keyword.clone(), text)
                    .on_input(Message::KeywordInput)
                    .on_submit(|_| Message::SaveKeyword)
                    .width(Length::Fill);
                let row = widget::row::with_capacity(3)
                    .push(input)
                    .push(widget::button::standard(fl!("cancel")).on_press(Message::CancelKeyword))
                    .push(
                        widget::button::suggested(fl!("search-extension-save"))
                            .on_press_maybe(problem.is_none().then_some(Message::SaveKeyword)),
                    )
                    .spacing(space.space_xs)
                    .align_y(Alignment::Center);
                section = section.add(settings::item_row(vec![row.into()]));
                section = section.add(info(problem.unwrap_or_else(|| fl!("search-extension-keyword-hint")), None));
            }
        }
        section.into()
    }

    /// Installed Search extensions, and whether each can use the internet
    /// now (declared, and allowed in the permissions files).
    fn extensions(&self) -> Vec<(iliada_bundle::search::Extension, bool)> {
        self.programs
            .iter()
            .filter_map(|a| {
                let ext = a.search.clone()?;
                let internet = self.policy.report(a).search.is_some_and(|s| s.internet);
                Some((ext, internet))
            })
            .collect()
    }

    /// Change the person's choices for one extension, then show them (the
    /// search service notices the file within a few seconds).
    fn change_extension(&mut self, id: &str, change: impl FnOnce(&mut iliada_bundle::search::Choice)) {
        let Some(path) = iliada_bundle::search::Choices::default_path() else { return };
        let mut choices = iliada_bundle::search::Choices::load(&path);
        change(choices.extensions.entry(id.to_owned()).or_default());
        match choices.save(&path) {
            Ok(()) => self.load_permissions(),
            Err(e) => self.error = Some(fl!("search-setting-failed", error = e.to_string())),
        }
    }

    fn load_notify(&mut self) {
        self.notify = iliada_notify::settings::Settings::load_user();
        self.dnd_from = self.notify.do_not_disturb.from.clone();
        self.dnd_to = self.notify.do_not_disturb.to.clone();
    }

    /// Change Settings → Notifications and save it (the notification
    /// service reads the file for every notification).
    fn change_notify(&mut self, change: impl FnOnce(&mut iliada_notify::settings::Settings)) {
        if let Err(e) = iliada_notify::settings::Settings::update(change) {
            self.error = Some(fl!("save-failed", error = format!("{e:#}")));
        }
        let (from, to) = (self.dnd_from.clone(), self.dnd_to.clone());
        self.load_notify();
        // Times still being typed stay as typed.
        self.dnd_from = from;
        self.dnd_to = to;
    }

    fn notifications(&self) -> Element<'_, Message> {
        use iliada_notify::dnd::{self, Clock, Why};
        let space = cosmic::theme::spacing();
        let d = &self.notify.do_not_disturb;
        let now = dnd::status(d, &Clock::now());
        let state = match now {
            None => fl!("dnd-off"),
            Some(Why::Manual) => fl!("dnd-on"),
            Some(Why::Until { until }) => fl!("dnd-on-until", time = dnd::clock_time(until)),
            Some(Why::Schedule { until }) => fl!("dnd-on-schedule", time = dnd::clock_time(until)),
        };
        let mut dnd_section = settings::section()
            .title(fl!("dnd-section"))
            .add(info(fl!("dnd-section"), Some(fl!("dnd-desc"))))
            .add(settings::item::builder(fl!("dnd-now")).description(state).toggler(now.is_some(), Message::Dnd))
            .add(
                settings::item::builder(fl!("dnd-schedule"))
                    .description(fl!("dnd-schedule-desc", from = d.from.as_str(), to = d.to.as_str()))
                    .toggler(d.scheduled, Message::DndSchedule),
            );
        if d.scheduled {
            let valid = dnd::parse_time(&self.dnd_from).is_some() && dnd::parse_time(&self.dnd_to).is_some();
            let changed = self.dnd_from.trim() != d.from || self.dnd_to.trim() != d.to;
            let row = widget::row::with_capacity(5)
                .push(widget::text::body(fl!("dnd-from")))
                .push(widget::text_input("23:00", &self.dnd_from).on_input(Message::DndFrom).width(Length::Fixed(90.0)))
                .push(widget::text::body(fl!("dnd-to")))
                .push(widget::text_input("07:00", &self.dnd_to).on_input(Message::DndTo).width(Length::Fixed(90.0)))
                .push(
                    widget::button::standard(fl!("dnd-save-times"))
                        .on_press_maybe((valid && changed).then_some(Message::DndSaveTimes)),
                )
                .spacing(space.space_xs)
                .align_y(Alignment::Center);
            dnd_section = dnd_section.add(settings::item_row(vec![row.into()]));
            if !valid {
                dnd_section = dnd_section.add(info(fl!("dnd-times-hint"), None));
            }
        }
        dnd_section = dnd_section.add(
            settings::item::builder(fl!("dnd-critical"))
                .description(fl!("dnd-critical-desc"))
                .toggler(d.allow_critical, Message::DndCritical),
        );

        let homer = self.policy.homer_notifications();
        let selected = HOMER_NOTIFICATION_VALUES.iter().position(|v| *v == homer);
        let homer_section = settings::section().title(fl!("page-homer")).add(
            settings::item::builder(fl!("homer-notifications"))
                .description(fl!("homer-notifications-desc"))
                .control(widget::dropdown(&HOMER_NOTIFICATION_CHOICES[..], selected, Message::HomerNotifications)),
        );

        let mut apps_section =
            settings::section().title(fl!("notify-apps")).add(info(fl!("notify-apps"), Some(fl!("notify-apps-desc"))));
        if self.notify.apps.is_empty() {
            apps_section = apps_section.add(info(fl!("notify-apps-none"), None));
        }
        let mut apps: Vec<(&String, &iliada_notify::settings::AppRule)> = self.notify.apps.iter().collect();
        apps.sort_by_key(|(id, r)| if r.name.is_empty() { id.to_lowercase() } else { r.name.to_lowercase() });
        let mut col = widget::column::with_capacity(apps.len() + 3).spacing(space.space_m);
        col = col.push(dnd_section).push(homer_section).push(apps_section);
        for (id, rule) in apps {
            let name = if rule.name.is_empty() { id.clone() } else { rule.name.clone() };
            let switch = |s: AppSwitch| {
                let id = id.clone();
                move |on| Message::NotifyApp(id.clone(), s, on)
            };
            let mut app = settings::section().title(name).add(
                settings::item::builder(fl!("notify-allow"))
                    .description(id.clone())
                    .toggler(rule.allow, switch(AppSwitch::Allow)),
            );
            if rule.allow {
                app = app
                    .add(
                        settings::item::builder(fl!("notify-banners"))
                            .description(fl!("notify-banners-desc"))
                            .toggler(rule.banners, switch(AppSwitch::Banners)),
                    )
                    .add(settings::item::builder(fl!("notify-sound")).toggler(rule.sound, switch(AppSwitch::Sound)))
                    .add(
                        settings::item::builder(fl!("notify-history"))
                            .description(fl!("notify-history-desc"))
                            .toggler(rule.history, switch(AppSwitch::History)),
                    );
            }
            col = col.push(app);
        }
        col.into()
    }

    fn load_clipboard(&mut self) {
        let (enabled, exclude) = load_clipboard_settings();
        self.clip_enabled = enabled;
        self.clip_exclude = exclude;
    }

    /// The API key for the chosen model's provider: whether one is saved,
    /// whether it works, and adding, replacing or removing it.
    fn key_section(&self, model: &str) -> Element<'_, Message> {
        let space = cosmic::theme::spacing();
        let provider = Provider::of(model);
        let name = provider.name();
        let (prefix, site) = match provider {
            Provider::Anthropic => ("sk-ant-", "console.anthropic.com"),
            Provider::OpenRouter => ("sk-or-", "openrouter.ai/keys"),
        };
        let saved = iliada_assistant::api::load_key(provider);
        let mut section = settings::section().title(fl!("api-key"));
        match (&saved, self.replacing_key) {
            (Some(k), false) => {
                let status = match &self.key_check {
                    Some(KeyCheck::Checking) => fl!("checking-key"),
                    Some(KeyCheck::Works(s)) => s.clone(),
                    Some(KeyCheck::Failed(e)) => fl!("key-does-not-work", error = e.as_str()),
                    None => fl!("key-private"),
                };
                let buttons = widget::row::with_capacity(2)
                    .push(widget::button::standard(fl!("replace")).on_press(Message::ReplaceKey))
                    .push(widget::button::destructive(fl!("remove")).on_press(Message::RemoveKey))
                    .spacing(space.space_xs);
                section = section.add(
                    settings::item::builder(fl!(
                        "key-saved",
                        provider = name,
                        masked = iliada_assistant::api::masked(k)
                    ))
                    .description(status)
                    .control(buttons),
                );
            }
            _ => {
                let typed = self.key_input.trim();
                let hint = if typed.is_empty() {
                    fl!("key-create-at", site = site)
                } else if !typed.starts_with(prefix) {
                    match iliada_assistant::api::provider_of_key(typed) {
                        Some(other) => {
                            fl!("key-wrong-provider", other = other.name(), provider = name, prefix = prefix)
                        }
                        None => fl!("key-starts-with", provider = name, prefix = prefix),
                    }
                } else {
                    fl!("key-looks-right")
                };
                let valid = typed.starts_with(prefix) && typed.len() > prefix.len() + 8;
                let mut row = widget::row::with_capacity(3)
                    .push(
                        widget::secure_input(format!("{prefix}…"), self.key_input.as_str(), None, true)
                            .on_input(Message::AssistantKey)
                            .on_submit(move |_| if valid { Message::SaveKey } else { Message::Refresh })
                            .width(Length::Fill),
                    )
                    .push(widget::button::suggested(fl!("add-key")).on_press_maybe(valid.then_some(Message::SaveKey)));
                if saved.is_some() {
                    row = row.push(widget::button::standard(fl!("cancel")).on_press(Message::CancelReplaceKey));
                }
                section = section
                    .add(info(
                        if saved.is_some() {
                            fl!("replace-key", provider = name)
                        } else {
                            fl!("add-api-key", provider = name)
                        },
                        Some(hint),
                    ))
                    .add(settings::item_row(vec![row.spacing(space.space_xs).align_y(Alignment::Center).into()]));
            }
        }
        let spent = iliada_assistant::total_spent();
        if spent > 0.0 {
            section = section.add(info(fl!("spent-so-far", amount = format!("${spent:.4}")), None));
        }
        section.into()
    }

    /// Checks the saved key for `model`'s provider (free: no model is asked).
    fn check_key(&mut self, model: &str) -> Task<Message> {
        let provider = Provider::of(model);
        let Some(key) = iliada_assistant::api::load_key(provider) else {
            self.key_check = None;
            return Task::none();
        };
        self.key_check = Some(KeyCheck::Checking);
        cosmic::task::future(async move {
            Message::KeyChecked(iliada_assistant::api::check_key(provider, &key).await.map_err(|e| format!("{e:#}")))
        })
    }

    /// The account the assistant sends email from (per user, like the key).
    fn email_section(&self) -> Element<'_, Message> {
        let space = cosmic::theme::spacing().space_xs;
        let saved = iliada_assistant::email::load();
        let mut section = settings::section().title(fl!("email")).add(info(
            match &saved {
                Some(a) => fl!("homer-sends-as", address = a.address.as_str()),
                None => fl!("let-homer-email"),
            },
            Some(fl!("email-desc")),
        ));
        let field = |label: String, input: Element<'static, Message>| -> Element<'static, Message> {
            widget::row::with_capacity(2)
                .push(widget::text::body(label).width(Length::Fixed(140.0)))
                .push(input)
                .spacing(space)
                .align_y(Alignment::Center)
                .into()
        };
        section = section
            .add(settings::item_row(vec![field(
                fl!("email-address"),
                widget::text_input("you@example.com", self.email.address.clone())
                    .on_input(Message::EmailAddress)
                    .width(Length::Fill)
                    .into(),
            )]))
            .add(settings::item_row(vec![field(
                fl!("outgoing-server"),
                widget::text_input("smtp.example.com", self.email.server.clone())
                    .on_input(Message::EmailServer)
                    .width(Length::Fill)
                    .into(),
            )]))
            .add(settings::item_row(vec![field(
                fl!("connection"),
                widget::dropdown(
                    &SECURITY_NAMES[..],
                    Some(security_index(self.email.security)),
                    Message::EmailSecurity,
                )
                .into(),
            )]))
            .add(settings::item_row(vec![field(
                fl!("password"),
                widget::secure_input(
                    if iliada_assistant::email::has_password() { fl!("password-saved") } else { fl!("password") },
                    self.email.password.clone(),
                    None,
                    true,
                )
                .on_input(Message::EmailPassword)
                .on_submit(|_| Message::SaveEmail)
                .width(Length::Fill)
                .into(),
            )]));
        let mut buttons = widget::row::with_capacity(2).spacing(space);
        if saved.is_some() {
            buttons = buttons.push(widget::button::standard(fl!("remove")).on_press(Message::RemoveEmail));
        }
        buttons = buttons.push(widget::button::suggested(fl!("save")).on_press(Message::SaveEmail));
        section.add(settings::item_row(vec![widget::space::horizontal().into(), buttons.into()])).into()
    }

    fn about(&self, s: &Status) -> Element<'_, Message> {
        let section = settings::section()
            .title(fl!("this-computer"))
            .add(settings::item(fl!("name"), widget::text::body(s.hostname.clone())))
            .add(settings::item(fl!("iliada"), widget::text::body(s.version.clone())))
            .add(settings::item(fl!("configuration"), widget::text::body(fl!("configuration-value"))));
        let (description, control): (String, Element<'_, Message>) = match &self.report {
            None => {
                (fl!("report-desc"), widget::button::standard(fl!("save-report")).on_press(Message::SaveReport).into())
            }
            Some(None) => (fl!("looking-at-computer"), widget::button::standard(fl!("save-report")).into()),
            Some(Some(Ok(path))) => (
                fl!("report-saved", path = path.as_str()),
                widget::button::standard(fl!("open")).on_press(Message::OpenReport).into(),
            ),
            Some(Some(Err(e))) => (
                fl!("report-failed", error = e.as_str()),
                widget::button::standard(fl!("try-again")).on_press(Message::SaveReport).into(),
            ),
        };
        let help = settings::section()
            .title(fl!("help"))
            .add(settings::item::builder(fl!("hardware-report")).description(description).control(control));
        settings::view_column(vec![section.into(), help.into()]).into()
    }
}

impl Panel {
    /// The computer's language: English or Ελληνικά, each in its own words.
    fn language(&self, s: &Status) -> Element<'_, Message> {
        let ok = self.can_change();
        let languages = &iliada_system::settings::LANGUAGES;
        let selected = languages.iter().position(|(l, _)| *l == s.language);
        let choose: Element<'_, Message> = if ok {
            widget::dropdown(&LANGUAGE_NAMES[..], selected, move |i| {
                Message::Run(Request::SetLanguage { locale: languages[i].0.to_owned() })
            })
            .into()
        } else {
            widget::text::body(language_name(&s.language)).into()
        };
        let mut section = settings::section()
            .title(fl!("page-language"))
            .add(settings::item::builder(fl!("language-title")).description(fl!("language-desc")).control(choose));
        // Chosen, but this session started in another one.
        if !session_has_language(&s.language) {
            section = section.add(
                settings::item::builder(fl!("language-pending", language = language_name(&s.language)))
                    .control(widget::button::standard(fl!("log-out")).on_press(Message::LogOut)),
            );
        }
        let keyboard = settings::section().title(fl!("keyboard-layouts")).add(
            settings::item::builder(fl!("keyboard-layouts"))
                .description(fl!("keyboard-layouts-desc"))
                .control(widget::button::standard(fl!("keyboard-settings")).on_press(Message::OpenKeyboard)),
        );
        settings::view_column(vec![section.into(), keyboard.into()]).into()
    }
}

/// A settings row that only informs.
fn info<'a>(title: impl Into<String>, description: Option<String>) -> Element<'a, Message> {
    let mut col = widget::column::with_capacity(2).push(widget::text::body(title.into()));
    if let Some(d) = description {
        col = col.push(widget::text::caption(d));
    }
    settings::item_row(vec![col.spacing(2).width(Length::Fill).into()]).into()
}

fn github_prefix(account: &str) -> String {
    format!("github.com/{account}/{}", iliada_system::github::PREFIX)
}

fn card(content: Element<'_, Message>) -> Element<'_, Message> {
    widget::container(content)
        .padding(cosmic::theme::spacing().space_s)
        .class(cosmic::theme::Container::Card)
        .width(Length::Fill)
        .into()
}

impl Panel {
    /// The panel showing `page`. Nothing is loaded until it is entered.
    pub fn new(page: Page) -> Self {
        Panel {
            page,
            status: None,
            job: None,
            sign_in: None,
            error: None,
            notice: None,
            tool_input: String::new(),
            backup_name: String::new(),
            backups: None,
            confirm: None,
            programs: Vec::new(),
            policy: Policy::load(),
            privacy_by: PrivacyBy::default(),
            extra_isolation: Vec::new(),
            key_input: String::new(),
            replacing_key: false,
            key_check: None,
            report: None,
            email: EmailForm::load(),
            account: Account::default(),
            keep_history: iliada_assistant::history::history_enabled(),
            confirm_clear_history: false,
            clip_enabled: true,
            clip_exclude: Vec::new(),
            confirm_clear_clipboard: false,
            show_windows: true,
            keyword_edit: None,
            notify: Default::default(),
            dnd_from: String::new(),
            dnd_to: String::new(),
        }
    }

    /// Which page this panel shows.
    pub fn page(&self) -> Page {
        self.page
    }

    /// The person opened `page` (again): fresh state for it.
    pub fn enter(&mut self, page: Page) -> Task<Message> {
        self.page = page;
        // "Iliada is up to date" belongs to the page it came from. Something
        // still running keeps showing.
        self.error = None;
        self.notice = None;
        if self.page() == Page::Assistant {
            self.replacing_key = false;
            let model = self.status().map(|s| s.assistant.model.clone()).unwrap_or_default();
            return Task::batch([load_status(), self.check_key(&model)]);
        }
        if self.page() == Page::Account {
            return Task::batch([load_status(), self.account.refresh().map(account_action)]);
        }
        if self.page() == Page::Notifications {
            self.load_notify();
            self.policy = Policy::load();
        }
        if self.page() == Page::Privacy {
            self.extra_isolation = load_extra_isolation();
            self.load_permissions();
            self.load_clipboard();
            self.show_windows = load_search_settings().windows;
        }
        load_status()
    }

    pub fn dialog(&self) -> Option<Element<'_, Message>> {
        let (title, body, action) = match self.confirm.as_ref()? {
            Confirm::Restore(repo) => (
                fl!("restore-backup-q"),
                fl!("restore-backup-body", repo = repo.as_str()),
                widget::button::destructive(fl!("restore")),
            ),
            Confirm::ResetPermissions => {
                (fl!("reset-permissions-q"), fl!("reset-permissions-body"), widget::button::destructive(fl!("reset")))
            }
            Confirm::RemoveKey(p) => (
                fl!("remove-key-q"),
                fl!("remove-key-body", provider = p.name()),
                widget::button::destructive(fl!("remove")),
            ),
            Confirm::AccountRestore(from) => (
                fl!("restore-settings-q"),
                fl!("restore-settings-body", from = from.as_str()),
                widget::button::destructive(fl!("restore")),
            ),
            Confirm::AccountSignOut => {
                (fl!("account-sign-out-q"), fl!("account-sign-out-body"), widget::button::destructive(fl!("sign-out")))
            }
            Confirm::TestingUpdates => {
                (fl!("testing-q"), fl!("testing-body"), widget::button::suggested(fl!("get-test-updates")))
            }
            Confirm::SignOut => {
                (fl!("github-sign-out-q"), fl!("github-sign-out-body"), widget::button::destructive(fl!("sign-out")))
            }
        };
        Some(
            widget::dialog()
                .title(title)
                .body(body)
                .primary_action(action.on_press(Message::Confirmed))
                .secondary_action(widget::button::standard(fl!("cancel")).on_press(Message::Cancel))
                .into(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => return load_status(),
            Message::Account(account::Msg::Restore) => {
                let from = self.account.latest().map(|m| m.computer.clone()).unwrap_or_default();
                self.confirm = Some(Confirm::AccountRestore(from));
            }
            Message::Account(account::Msg::SignOut) => self.confirm = Some(Confirm::AccountSignOut),
            Message::Account(m) => return self.account.update(m).map(account_action),
            Message::Status(s) => {
                if let (Ok(st), true) = (&s, self.backup_name.is_empty()) {
                    self.backup_name = st.hostname.clone();
                }
                self.status = Some(s);
            }
            Message::Run(Request::BackupEnable { name }) if name.is_empty() => {
                let name = self.backup_name.trim().to_owned();
                if iliada_system::github::valid_name(&name) && self.can_change() {
                    return self.start(Request::BackupEnable { name });
                }
            }
            Message::Run(request) => return self.start(request),
            Message::Progress(Progress::Event(e)) => match e {
                Event::Log { line } => {
                    if let Some(job) = &mut self.job {
                        job.line = line.chars().take(120).collect();
                    }
                }
                Event::SignInCode { code, url } => {
                    open_url(&url);
                    self.sign_in = Some((code, url));
                }
                _ => {}
            },
            Message::Progress(Progress::Finished(result)) => {
                let job = self.job.take();
                self.sign_in = None;
                match (job, result) {
                    (Some(job), Ok(v)) => {
                        self.notice = success(&job.request, &v);
                        if matches!(job.request, Request::AddTools { .. }) {
                            self.tool_input.clear();
                        }
                        if matches!(job.request, Request::BackupSignIn) {
                            return Task::batch([load_status(), load_backups()]);
                        }
                        if matches!(job.request, Request::BackupSignOut) {
                            self.backups = None;
                        }
                    }
                    (_, Err(e)) => self.error = Some(e),
                    (None, Ok(_)) => {}
                }
                return load_status();
            }
            Message::ToolInput(s) => self.tool_input = s,
            Message::AddTool => {
                let name = self.tool_input.trim().to_owned();
                if iliada_system::settings::valid_tool_name(&name) && self.can_change() {
                    return self.start(Request::AddTools { packages: vec![name] });
                }
            }
            Message::BackupName(s) => self.backup_name = s,
            Message::OpenSignIn => {
                if let Some((_, url)) = &self.sign_in {
                    open_url(url);
                }
            }
            Message::LoadBackups => {
                self.backups = None;
                return load_backups();
            }
            Message::Backups(b) => self.backups = Some(b),
            Message::Ask(c) => self.confirm = Some(c),
            Message::AssistantSet(a) => {
                // Another provider's model: check that provider's key.
                let before = self.status().map(|s| Provider::of(&s.assistant.model));
                let check = if before != Some(Provider::of(&a.model)) {
                    self.replacing_key = false;
                    self.check_key(&a.model)
                } else {
                    Task::none()
                };
                return Task::batch([self.start(Request::SetAssistant { settings: a }), check]);
            }
            Message::AssistantKey(k) => self.key_input = k,
            Message::EmailAddress(a) => {
                // Known providers: fill in their server (until it's edited).
                if self.email.server.is_empty() || self.email.suggested {
                    match EmailAccount::suggested(&a) {
                        Some(s) => {
                            self.email.server = s.server;
                            self.email.security = s.security;
                            self.email.suggested = true;
                        }
                        None if self.email.suggested => {
                            self.email.server.clear();
                            self.email.suggested = false;
                        }
                        None => {}
                    }
                }
                self.email.address = a;
            }
            Message::EmailServer(s) => {
                self.email.server = s;
                self.email.suggested = false;
            }
            Message::EmailSecurity(i) => {
                self.email.security = if i == 1 { Security::Starttls } else { Security::Tls };
            }
            Message::EmailPassword(p) => self.email.password = p,
            Message::SaveEmail => {
                let account = EmailAccount {
                    address: self.email.address.trim().to_owned(),
                    server: self.email.server.trim().to_owned(),
                    security: self.email.security,
                    ..Default::default()
                };
                let password = (!self.email.password.is_empty()).then_some(self.email.password.as_str());
                if password.is_none() && !iliada_assistant::email::has_password() {
                    self.error = Some(fl!("email-password-first"));
                } else {
                    match iliada_assistant::email::save(&account, password) {
                        Err(e) => self.error = Some(fl!("email-save-failed", error = format!("{e:#}"))),
                        Ok(()) => {
                            self.email.password.clear();
                            self.notice = Some(fl!("email-saved", address = account.address.as_str()));
                        }
                    }
                }
            }
            Message::RemoveEmail => match iliada_assistant::email::remove() {
                Err(e) => self.error = Some(fl!("email-remove-failed", error = format!("{e:#}"))),
                Ok(()) => {
                    self.email = EmailForm::default();
                    self.notice = Some(fl!("email-removed"));
                }
            },
            Message::SaveKey => {
                let k = self.key_input.trim().to_owned();
                match iliada_assistant::api::save_key(&k) {
                    Err(e) => self.error = Some(fl!("key-save-failed", error = format!("{e:#}"))),
                    Ok(p) => {
                        self.key_input.clear();
                        self.replacing_key = false;
                        self.notice = Some(fl!("key-added", provider = p.name()));
                        let model = self.status().map(|s| s.assistant.model.clone()).unwrap_or_default();
                        return self.check_key(&model);
                    }
                }
            }
            Message::ReplaceKey => {
                self.replacing_key = true;
                self.key_input.clear();
            }
            Message::CancelReplaceKey => {
                self.replacing_key = false;
                self.key_input.clear();
            }
            Message::RemoveKey => {
                let model = self.status().map(|s| s.assistant.model.clone()).unwrap_or_default();
                self.confirm = Some(Confirm::RemoveKey(Provider::of(&model)));
            }
            Message::KeyChecked(r) => {
                self.key_check = Some(match r {
                    Ok(s) => KeyCheck::Works(s),
                    Err(e) => KeyCheck::Failed(e),
                });
            }
            Message::ExtraIsolation(app, on) => {
                self.extra_isolation.retain(|a| *a != app);
                if on {
                    self.extra_isolation.push(app);
                }
                if let Err(e) = save_extra_isolation(&self.extra_isolation) {
                    self.error = Some(fl!("save-failed", error = e.to_string()));
                }
                // The tier changes what an app can be given; say so in the file.
                self.load_permissions();
            }
            Message::HomerApps(on) => {
                if !self.policy.homer_forced_off() {
                    let programs = self.programs.clone();
                    if let Err(e) = self.policy.edit(&programs, |p| {
                        p.set_homer_enabled(on);
                        Ok(())
                    }) {
                        self.error = Some(fl!("save-failed", error = e.to_string()));
                    }
                }
            }
            Message::Permission(app, key, value) => {
                if let Some(f) = self.programs.iter().find(|a| a.id == app).cloned() {
                    let programs = self.programs.clone();
                    if let Err(e) = self.policy.edit(&programs, |p| p.set(&f, key, value)) {
                        self.error = Some(fl!("save-failed", error = e.to_string()));
                    }
                }
            }
            Message::PrivacyBy(by) => self.privacy_by = by,
            Message::OpenPermissionsFile => {
                if let Some(p) = self.policy.user_path.clone() {
                    if !p.exists() {
                        let programs = self.programs.clone();
                        let _ = self.policy.edit(&programs, |_| Ok(()));
                    }
                    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
                }
            }
            Message::Cancel => {
                self.confirm = None;
                self.confirm_clear_history = false;
                self.confirm_clear_clipboard = false;
            }
            Message::Confirmed => {
                return match self.confirm.take() {
                    Some(Confirm::Restore(repo)) => self.start(Request::Restore { repo }),
                    Some(Confirm::SignOut) => self.start(Request::BackupSignOut),
                    Some(Confirm::TestingUpdates) => self.start(Request::SetTestingUpdates { on: true }),
                    Some(Confirm::ResetPermissions) => {
                        if let Some(path) = self.policy.user_path.clone()
                            && self.policy.user.broken
                        {
                            let _ = std::fs::copy(&path, path.with_extension("toml.broken"));
                        }
                        let programs = self.programs.clone();
                        match self.policy.edit(&programs, |p| {
                            p.reset(None);
                            Ok(())
                        }) {
                            Ok(()) => self.notice = Some(fl!("permissions-reset")),
                            Err(e) => self.error = Some(fl!("permissions-reset-failed", error = e.to_string())),
                        }
                        Task::none()
                    }
                    Some(Confirm::AccountRestore(_)) => self.account.update(account::Msg::Restore).map(account_action),
                    Some(Confirm::AccountSignOut) => self.account.update(account::Msg::SignOut).map(account_action),
                    Some(Confirm::RemoveKey(p)) => {
                        match iliada_assistant::api::remove_key(p) {
                            Err(e) => self.error = Some(fl!("key-remove-failed", error = format!("{e:#}"))),
                            Ok(()) => {
                                self.key_check = None;
                                self.notice = Some(fl!("key-removed", provider = p.name()));
                            }
                        }
                        Task::none()
                    }
                    None => Task::none(),
                };
            }
            Message::Dismiss => {
                self.error = None;
                self.notice = None;
            }
            Message::SaveReport => {
                self.report = Some(None);
                return cosmic::task::future(async {
                    let out = tokio::process::Command::new("iliada")
                        .arg("report")
                        .stdin(std::process::Stdio::null())
                        .output()
                        .await;
                    Message::ReportSaved(match out {
                        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned()),
                        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_owned()),
                        Err(e) => Err(e.to_string()),
                    })
                });
            }
            Message::ReportSaved(r) => self.report = Some(Some(r)),
            Message::OpenReport => {
                if let Some(Some(Ok(path))) = &self.report {
                    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
                }
            }
            Message::ClipHistory(on) => match set_clipboard_enabled(on) {
                Ok(()) => self.clip_enabled = on,
                Err(e) => self.error = Some(fl!("clipboard-history-setting-failed", error = format!("{e:#}"))),
            },
            Message::ClipExclude(app, on) => match set_clipboard_excluded(&app, on) {
                Ok(()) => {
                    if on {
                        if !self.clip_exclude.iter().any(|e| clip_excludes(e, &app)) {
                            self.clip_exclude.push(app);
                        }
                    } else {
                        self.clip_exclude.retain(|e| !clip_excludes(e, &app));
                    }
                }
                Err(e) => self.error = Some(fl!("clipboard-history-setting-failed", error = format!("{e:#}"))),
            },
            Message::ExtensionOn(id, on) => self.change_extension(&id, |c| c.enabled = on),
            Message::ChangeKeyword(id) => {
                let current = self.programs.iter().find(|a| a.id == id).and_then(|a| a.search.as_ref());
                let text = current.map(|s| s.keyword.clone()).unwrap_or_default();
                self.keyword_edit = Some((id, text));
            }
            Message::KeywordInput(text) => {
                if let Some((_, t)) = self.keyword_edit.as_mut() {
                    *t = text;
                }
            }
            Message::CancelKeyword => self.keyword_edit = None,
            Message::SaveKeyword => {
                if let Some((id, text)) = self.keyword_edit.clone() {
                    let keyword = text.trim().to_owned();
                    if keyword_problem(&self.programs, &id, &keyword).is_none() {
                        self.keyword_edit = None;
                        self.change_extension(&id, |c| c.keyword = Some(keyword));
                    }
                }
            }
            Message::Dnd(on) => {
                if let Err(e) = iliada_notify::dnd::switch(on, None) {
                    self.error = Some(fl!("save-failed", error = format!("{e:#}")));
                }
                self.load_notify();
            }
            Message::DndSchedule(on) => self.change_notify(|s| {
                s.do_not_disturb.scheduled = on;
                s.do_not_disturb.paused_until = None;
            }),
            Message::DndFrom(t) => self.dnd_from = t,
            Message::DndTo(t) => self.dnd_to = t,
            Message::DndSaveTimes => {
                let (from, to) = (self.dnd_from.trim().to_owned(), self.dnd_to.trim().to_owned());
                if let (Some(f), Some(t)) = (iliada_notify::dnd::parse_time(&from), iliada_notify::dnd::parse_time(&to))
                {
                    let (from, to) = (hhmm(f), hhmm(t));
                    self.change_notify(|s| {
                        s.do_not_disturb.from = from;
                        s.do_not_disturb.to = to;
                        s.do_not_disturb.paused_until = None;
                    });
                }
            }
            Message::DndCritical(on) => self.change_notify(|s| s.do_not_disturb.allow_critical = on),
            Message::HomerNotifications(i) => {
                let value = HOMER_NOTIFICATION_VALUES[i.min(HOMER_NOTIFICATION_VALUES.len() - 1)];
                let programs = policy::installed_programs();
                if let Err(e) = self.policy.edit(&programs, |p| {
                    p.set_homer_notifications(value);
                    Ok(())
                }) {
                    self.error = Some(fl!("save-failed", error = e.to_string()));
                }
            }
            Message::NotifyApp(app, switch, on) => self.change_notify(|s| {
                let rule = s.apps.entry(app).or_default();
                match switch {
                    AppSwitch::Allow => rule.allow = on,
                    AppSwitch::Banners => rule.banners = on,
                    AppSwitch::Sound => rule.sound = on,
                    AppSwitch::History => rule.history = on,
                }
            }),
            Message::ShowWindows(on) => match set_show_windows(on) {
                Ok(()) => self.show_windows = on,
                Err(e) => self.error = Some(fl!("search-setting-failed", error = format!("{e:#}"))),
            },
            Message::ClipClear => self.confirm_clear_clipboard = true,
            Message::ClipClearConfirmed => {
                self.confirm_clear_clipboard = false;
                match clear_clipboard_history() {
                    Ok(()) => self.notice = Some(fl!("clipboard-history-cleared")),
                    Err(e) => self.error = Some(fl!("clipboard-history-clear-failed", error = format!("{e:#}"))),
                }
            }
            Message::KeepHistory(on) => match iliada_assistant::history::set_history_enabled(on) {
                Ok(()) => self.keep_history = on,
                Err(e) => self.error = Some(fl!("history-setting-failed", error = format!("{e:#}"))),
            },
            Message::ClearHistory => self.confirm_clear_history = true,
            Message::ClearHistoryConfirmed => {
                self.confirm_clear_history = false;
                match iliada_assistant::history::ConversationStore::open().clear() {
                    Ok(0) => self.notice = Some(fl!("history-already-empty")),
                    Ok(n) => self.notice = Some(fl!("history-cleared", n = n)),
                    Err(e) => self.error = Some(fl!("history-clear-failed", error = format!("{e:#}"))),
                }
            }
            Message::LogOut => {
                let _ = std::process::Command::new("cosmic-osd").arg("log-out").spawn();
            }
            Message::OpenKeyboard => {
                let _ = std::process::Command::new("cosmic-settings").arg("keyboard").spawn();
            }
            Message::EditVoice => match iliada_assistant::voice_path() {
                Some(path) => {
                    if !path.exists() {
                        let made = path
                            .parent()
                            .map_or(Ok(()), std::fs::create_dir_all)
                            .and_then(|()| std::fs::write(&path, iliada_assistant::DEFAULT_VOICE));
                        if let Err(e) = made {
                            self.error = Some(fl!("voice-file-failed", error = e.to_string()));
                            return Task::none();
                        }
                    }
                    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                }
                None => self.error = Some(fl!("no-home-for-voice")),
            },
        }
        Task::none()
    }

    /// The page in its own scrolling area (the standalone window).
    pub fn view(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        widget::container(
            widget::scrollable(widget::container(self.content()).width(Length::Fill).align_x(Alignment::Center))
                .height(Length::Fill),
        )
        .padding(spacing.space_m)
        .into()
    }

    /// The page's content, for a window that already scrolls (COSMIC
    /// Settings shows Iliada's pages among its own this way).
    pub fn content(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        match &self.status {
            None => widget::text::body(fl!("loading")).into(),
            Some(Err(e)) => widget::column::with_capacity(3)
                .push(widget::text::title4(fl!("settings-unavailable")))
                .push(widget::text::body(e.as_str()))
                .push(widget::button::standard(fl!("try-again")).on_press(Message::Refresh))
                .spacing(spacing.space_s)
                .into(),
            Some(Ok(s)) => {
                let mut col = widget::column::with_capacity(4).spacing(spacing.space_m);
                if let Some(b) = self.banner() {
                    col = col.push(b);
                }
                if matches!(self.page(), Page::Account | Page::Notifications) {
                    // Each person's own: no administrator needed.
                } else if !s.admin {
                    col = col.push(card(widget::text::body(fl!("admin-only")).into()));
                } else if let (Some(busy), None) = (&s.busy, &self.job) {
                    col = col
                        .push(card(widget::text::body(fl!("iliada-busy", what = crate::i18n::activity(busy))).into()));
                }
                col = col.push(match self.page() {
                    Page::Account => self.account.view().map(Message::Account),
                    Page::Updates => self.updates(s),
                    Page::Backup => self.backup(s),
                    Page::Tools => self.tools(s),
                    Page::Services => self.services(s),
                    Page::Privacy => self.privacy(),
                    Page::Notifications => self.notifications(),
                    Page::Assistant => self.assistant(s),
                    Page::Language => self.language(s),
                    Page::About => self.about(s),
                });
                col.max_width(760).into()
            }
        }
    }
}

/// Minutes since midnight as "HH:MM".
fn hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn account_action(m: account::Msg) -> cosmic::Action<Message> {
    cosmic::Action::App(Message::Account(m))
}

/// What `iliada-settings` hands to `cosmic-settings` so a page actually opens.
/// A single argument `iliada-updates` is refused (0.2.6).
pub fn cosmic_settings_args(page: Page) -> [&'static str; 2] {
    ["iliada", page.id()]
}

fn clip_excludes(listed: &str, app: &str) -> bool {
    iliada_search::clipboard::Config { enabled: true, exclude: vec![listed.to_owned()] }.excludes(app)
}

fn clipboard_config_path() -> Option<std::path::PathBuf> {
    iliada_search::config_dir().map(|d| d.join("clipboard.toml"))
}

fn load_clipboard_settings() -> (bool, Vec<String>) {
    use iliada_search::proto::{Client, Request, Response};
    if let Ok(mut c) = Client::connect()
        && let Ok(Response::ClipboardConfig { enabled, exclude }) = c.call(&Request::ClipboardConfig)
    {
        return (enabled, exclude);
    }
    let cfg = clipboard_config_path().map(|p| iliada_search::clipboard::Config::load(&p)).unwrap_or_default();
    (cfg.enabled, cfg.exclude)
}

fn set_clipboard_enabled(on: bool) -> anyhow::Result<()> {
    use iliada_search::proto::{Client, Request, Response};
    if let Ok(mut c) = Client::connect() {
        match c.call(&Request::ClipboardSetEnabled { enabled: on })? {
            Response::Error { message } => anyhow::bail!(message),
            _ => return Ok(()),
        }
    }
    let path = clipboard_config_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let mut cfg = iliada_search::clipboard::Config::load(&path);
    cfg.enabled = on;
    cfg.save(&path)
}

fn set_clipboard_excluded(app: &str, exclude: bool) -> anyhow::Result<()> {
    use iliada_search::proto::{Client, Request, Response};
    if let Ok(mut c) = Client::connect() {
        match c.call(&Request::ClipboardExclude { app: app.to_owned(), exclude })? {
            Response::Error { message } => anyhow::bail!(message),
            _ => return Ok(()),
        }
    }
    let path = clipboard_config_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let mut cfg = iliada_search::clipboard::Config::load(&path);
    if exclude {
        if !cfg.excludes(app) {
            cfg.exclude.push(app.to_owned());
        }
    } else {
        cfg.exclude.retain(|e| !clip_excludes(e, app));
    }
    cfg.save(&path)
}

/// Why `keyword` can't be extension `id`'s, in words; `None` when it can.
fn keyword_problem(programs: &[AppFacts], id: &str, keyword: &str) -> Option<String> {
    use iliada_bundle::search::{KeywordError, keyword_free};
    let all: Vec<iliada_bundle::search::Extension> = programs.iter().filter_map(|a| a.search.clone()).collect();
    match keyword_free(&all, id, keyword) {
        Ok(()) => None,
        Err(KeywordError::Invalid) => Some(fl!("search-keyword-invalid")),
        Err(KeywordError::Reserved) => Some(fl!("search-keyword-reserved", keyword = keyword)),
        Err(KeywordError::Taken(by)) => Some(fl!("search-keyword-taken", keyword = keyword, other = by)),
    }
}

fn load_search_settings() -> iliada_search::windows::Config {
    iliada_search::windows::Config::default_path().map(|p| iliada_search::windows::Config::load(&p)).unwrap_or_default()
}

/// Through the search service when it runs (it applies the change at once
/// and saves it), else straight to the file it reads when it starts.
fn set_show_windows(on: bool) -> anyhow::Result<()> {
    use iliada_search::proto::{Client, Request, Response};
    if let Ok(mut c) = Client::connect() {
        match c.call(&Request::ShowWindows { enabled: on })? {
            Response::Error { message } => anyhow::bail!(message),
            _ => return Ok(()),
        }
    }
    let path = iliada_search::windows::Config::default_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let mut cfg = iliada_search::windows::Config::load(&path);
    cfg.windows = on;
    cfg.save(&path)
}

fn clear_clipboard_history() -> anyhow::Result<()> {
    use iliada_search::proto::{Client, Request, Response};
    if let Ok(mut c) = Client::connect() {
        match c.call(&Request::ClipboardClear)? {
            Response::Error { message } => anyhow::bail!(message),
            _ => return Ok(()),
        }
    }
    if let Some((dir, cfg)) = iliada_search::clipboard::Store::default_paths() {
        let mut store = iliada_search::clipboard::Store::open(dir, cfg)?;
        store.clear();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosmic_settings_takes_iliada_then_the_page() {
        assert_eq!(cosmic_settings_args(Page::Updates), ["iliada", "updates"]);
        assert_eq!(cosmic_settings_args(Page::Assistant), ["iliada", "homer"]);
        for p in Page::ALL {
            let [group, id] = cosmic_settings_args(p);
            assert_eq!(group, "iliada");
            assert_eq!(id, p.id());
            assert_ne!(format!("iliada-{id}").as_str(), "iliada", "0.2.6 passed one glued argument");
        }
    }
}
