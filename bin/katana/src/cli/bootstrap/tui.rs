//! Full-screen ratatui-based TUI for `katana bootstrap --interactive`.
//!
//! ## Architecture
//!
//! The event loop runs on a `spawn_blocking` thread because crossterm's `event::poll` /
//! `event::read` are synchronous and would block the async runtime if called directly.
//! The bootstrap executor itself runs on the regular tokio runtime via `tokio::spawn`,
//! and streams progress to the UI thread through a `tokio::sync::mpsc::UnboundedSender`
//! that we drain non-blockingly each tick.
//!
//! Visual layout:
//!
//! ```text
//! ┌────────────────────────────────────────────────────┐
//! │  Classes │ Contracts │ Settings │ Execute          │ <- top tab bar
//! ├────────────────────────────────────────────────────┤
//! │                                                    │
//! │              tab-specific content                  │
//! │                                                    │
//! ├────────────────────────────────────────────────────┤
//! │  a add  d delete  Tab next  q quit  …              │ <- bottom hint bar
//! └────────────────────────────────────────────────────┘
//! ```
//!
//! Modals (add class, add contract, save manifest, …) render as centered overlays on
//! top of the tab content via `Clear` + a centered `Block`.

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use katana_primitives::class::ContractClass;
use katana_primitives::{ContractAddress, Felt};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};
use ratatui::Terminal;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::task::JoinHandle;
use url::Url;

use super::embedded::{self, EmbeddedClass};
use super::executor::{
    execute_with_progress, BootstrapEvent, BootstrapReport, ExecutorConfig,
};
use super::manifest::{ClassEntry, ContractEntry, Manifest};
use super::plan::{BootstrapPlan, ClassSource, DeclareStep, DeployStep};

// =============================================================================
// Public entry point
// =============================================================================

/// CLI-supplied defaults that prefill the Settings tab.
#[derive(Debug, Clone, Default)]
pub struct SignerDefaults {
    pub rpc_url: Option<String>,
    pub account: Option<ContractAddress>,
    pub private_key: Option<Felt>,
    pub skip_existing: bool,
}

/// Run the interactive TUI. Blocks (off the async runtime via `spawn_blocking`) until
/// the user exits. Any unsaved plan is dropped on exit; the only persistence is the
/// optional "save manifest" prompt offered after a successful execution.
pub async fn run(initial: Option<Manifest>, defaults: SignerDefaults) -> Result<()> {
    // Capture a runtime handle so the blocking event-loop thread can still spawn the
    // executor task back onto the multi-thread tokio runtime.
    let runtime = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || run_blocking(initial, defaults, runtime))
        .await
        .map_err(|e| anyhow!("TUI thread panicked: {e}"))?
}

fn run_blocking(
    initial: Option<Manifest>,
    defaults: SignerDefaults,
    runtime: tokio::runtime::Handle,
) -> Result<()> {
    let mut app = AppState::new(defaults);
    if let Some(manifest) = initial {
        app.load_manifest(&manifest)?;
    }

    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    event_loop(&mut terminal, &mut app, &runtime)
}

// =============================================================================
// Terminal RAII guard — restores the terminal even on panic
// =============================================================================

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        io::stdout().execute(EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = io::stdout().execute(LeaveAlternateScreen);
    }
}

// =============================================================================
// App state
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Classes,
    Contracts,
    Settings,
    Execute,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Classes, Tab::Contracts, Tab::Settings, Tab::Execute];

    fn idx(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap()
    }

    fn next(self) -> Tab {
        Self::ALL[(self.idx() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Tab {
        Self::ALL[(self.idx() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    fn title(self) -> &'static str {
        match self {
            Tab::Classes => "Classes",
            Tab::Contracts => "Contracts",
            Tab::Settings => "Settings",
            Tab::Execute => "Execute",
        }
    }
}

struct AppState {
    current_tab: Tab,
    classes: Vec<DeclareStep>,
    classes_state: ListState,
    contracts: Vec<DeployStep>,
    contracts_state: ListState,
    settings: SettingsForm,
    modal: Option<Modal>,
    execution: ExecutionState,
    quit: bool,
    /// Transient banner (e.g. validation errors) shown in the bottom hint bar.
    flash: Option<String>,
}

impl AppState {
    fn new(defaults: SignerDefaults) -> Self {
        Self {
            current_tab: Tab::Classes,
            classes: Vec::new(),
            classes_state: ListState::default(),
            contracts: Vec::new(),
            contracts_state: ListState::default(),
            settings: SettingsForm::from_defaults(defaults),
            modal: None,
            execution: ExecutionState::Idle,
            quit: false,
            flash: None,
        }
    }

    fn load_manifest(&mut self, manifest: &Manifest) -> Result<()> {
        // Reuse the existing manifest → plan resolver so we get the same validation,
        // file IO, and class-hash computation as programmatic mode.
        let plan = BootstrapPlan::from_manifest(manifest)?;
        self.classes = plan.declares;
        self.contracts = plan.deploys;
        if !self.classes.is_empty() {
            self.classes_state.select(Some(0));
        }
        if !self.contracts.is_empty() {
            self.contracts_state.select(Some(0));
        }
        Ok(())
    }

    fn flash<S: Into<String>>(&mut self, msg: S) {
        self.flash = Some(msg.into());
    }
}

// -----------------------------------------------------------------------------
// Settings form
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsField {
    RpcUrl,
    Account,
    PrivateKey,
    SkipExisting,
}

impl SettingsField {
    const ALL: [SettingsField; 4] = [
        SettingsField::RpcUrl,
        SettingsField::Account,
        SettingsField::PrivateKey,
        SettingsField::SkipExisting,
    ];

    fn idx(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap()
    }

    fn next(self) -> SettingsField {
        Self::ALL[(self.idx() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> SettingsField {
        Self::ALL[(self.idx() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    fn label(self) -> &'static str {
        match self {
            SettingsField::RpcUrl => "RPC URL",
            SettingsField::Account => "Account",
            SettingsField::PrivateKey => "Private key",
            SettingsField::SkipExisting => "Skip existing",
        }
    }
}

#[derive(Debug)]
struct SettingsForm {
    rpc_url: String,
    account: String,
    private_key: String,
    skip_existing: bool,
    focused: SettingsField,
    /// `true` while the user is typing into the focused field.
    editing: bool,
}

impl SettingsForm {
    fn from_defaults(d: SignerDefaults) -> Self {
        Self {
            rpc_url: d.rpc_url.unwrap_or_else(|| "http://localhost:5050".to_string()),
            account: d.account.map(|a| format!("{:#x}", Felt::from(a))).unwrap_or_default(),
            private_key: d.private_key.map(|k| format!("{k:#x}")).unwrap_or_default(),
            skip_existing: d.skip_existing,
            focused: SettingsField::RpcUrl,
            editing: false,
        }
    }

    /// Validate and convert into an [`ExecutorConfig`]. Returns a list of human-readable
    /// errors instead of bailing on the first one — better UX in a form.
    fn build(&self) -> std::result::Result<ExecutorConfig, Vec<String>> {
        let mut errs = Vec::new();
        let rpc_url = match Url::parse(&self.rpc_url) {
            Ok(u) => Some(u),
            Err(e) => {
                errs.push(format!("RPC URL: {e}"));
                None
            }
        };
        let account = if self.account.is_empty() {
            errs.push("Account is required".to_string());
            None
        } else {
            match Felt::from_str(&self.account) {
                Ok(f) => Some(ContractAddress::from(f)),
                Err(e) => {
                    errs.push(format!("Account: {e}"));
                    None
                }
            }
        };
        let private_key = if self.private_key.is_empty() {
            errs.push("Private key is required".to_string());
            None
        } else {
            match Felt::from_str(&self.private_key) {
                Ok(f) => Some(f),
                Err(e) => {
                    errs.push(format!("Private key: {e}"));
                    None
                }
            }
        };
        if errs.is_empty() {
            Ok(ExecutorConfig {
                rpc_url: rpc_url.unwrap(),
                account_address: account.unwrap(),
                private_key: private_key.unwrap(),
                skip_existing: self.skip_existing,
            })
        } else {
            Err(errs)
        }
    }
}

// -----------------------------------------------------------------------------
// Modals
// -----------------------------------------------------------------------------

#[derive(Debug)]
enum Modal {
    /// Pick an embedded class to declare, or open the file-load sub-modal.
    AddClassPicker { picker_state: ListState },
    /// Free-text path entry for loading a Sierra class file.
    AddClassFile { path: String, error: Option<String> },
    /// Add or edit a deploy. `editing_index = Some(i)` means we're editing in place.
    ContractForm {
        editing_index: Option<usize>,
        form: ContractForm,
    },
    /// Save manifest path prompt shown after successful execution.
    SaveManifest { path: String, error: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContractField {
    Class,
    Label,
    Salt,
    Unique,
    Calldata,
}

impl ContractField {
    const ALL: [ContractField; 5] = [
        ContractField::Class,
        ContractField::Label,
        ContractField::Salt,
        ContractField::Unique,
        ContractField::Calldata,
    ];

    fn idx(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap()
    }
    fn next(self) -> Self {
        Self::ALL[(self.idx() + 1) % Self::ALL.len()]
    }
    fn prev(self) -> Self {
        Self::ALL[(self.idx() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
    fn label(self) -> &'static str {
        match self {
            ContractField::Class => "Class",
            ContractField::Label => "Label",
            ContractField::Salt => "Salt",
            ContractField::Unique => "Unique",
            ContractField::Calldata => "Calldata",
        }
    }
}

#[derive(Debug)]
struct ContractForm {
    /// Index into the resolved class options list (declared + embedded).
    class_idx: usize,
    label: String,
    salt: String,
    unique: bool,
    calldata: String,
    focused: ContractField,
    error: Option<String>,
}

impl ContractForm {
    fn new() -> Self {
        Self {
            class_idx: 0,
            label: String::new(),
            salt: "0x0".to_string(),
            unique: false,
            calldata: String::new(),
            focused: ContractField::Class,
            error: None,
        }
    }

    fn from_existing(step: &DeployStep, class_options: &[ClassOption]) -> Self {
        let class_idx = class_options
            .iter()
            .position(|o| o.name == step.class_name)
            .unwrap_or(0);
        Self {
            class_idx,
            label: step.label.clone().unwrap_or_default(),
            salt: format!("{:#x}", step.salt),
            unique: step.unique,
            calldata: step
                .calldata
                .iter()
                .map(|f| format!("{f:#x}"))
                .collect::<Vec<_>>()
                .join(", "),
            focused: ContractField::Class,
            error: None,
        }
    }

    fn build(&self, class_options: &[ClassOption]) -> std::result::Result<DeployStep, String> {
        if class_options.is_empty() {
            return Err("no classes available — add one in the Classes tab first".to_string());
        }
        let class = &class_options[self.class_idx];
        let salt = Felt::from_str(self.salt.trim()).map_err(|e| format!("salt: {e}"))?;
        let calldata = if self.calldata.trim().is_empty() {
            Vec::new()
        } else {
            self.calldata
                .split(',')
                .map(|s| Felt::from_str(s.trim()).map_err(|e| format!("calldata `{s}`: {e}")))
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(DeployStep {
            label: if self.label.is_empty() { None } else { Some(self.label.clone()) },
            class_hash: class.class_hash,
            class_name: class.name.clone(),
            salt,
            unique: self.unique,
            calldata,
        })
    }
}

/// One row in the class picker that the contract form uses. Built fresh from the
/// app's classes + embedded registry whenever the modal opens, so it always reflects
/// the current declared set.
#[derive(Debug, Clone)]
struct ClassOption {
    name: String,
    class_hash: katana_primitives::class::ClassHash,
}

fn class_options(app: &AppState) -> Vec<ClassOption> {
    let mut out: Vec<ClassOption> = app
        .classes
        .iter()
        .map(|c| ClassOption { name: c.name.clone(), class_hash: c.class_hash })
        .collect();
    for entry in embedded::REGISTRY {
        if !out.iter().any(|o| o.name == entry.name) {
            out.push(ClassOption {
                name: entry.name.to_string(),
                class_hash: entry.class_hash,
            });
        }
    }
    out
}

// -----------------------------------------------------------------------------
// Execution state
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum RowStatus {
    Pending,
    Running,
    Done(String),
    Failed(String),
}

#[derive(Debug, Clone)]
struct ExecRow {
    label: String,
    status: RowStatus,
}

enum ExecutionState {
    Idle,
    Running {
        rx: UnboundedReceiver<BootstrapEvent>,
        /// Handle to the executor task. We keep it for ownership / cleanup; completion
        /// is detected via the terminal `Done`/`Failed` events on `rx`, not by polling
        /// the join handle (which would force us to drag a runtime handle into draining).
        _handle: JoinHandle<Result<BootstrapReport>>,
        rows: Vec<ExecRow>,
        tick: u64,
    },
    Done {
        rows: Vec<ExecRow>,
        result: std::result::Result<BootstrapReport, String>,
    },
}

impl std::fmt::Debug for ExecutionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Running { rows, tick, .. } => {
                write!(f, "Running({} rows, tick={tick})", rows.len())
            }
            Self::Done { result, .. } => write!(f, "Done(ok={})", result.is_ok()),
        }
    }
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// =============================================================================
// Event loop
// =============================================================================

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut AppState,
    runtime: &tokio::runtime::Handle,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw_app(f, app))?;

        if app.quit {
            return Ok(());
        }

        // Drain any pending progress events without blocking, so the spinner ticks
        // and the rows update on the next draw.
        drain_progress(app);

        // Poll for keyboard input with a short timeout — short enough that the spinner
        // looks alive (~16fps), long enough to avoid hot-spinning the CPU.
        if event::poll(Duration::from_millis(60))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                handle_key(app, key.code, key.modifiers, runtime);
            }
        } else if let ExecutionState::Running { tick, .. } = &mut app.execution {
            // No input → still tick the spinner so the running view animates.
            *tick = tick.wrapping_add(1);
        }
    }
}

fn drain_progress(app: &mut AppState) {
    // We need both the rows (mutable) and the classes len (immutable) at the same time,
    // so split the borrow up front.
    let classes_len = app.classes.len();
    let ExecutionState::Running { rx, rows, .. } = &mut app.execution else {
        return;
    };

    // Collect terminal events as we go so we can transition state after the drain loop
    // (the borrow checker would otherwise complain about reassigning `app.execution`).
    let mut terminal: Option<std::result::Result<BootstrapReport, String>> = None;

    while let Ok(event) = rx.try_recv() {
        match event {
            BootstrapEvent::DeclareStarted { idx, name, .. } => {
                if let Some(row) = rows.get_mut(idx) {
                    row.status = RowStatus::Running;
                    row.label = format!("declare  {name}");
                }
            }
            BootstrapEvent::DeclareCompleted {
                idx, name, class_hash, already_declared, ..
            } => {
                if let Some(row) = rows.get_mut(idx) {
                    let suffix = if already_declared { " (already)" } else { "" };
                    row.status = RowStatus::Done(format!("{class_hash:#x}{suffix}"));
                    row.label = format!("declare  {name}");
                }
            }
            BootstrapEvent::DeployStarted { idx, label, class_name } => {
                let row_idx = classes_len + idx;
                if let Some(row) = rows.get_mut(row_idx) {
                    row.status = RowStatus::Running;
                    row.label =
                        format!("deploy   {} ({class_name})", label.as_deref().unwrap_or("-"));
                }
            }
            BootstrapEvent::DeployCompleted {
                idx, label, class_name, address, ..
            } => {
                let row_idx = classes_len + idx;
                if let Some(row) = rows.get_mut(row_idx) {
                    row.status = RowStatus::Done(format!("{:#x}", Felt::from(address)));
                    row.label =
                        format!("deploy   {} ({class_name})", label.as_deref().unwrap_or("-"));
                }
            }
            BootstrapEvent::Failed { error } => {
                if let Some(row) = rows.iter_mut().find(|r| r.status == RowStatus::Running) {
                    row.status = RowStatus::Failed(error.clone());
                }
                terminal = Some(Err(error));
            }
            BootstrapEvent::Done { report } => {
                terminal = Some(Ok(report));
            }
        }
    }

    if let Some(result) = terminal {
        let prev = std::mem::replace(&mut app.execution, ExecutionState::Idle);
        if let ExecutionState::Running { rows, .. } = prev {
            app.execution = ExecutionState::Done { rows, result };
        }
    }
}

// =============================================================================
// Input handling
// =============================================================================

fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    mods: KeyModifiers,
    runtime: &tokio::runtime::Handle,
) {
    // Global Ctrl+C: hard quit no matter what's focused.
    if code == KeyCode::Char('c') && mods.contains(KeyModifiers::CONTROL) {
        app.quit = true;
        return;
    }

    app.flash = None;

    // Modal-first: if a modal is up, route input to it.
    if app.modal.is_some() {
        handle_modal_key(app, code);
        return;
    }

    // Global tab navigation (only when no modal is open).
    if code == KeyCode::Tab {
        app.current_tab = app.current_tab.next();
        return;
    }
    if code == KeyCode::BackTab {
        app.current_tab = app.current_tab.prev();
        return;
    }

    match app.current_tab {
        Tab::Classes => handle_classes_key(app, code),
        Tab::Contracts => handle_contracts_key(app, code),
        Tab::Settings => handle_settings_key(app, code),
        Tab::Execute => handle_execute_key(app, code, runtime),
    }
}

fn handle_classes_key(app: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char('a') => {
            app.modal = Some(Modal::AddClassPicker { picker_state: ListState::default() });
        }
        KeyCode::Char('d') => {
            if let Some(i) = app.classes_state.selected() {
                if i < app.classes.len() {
                    let removed = app.classes.remove(i);
                    if app.contracts.iter().any(|c| c.class_name == removed.name) {
                        app.flash(format!(
                            "warning: deleted class `{}` is referenced by a deploy",
                            removed.name
                        ));
                    }
                    if app.classes.is_empty() {
                        app.classes_state.select(None);
                    } else if i >= app.classes.len() {
                        app.classes_state.select(Some(app.classes.len() - 1));
                    }
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => move_list(&mut app.classes_state, app.classes.len(), 1),
        KeyCode::Up | KeyCode::Char('k') => move_list(&mut app.classes_state, app.classes.len(), -1),
        _ => {}
    }
}

fn handle_contracts_key(app: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char('a') => {
            app.modal = Some(Modal::ContractForm {
                editing_index: None,
                form: ContractForm::new(),
            });
        }
        KeyCode::Char('e') => {
            if let Some(i) = app.contracts_state.selected() {
                if let Some(existing) = app.contracts.get(i) {
                    let opts = class_options(app);
                    app.modal = Some(Modal::ContractForm {
                        editing_index: Some(i),
                        form: ContractForm::from_existing(existing, &opts),
                    });
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(i) = app.contracts_state.selected() {
                if i < app.contracts.len() {
                    app.contracts.remove(i);
                    if app.contracts.is_empty() {
                        app.contracts_state.select(None);
                    } else if i >= app.contracts.len() {
                        app.contracts_state.select(Some(app.contracts.len() - 1));
                    }
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            move_list(&mut app.contracts_state, app.contracts.len(), 1)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            move_list(&mut app.contracts_state, app.contracts.len(), -1)
        }
        _ => {}
    }
}

fn handle_settings_key(app: &mut AppState, code: KeyCode) {
    if app.settings.editing {
        match code {
            KeyCode::Esc | KeyCode::Enter => app.settings.editing = false,
            KeyCode::Char(c) => match app.settings.focused {
                SettingsField::RpcUrl => app.settings.rpc_url.push(c),
                SettingsField::Account => app.settings.account.push(c),
                SettingsField::PrivateKey => app.settings.private_key.push(c),
                SettingsField::SkipExisting => {}
            },
            KeyCode::Backspace => match app.settings.focused {
                SettingsField::RpcUrl => {
                    app.settings.rpc_url.pop();
                }
                SettingsField::Account => {
                    app.settings.account.pop();
                }
                SettingsField::PrivateKey => {
                    app.settings.private_key.pop();
                }
                SettingsField::SkipExisting => {}
            },
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Down | KeyCode::Char('j') => {
            app.settings.focused = app.settings.focused.next();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.settings.focused = app.settings.focused.prev();
        }
        KeyCode::Enter | KeyCode::Char('e') => {
            if app.settings.focused == SettingsField::SkipExisting {
                app.settings.skip_existing = !app.settings.skip_existing;
            } else {
                app.settings.editing = true;
            }
        }
        KeyCode::Char(' ') if app.settings.focused == SettingsField::SkipExisting => {
            app.settings.skip_existing = !app.settings.skip_existing;
        }
        _ => {}
    }
}

fn handle_execute_key(
    app: &mut AppState,
    code: KeyCode,
    runtime: &tokio::runtime::Handle,
) {
    match code {
        KeyCode::Char('q') => {
            // Don't allow quit while a task is mid-flight; the executor isn't cancellable.
            if matches!(app.execution, ExecutionState::Running { .. }) {
                app.flash("execution in progress — wait for it to finish");
            } else {
                app.quit = true;
            }
        }
        KeyCode::Esc => {
            if matches!(app.execution, ExecutionState::Running { .. }) {
                app.flash("execution in progress — wait for it to finish");
            } else {
                app.quit = true;
            }
        }
        KeyCode::Char('x') => {
            if matches!(app.execution, ExecutionState::Running { .. }) {
                app.flash("already running");
                return;
            }
            start_execution(app, runtime);
        }
        KeyCode::Char('s') => {
            if matches!(&app.execution, ExecutionState::Done { result: Ok(_), .. }) {
                app.modal = Some(Modal::SaveManifest {
                    path: "./bootstrap.toml".to_string(),
                    error: None,
                });
            }
        }
        _ => {}
    }
}

fn start_execution(app: &mut AppState, runtime: &tokio::runtime::Handle) {
    if app.classes.is_empty() && app.contracts.is_empty() {
        app.flash("nothing to do — add a class or contract first");
        return;
    }
    let cfg = match app.settings.build() {
        Ok(c) => c,
        Err(errs) => {
            app.flash(format!("settings invalid: {}", errs.join("; ")));
            app.current_tab = Tab::Settings;
            return;
        }
    };

    let plan =
        BootstrapPlan { declares: app.classes.clone(), deploys: app.contracts.clone() };

    // Build the per-row state up front from the plan, so the user sees every step
    // queued before any of them run.
    let mut rows: Vec<ExecRow> = Vec::with_capacity(plan.declares.len() + plan.deploys.len());
    for d in &plan.declares {
        rows.push(ExecRow {
            label: format!("declare  {}", d.name),
            status: RowStatus::Pending,
        });
    }
    for d in &plan.deploys {
        rows.push(ExecRow {
            label: format!("deploy   {} ({})", d.label.as_deref().unwrap_or("-"), d.class_name),
            status: RowStatus::Pending,
        });
    }

    let (tx, rx) = unbounded_channel();
    let plan_arc = Arc::new(plan);
    let cfg_arc = Arc::new(cfg);
    let plan_for_task = plan_arc.clone();
    let cfg_for_task = cfg_arc.clone();
    let handle: JoinHandle<Result<BootstrapReport>> = runtime.spawn(async move {
        execute_with_progress(&plan_for_task, &cfg_for_task, Some(tx)).await
    });

    app.execution = ExecutionState::Running { rx, _handle: handle, rows, tick: 0 };
}

// -----------------------------------------------------------------------------
// Modal input handling
// -----------------------------------------------------------------------------

fn handle_modal_key(app: &mut AppState, code: KeyCode) {
    // Take ownership so we can mutate the modal and then put it back, avoiding nested
    // borrows of `app`.
    let Some(modal) = app.modal.take() else { return };
    match modal {
        Modal::AddClassPicker { mut picker_state } => {
            // Picker entries: every embedded class + a final "Load from file…" row.
            let total = embedded::REGISTRY.len() + 1;
            match code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    // discard
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    move_list(&mut picker_state, total, 1);
                    app.modal = Some(Modal::AddClassPicker { picker_state });
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    move_list(&mut picker_state, total, -1);
                    app.modal = Some(Modal::AddClassPicker { picker_state });
                }
                KeyCode::Enter => {
                    let i = picker_state.selected().unwrap_or(0);
                    if i < embedded::REGISTRY.len() {
                        let entry = &embedded::REGISTRY[i];
                        push_embedded_class(app, entry);
                    } else {
                        // "Load from file…" → switch modals
                        app.modal = Some(Modal::AddClassFile {
                            path: String::new(),
                            error: None,
                        });
                    }
                }
                _ => {
                    app.modal = Some(Modal::AddClassPicker { picker_state });
                }
            }
        }
        Modal::AddClassFile { mut path, error: _ } => match code {
            KeyCode::Esc => {} // discard
            KeyCode::Enter => {
                let pb = PathBuf::from(path.trim());
                match load_class_file(&pb) {
                    Ok(step) => app.classes.push(step),
                    Err(e) => {
                        app.modal = Some(Modal::AddClassFile {
                            path,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
            KeyCode::Backspace => {
                path.pop();
                app.modal = Some(Modal::AddClassFile { path, error: None });
            }
            KeyCode::Char(c) => {
                path.push(c);
                app.modal = Some(Modal::AddClassFile { path, error: None });
            }
            _ => {
                app.modal = Some(Modal::AddClassFile { path, error: None });
            }
        },
        Modal::ContractForm { editing_index, mut form } => {
            // Reconstruct the class options every keystroke so the form always reflects
            // the up-to-date set (in case the user added classes elsewhere).
            let opts = class_options(app);
            match code {
                KeyCode::Esc => {} // discard
                KeyCode::Tab | KeyCode::Down => {
                    form.focused = form.focused.next();
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::BackTab | KeyCode::Up => {
                    form.focused = form.focused.prev();
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Left if form.focused == ContractField::Class => {
                    if !opts.is_empty() {
                        form.class_idx = (form.class_idx + opts.len() - 1) % opts.len();
                    }
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Right if form.focused == ContractField::Class => {
                    if !opts.is_empty() {
                        form.class_idx = (form.class_idx + 1) % opts.len();
                    }
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Char(' ') if form.focused == ContractField::Unique => {
                    form.unique = !form.unique;
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Char(c) => {
                    match form.focused {
                        ContractField::Label => form.label.push(c),
                        ContractField::Salt => form.salt.push(c),
                        ContractField::Calldata => form.calldata.push(c),
                        _ => {}
                    }
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Backspace => {
                    match form.focused {
                        ContractField::Label => {
                            form.label.pop();
                        }
                        ContractField::Salt => {
                            form.salt.pop();
                        }
                        ContractField::Calldata => {
                            form.calldata.pop();
                        }
                        _ => {}
                    }
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Enter => match form.build(&opts) {
                    Ok(step) => match editing_index {
                        Some(i) => {
                            if let Some(slot) = app.contracts.get_mut(i) {
                                *slot = step;
                            }
                        }
                        None => {
                            app.contracts.push(step);
                            app.contracts_state.select(Some(app.contracts.len() - 1));
                        }
                    },
                    Err(e) => {
                        form.error = Some(e);
                        app.modal = Some(Modal::ContractForm { editing_index, form });
                    }
                },
                _ => {
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
            }
        }
        Modal::SaveManifest { mut path, error: _ } => match code {
            KeyCode::Esc => {} // discard
            KeyCode::Enter => match save_manifest_from_app(app, &path) {
                Ok(()) => {
                    app.flash(format!("manifest saved to {path}"));
                }
                Err(e) => {
                    app.modal = Some(Modal::SaveManifest {
                        path,
                        error: Some(e.to_string()),
                    });
                }
            },
            KeyCode::Backspace => {
                path.pop();
                app.modal = Some(Modal::SaveManifest { path, error: None });
            }
            KeyCode::Char(c) => {
                path.push(c);
                app.modal = Some(Modal::SaveManifest { path, error: None });
            }
            _ => {
                app.modal = Some(Modal::SaveManifest { path, error: None });
            }
        },
    }
}

fn push_embedded_class(app: &mut AppState, entry: &'static EmbeddedClass) {
    // Don't add a duplicate by name.
    if app.classes.iter().any(|c| c.name == entry.name) {
        app.flash(format!("class `{}` is already in the plan", entry.name));
        return;
    }
    app.classes.push(DeclareStep {
        name: entry.name.to_string(),
        class: Arc::new(entry.class()),
        class_hash: entry.class_hash,
        casm_hash: entry.casm_hash,
        source: ClassSource::Embedded(entry.name),
    });
    app.classes_state.select(Some(app.classes.len() - 1));
}

fn load_class_file(path: &std::path::Path) -> Result<DeclareStep> {
    if !path.is_file() {
        return Err(anyhow!("file does not exist"));
    }
    let raw = std::fs::read_to_string(path)?;
    let class = ContractClass::from_str(&raw)?;
    if class.is_legacy() {
        return Err(anyhow!("legacy (Cairo 0) classes are not supported"));
    }
    let class_hash = class.class_hash()?;
    let casm_hash = class.clone().compile()?.class_hash()?;
    let alias = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{class_hash:#x}"));
    Ok(DeclareStep {
        name: alias,
        class: Arc::new(class),
        class_hash,
        casm_hash,
        source: ClassSource::File(path.to_path_buf()),
    })
}

fn save_manifest_from_app(app: &AppState, path: &str) -> Result<()> {
    let manifest = build_manifest_from_app(app);
    let serialized = toml::to_string_pretty(&manifest)?;
    std::fs::write(path, serialized)?;
    Ok(())
}

fn build_manifest_from_app(app: &AppState) -> Manifest {
    let classes = app
        .classes
        .iter()
        .map(|d| match &d.source {
            ClassSource::Embedded(name) => ClassEntry {
                name: d.name.clone(),
                embedded: Some((*name).to_string()),
                path: None,
            },
            ClassSource::File(path) => ClassEntry {
                name: d.name.clone(),
                embedded: None,
                path: Some(path.clone()),
            },
        })
        .collect();
    let contracts = app
        .contracts
        .iter()
        .map(|d| ContractEntry {
            class: d.class_name.clone(),
            label: d.label.clone(),
            salt: if d.salt == Felt::ZERO { None } else { Some(d.salt) },
            unique: d.unique,
            calldata: d.calldata.clone(),
        })
        .collect();
    Manifest { schema: 1, classes, contracts }
}

fn move_list(state: &mut ListState, len: usize, delta: i32) {
    if len == 0 {
        state.select(None);
        return;
    }
    let cur = state.selected().unwrap_or(0) as i32;
    let next = (cur + delta).clamp(0, len as i32 - 1);
    state.select(Some(next as usize));
}

// =============================================================================
// Drawing
// =============================================================================

fn draw_app(f: &mut ratatui::Frame<'_>, app: &mut AppState) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(2)])
        .split(f.area());

    draw_tab_bar(f, app, outer[0]);
    match app.current_tab {
        Tab::Classes => draw_classes_tab(f, app, outer[1]),
        Tab::Contracts => draw_contracts_tab(f, app, outer[1]),
        Tab::Settings => draw_settings_tab(f, app, outer[1]),
        Tab::Execute => draw_execute_tab(f, app, outer[1]),
    }
    draw_hint_bar(f, app, outer[2]);

    if let Some(modal) = app.modal.as_ref() {
        draw_modal(f, app, modal);
    }
}

fn draw_tab_bar(f: &mut ratatui::Frame<'_>, app: &AppState, area: Rect) {
    let titles: Vec<Line> = Tab::ALL.iter().map(|t| Line::from(t.title())).collect();
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("katana bootstrap"))
        .select(app.current_tab.idx())
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(tabs, area);
}

fn draw_hint_bar(f: &mut ratatui::Frame<'_>, app: &AppState, area: Rect) {
    let base = match app.current_tab {
        Tab::Classes => "[a] add  [d] delete  [j/k] navigate  [Tab] next tab  [q] quit",
        Tab::Contracts => "[a] add  [e] edit  [d] delete  [j/k] navigate  [Tab] next tab  [q] quit",
        Tab::Settings if app.settings.editing => "[Esc/Enter] stop editing",
        Tab::Settings => {
            "[j/k] move  [e/Enter] edit  [Space] toggle  [Tab] next tab  [q] quit"
        }
        Tab::Execute => match &app.execution {
            ExecutionState::Idle => "[x] run  [Tab] next tab  [q] quit",
            ExecutionState::Running { .. } => "running…",
            ExecutionState::Done { result: Ok(_), .. } => "[s] save manifest  [q] quit",
            ExecutionState::Done { .. } => "[q] quit",
        },
    };
    let text = if let Some(flash) = &app.flash {
        format!("{flash}    {base}")
    } else {
        base.to_string()
    };
    let style =
        if app.flash.is_some() { Style::default().fg(Color::Yellow) } else { Style::default() };
    let p = Paragraph::new(text).style(style);
    f.render_widget(p, area);
}

fn draw_classes_tab(f: &mut ratatui::Frame<'_>, app: &mut AppState, area: Rect) {
    let items: Vec<ListItem> = app
        .classes
        .iter()
        .map(|c| {
            let source = match &c.source {
                ClassSource::Embedded(_) => "embedded",
                ClassSource::File(_) => "file",
            };
            ListItem::new(format!("{:<20} {:<10} {:#x}", c.name, source, c.class_hash))
        })
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Classes to declare"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, &mut app.classes_state);
}

fn draw_contracts_tab(f: &mut ratatui::Frame<'_>, app: &mut AppState, area: Rect) {
    let items: Vec<ListItem> = app
        .contracts
        .iter()
        .map(|c| {
            ListItem::new(format!(
                "{:<15} {:<20} salt={:#x}  calldata=[{}]",
                c.label.as_deref().unwrap_or("-"),
                c.class_name,
                c.salt,
                c.calldata.iter().map(|f| format!("{f:#x}")).collect::<Vec<_>>().join(", ")
            ))
        })
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Contracts to deploy"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, &mut app.contracts_state);
}

fn draw_settings_tab(f: &mut ratatui::Frame<'_>, app: &AppState, area: Rect) {
    let mut lines = Vec::new();
    for field in SettingsField::ALL {
        let value = match field {
            SettingsField::RpcUrl => app.settings.rpc_url.clone(),
            SettingsField::Account => {
                if app.settings.account.is_empty() {
                    "(not set)".to_string()
                } else {
                    app.settings.account.clone()
                }
            }
            SettingsField::PrivateKey => {
                if app.settings.private_key.is_empty() {
                    "(not set)".to_string()
                } else {
                    "*".repeat(app.settings.private_key.len().min(16))
                }
            }
            SettingsField::SkipExisting => {
                if app.settings.skip_existing { "[x]" } else { "[ ]" }.to_string()
            }
        };
        let focused = field == app.settings.focused;
        let editing = focused && app.settings.editing;
        let marker = if focused { "> " } else { "  " };
        let cursor = if editing { "_" } else { "" };
        let style = if focused {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{:<14}", field.label()), style),
            Span::raw("  "),
            Span::styled(format!("{value}{cursor}"), style),
        ]));
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Settings"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_execute_tab(f: &mut ratatui::Frame<'_>, app: &AppState, area: Rect) {
    let header = format!(
        "Plan: {} declares, {} deploys",
        app.classes.len(),
        app.contracts.len()
    );

    let (rows, tick): (&[ExecRow], u64) = match &app.execution {
        ExecutionState::Idle => (&[], 0),
        ExecutionState::Running { rows, tick, .. } => (rows.as_slice(), *tick),
        ExecutionState::Done { rows, .. } => (rows.as_slice(), 0),
    };

    let mut lines: Vec<Line> = vec![Line::from(header), Line::from("")];
    if rows.is_empty() {
        lines.push(Line::from("(press `x` to start)"));
    } else {
        for row in rows {
            let (icon, style) = match &row.status {
                RowStatus::Pending => ("  ".to_string(), Style::default().fg(Color::DarkGray)),
                RowStatus::Running => {
                    let frame = SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()];
                    (format!("{frame} "), Style::default().fg(Color::Yellow))
                }
                RowStatus::Done(_) => ("✓ ".to_string(), Style::default().fg(Color::Green)),
                RowStatus::Failed(_) => ("✗ ".to_string(), Style::default().fg(Color::Red)),
            };
            let detail = match &row.status {
                RowStatus::Done(s) => format!("    {s}"),
                RowStatus::Failed(s) => format!("    {s}"),
                _ => String::new(),
            };
            lines.push(Line::from(vec![
                Span::styled(icon, style),
                Span::raw(row.label.clone()),
                Span::styled(detail, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    if let ExecutionState::Done { result: Err(err), .. } = &app.execution {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Failed: {err}"),
            Style::default().fg(Color::Red),
        )));
    }
    if let ExecutionState::Done { result: Ok(_), .. } = &app.execution {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Done. Press `s` to save the manifest or `q` to quit.",
            Style::default().fg(Color::Green),
        )));
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Execute"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_modal(f: &mut ratatui::Frame<'_>, app: &AppState, modal: &Modal) {
    let area = centered_rect(60, 70, f.area());
    f.render_widget(Clear, area);
    match modal {
        Modal::AddClassPicker { picker_state } => {
            let mut items: Vec<ListItem> = embedded::REGISTRY
                .iter()
                .map(|c| ListItem::new(format!("{} — {}", c.name, c.description)))
                .collect();
            items.push(ListItem::new("[Load Sierra class from file…]"));
            let mut state = picker_state.clone();
            if state.selected().is_none() {
                state.select(Some(0));
            }
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Add a class"))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("> ");
            f.render_stateful_widget(list, area, &mut state);
        }
        Modal::AddClassFile { path, error } => {
            let mut lines = vec![
                Line::from("Path to Sierra class JSON:"),
                Line::from(format!("  {path}_")),
                Line::from(""),
                Line::from(Span::styled(
                    "[Enter] add  [Esc] cancel",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            if let Some(e) = error {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("error: {e}"),
                    Style::default().fg(Color::Red),
                )));
            }
            let p = Paragraph::new(lines)
                .block(Block::default().borders(Borders::ALL).title("Load class file"));
            f.render_widget(p, area);
        }
        Modal::ContractForm { editing_index, form } => {
            let opts = class_options(app);
            let class_display = if opts.is_empty() {
                "(no classes)".to_string()
            } else {
                let c = &opts[form.class_idx];
                format!("◀ {} ▶", c.name)
            };
            let unique = if form.unique { "[x]" } else { "[ ]" };
            let title = if editing_index.is_some() { "Edit contract" } else { "Add contract" };

            let mut lines = Vec::new();
            for field in ContractField::ALL {
                let focused = field == form.focused;
                let marker = if focused { "> " } else { "  " };
                let value = match field {
                    ContractField::Class => class_display.clone(),
                    ContractField::Label => format!(
                        "{}{}",
                        form.label,
                        if focused { "_" } else { "" }
                    ),
                    ContractField::Salt => format!(
                        "{}{}",
                        form.salt,
                        if focused { "_" } else { "" }
                    ),
                    ContractField::Unique => unique.to_string(),
                    ContractField::Calldata => format!(
                        "{}{}",
                        form.calldata,
                        if focused { "_" } else { "" }
                    ),
                };
                let style = if focused {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{marker}{:<10}", field.label()), style),
                    Span::raw("  "),
                    Span::styled(value, style),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "[Tab] next field  [←/→] cycle class  [Space] toggle unique  [Enter] save  [Esc] cancel",
                Style::default().fg(Color::DarkGray),
            )));
            if let Some(e) = &form.error {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("error: {e}"),
                    Style::default().fg(Color::Red),
                )));
            }
            let p =
                Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title));
            f.render_widget(p, area);
        }
        Modal::SaveManifest { path, error } => {
            let mut lines = vec![
                Line::from("Save manifest to:"),
                Line::from(format!("  {path}_")),
                Line::from(""),
                Line::from(Span::styled(
                    "[Enter] save  [Esc] cancel",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            if let Some(e) = error {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("error: {e}"),
                    Style::default().fg(Color::Red),
                )));
            }
            let p = Paragraph::new(lines)
                .block(Block::default().borders(Borders::ALL).title("Save manifest"));
            f.render_widget(p, area);
        }
    }
}

/// Centered rect helper for modal overlays.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
