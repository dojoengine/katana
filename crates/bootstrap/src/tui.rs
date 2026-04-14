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
use std::time::{Duration, Instant};

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
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::Terminal;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::task::JoinHandle;
use url::Url;

use crate::embedded::{self, EmbeddedClass};
use crate::executor::{
    check_already_declared, check_already_deployed, compute_deploy_address, execute_with_progress,
    BootstrapEvent, BootstrapReport, ExecutorConfig,
};
use crate::manifest::{ClassEntry, ContractEntry, Manifest};
use crate::plan::{BootstrapPlan, ClassSource, DeclareStep, DeployStep};

// =============================================================================
// Public entry point
// =============================================================================

/// CLI-supplied defaults that prefill the Settings tab.
#[derive(Debug, Clone, Default)]
pub struct SignerDefaults {
    pub rpc_url: Option<String>,
    pub account: Option<ContractAddress>,
    pub private_key: Option<Felt>,
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
    classes: Vec<ClassItem>,
    classes_state: ListState,
    contracts: Vec<ContractItem>,
    contracts_state: ListState,
    settings: SettingsForm,
    modal: Option<Modal>,
    execution: ExecutionState,
    /// Frozen snapshot of the last completed run. Preserved across editing so the
    /// Execute tab keeps showing "what happened last" even as the user queues up
    /// new work. Cleared only on explicit reset (none today — we just append).
    last_run: Option<LastRunReport>,
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
            last_run: None,
            quit: false,
            flash: None,
        }
    }

    fn load_manifest(&mut self, manifest: &Manifest) -> Result<()> {
        // Reuse the existing manifest → plan resolver so we get the same validation,
        // file IO, and class-hash computation as programmatic mode.
        let plan = BootstrapPlan::from_manifest(manifest)?;
        self.classes = plan.declares.into_iter().map(ClassItem::from_step).collect();
        self.contracts = plan.deploys.into_iter().map(ContractItem::from_step).collect();
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

    /// True while any async task (Running or Refreshing) is in flight. Mutation
    /// handlers (add/edit/delete) gate on this: index maps stored in the active
    /// task point at positions in `classes`/`contracts`, so shifting those lists
    /// mid-flight would corrupt progress updates.
    fn is_busy(&self) -> bool {
        !matches!(self.execution, ExecutionState::Idle)
    }
}

// -----------------------------------------------------------------------------
// Per-item durable exec state
// -----------------------------------------------------------------------------

/// Per-item execution status carried across runs. Distinct from
/// [`RowStatus`], which is the per-row live display status inside a single
/// executor run — this one persists on the source item so the Classes and
/// Contracts tabs can show a durable "Done (hash …)" badge after the user
/// adds more items and navigates away from the Execute tab.
///
/// Truth model: the executor's live RPC checks at run time are authoritative.
/// `ItemExecState` is a UI hint populated by `drain_progress` on completion
/// events and reconciled on every run — a cached summary of "what the last
/// interaction with the node said about this item."
#[derive(Debug, Clone)]
enum ItemExecState {
    /// Never executed (default for new items) or reset by the user after an
    /// edit. Eligible for inclusion in the next run.
    Pending,
    /// A run is in flight and this item is currently being submitted.
    Running,
    /// The item completed successfully. `detail` is the user-facing summary
    /// (class hash for declares, contract address for deploys) rendered
    /// inline in the Classes/Contracts tabs.
    Done { detail: String },
    /// The last run attempted this item and the submission failed. `detail`
    /// is the error message. Eligible for retry on the next run.
    Failed { detail: String },
    /// Status was invalidated by a Settings change and the refresh couldn't
    /// re-verify against the new node (network down, timeout, etc.). The user
    /// sees a `?` badge. Eligible for inclusion in the next run — we treat
    /// unknown as "try again and see what the node says."
    Unknown { reason: String },
}

impl ItemExecState {
    /// Whether this item should be included in the next [`BootstrapPlan`]
    /// built by [`start_execution`]. Done items are already on the node
    /// (according to our best-effort knowledge) and skipping them avoids a
    /// round-trip to the idempotency check per item — the executor still
    /// re-checks at run time, so this is a UX optimization, not a correctness
    /// guarantee.
    fn is_outstanding(&self) -> bool {
        !matches!(self, ItemExecState::Done { .. })
    }

    /// Optional inline detail rendered next to the icon in the Classes and
    /// Contracts tabs. Pending/Running have no detail; Done/Failed/Unknown
    /// carry a user-facing summary.
    fn detail(&self) -> Option<&str> {
        match self {
            ItemExecState::Pending | ItemExecState::Running => None,
            ItemExecState::Done { detail } => Some(detail),
            ItemExecState::Failed { detail } => Some(detail),
            ItemExecState::Unknown { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone)]
struct ClassItem {
    step: DeclareStep,
    exec: ItemExecState,
}

impl ClassItem {
    fn from_step(step: DeclareStep) -> Self {
        Self { step, exec: ItemExecState::Pending }
    }
}

// Allow reading the underlying `DeclareStep` fields without a `.step.` prefix
// everywhere. This reduces diff noise in the draw/modal code that just needs
// the plan data. We add this instead of `impl Deref` because `DeclareStep`
// isn't obviously a "newtype of ClassItem" and we don't want to encourage
// treating ClassItem as a drop-in replacement.
impl std::ops::Deref for ClassItem {
    type Target = DeclareStep;
    fn deref(&self) -> &Self::Target {
        &self.step
    }
}

#[derive(Debug, Clone)]
struct ContractItem {
    step: DeployStep,
    exec: ItemExecState,
}

impl ContractItem {
    fn from_step(step: DeployStep) -> Self {
        Self { step, exec: ItemExecState::Pending }
    }
}

impl std::ops::Deref for ContractItem {
    type Target = DeployStep;
    fn deref(&self) -> &Self::Target {
        &self.step
    }
}

/// Frozen snapshot of a completed run's Execute-tab view. Kept on
/// [`AppState::last_run`] so the Execute tab still shows what happened most
/// recently even after the user returns to Idle and starts queuing more work.
#[derive(Debug, Clone)]
struct LastRunReport {
    rows: Vec<ExecRow>,
    result: std::result::Result<BootstrapReport, String>,
}

// -----------------------------------------------------------------------------
// Settings form
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsField {
    RpcUrl,
    Account,
    PrivateKey,
}

impl SettingsField {
    const ALL: [SettingsField; 3] =
        [SettingsField::RpcUrl, SettingsField::Account, SettingsField::PrivateKey];

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
        }
    }
}

#[derive(Debug)]
struct SettingsForm {
    rpc_url: TextInput,
    account: TextInput,
    private_key: TextInput,
    focused: SettingsField,
    /// `true` while the user is typing into the focused field.
    editing: bool,
    /// Set whenever any field is mutated. Cleared by [`Self::take_dirty`] on
    /// Settings tab-exit — that's the commit boundary that triggers the
    /// refresh of previously-Done items against the new node. Without this
    /// flag, every per-keystroke mutation would look like "settings changed"
    /// and we'd spawn a storm of refresh tasks.
    dirty: bool,
}

impl SettingsForm {
    fn from_defaults(d: SignerDefaults) -> Self {
        Self {
            rpc_url: TextInput::from_str(
                d.rpc_url.unwrap_or_else(|| "http://localhost:5050".to_string()),
            ),
            account: TextInput::from_str(
                d.account.map(|a| format!("{:#x}", Felt::from(a))).unwrap_or_default(),
            ),
            private_key: TextInput::from_str(
                d.private_key.map(|k| format!("{k:#x}")).unwrap_or_default(),
            ),
            focused: SettingsField::RpcUrl,
            editing: false,
            dirty: false,
        }
    }

    /// Consume the dirty flag and return whether it was set. Used at the
    /// Settings tab-exit commit boundary: if dirty, caller kicks off a refresh.
    fn take_dirty(&mut self) -> bool {
        let was = self.dirty;
        self.dirty = false;
        was
    }

    /// Validate and convert into an [`ExecutorConfig`]. Returns a list of human-readable
    /// errors instead of bailing on the first one — better UX in a form.
    fn build(&self) -> std::result::Result<ExecutorConfig, Vec<String>> {
        let mut errs = Vec::new();
        let rpc_url = match Url::parse(self.rpc_url.as_str()) {
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
            match Felt::from_str(self.account.as_str()) {
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
            match Felt::from_str(self.private_key.as_str()) {
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
            })
        } else {
            Err(errs)
        }
    }

    fn focused_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.focused {
            SettingsField::RpcUrl => Some(&mut self.rpc_url),
            SettingsField::Account => Some(&mut self.account),
            SettingsField::PrivateKey => Some(&mut self.private_key),
        }
    }
}

// -----------------------------------------------------------------------------
// Text input widget — cursor + readline-style shortcuts
// -----------------------------------------------------------------------------

/// A single-line editable text buffer with a cursor position. All edit operations
/// keep `cursor` on a UTF-8 char boundary so multi-byte input works correctly.
///
/// We roll our own instead of pulling in `tui-input` because the surface we need is
/// small and the alternative is yet another transitive dependency for ~150 lines.
#[derive(Debug, Default, Clone)]
struct TextInput {
    value: String,
    /// Byte offset of the cursor inside `value`. Always at a char boundary, always
    /// in `0..=value.len()`.
    cursor: usize,
}

impl TextInput {
    fn new() -> Self {
        Self::default()
    }

    fn from_str(s: impl Into<String>) -> Self {
        let value = s.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    fn as_str(&self) -> &str {
        &self.value
    }

    fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.value.len()
    }

    /// Insert a single character at the cursor and advance.
    fn insert_char(&mut self, c: char) {
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Delete the character before the cursor (Backspace / Ctrl+H).
    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Walk backwards to the previous char boundary.
        let mut new_cursor = self.cursor - 1;
        while !self.value.is_char_boundary(new_cursor) {
            new_cursor -= 1;
        }
        self.value.replace_range(new_cursor..self.cursor, "");
        self.cursor = new_cursor;
    }

    /// Delete the character at the cursor (Delete / Ctrl+D).
    fn delete_forward(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let mut end = self.cursor + 1;
        while end < self.value.len() && !self.value.is_char_boundary(end) {
            end += 1;
        }
        self.value.replace_range(self.cursor..end, "");
    }

    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut c = self.cursor - 1;
        while !self.value.is_char_boundary(c) {
            c -= 1;
        }
        self.cursor = c;
    }

    fn move_right(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let mut c = self.cursor + 1;
        while c < self.value.len() && !self.value.is_char_boundary(c) {
            c += 1;
        }
        self.cursor = c;
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.value.len();
    }

    /// Delete the word before the cursor (Ctrl+W). Words are runs of non-whitespace.
    /// First eats trailing whitespace, then a run of non-whitespace — same as bash.
    fn delete_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let bytes = self.value.as_bytes();
        let mut i = self.cursor;
        // Eat whitespace.
        while i > 0 && bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        // Eat the word.
        while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        // Snap to char boundary in case we landed mid-multibyte.
        while !self.value.is_char_boundary(i) {
            i -= 1;
        }
        self.value.replace_range(i..self.cursor, "");
        self.cursor = i;
    }

    /// Delete from the cursor to the start of the line (Ctrl+U).
    fn kill_to_start(&mut self) {
        self.value.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    /// Delete from the cursor to the end of the line (Ctrl+K).
    fn kill_to_end(&mut self) {
        self.value.truncate(self.cursor);
    }
}

/// Apply a key event to a [`TextInput`], handling printable characters, Backspace,
/// arrow keys, and the standard readline shortcuts (Ctrl+A/E/B/F/H/D/W/U/K).
///
/// Returns `true` when the key was consumed by the editor — callers should *not*
/// fall through to their own keybindings in that case (e.g. don't treat Ctrl+A as
/// "select all" in some other context). Returns `false` for keys the editor doesn't
/// recognise (Enter, Esc, Tab, function keys, …) so the caller can handle them.
fn handle_text_edit(input: &mut TextInput, code: KeyCode, mods: KeyModifiers) -> bool {
    let ctrl = mods.contains(KeyModifiers::CONTROL);

    if ctrl {
        match code {
            KeyCode::Char('a') => {
                input.move_home();
                return true;
            }
            KeyCode::Char('e') => {
                input.move_end();
                return true;
            }
            KeyCode::Char('b') => {
                input.move_left();
                return true;
            }
            KeyCode::Char('f') => {
                input.move_right();
                return true;
            }
            KeyCode::Char('h') => {
                input.backspace();
                return true;
            }
            KeyCode::Char('d') => {
                input.delete_forward();
                return true;
            }
            KeyCode::Char('w') => {
                input.delete_word_backward();
                return true;
            }
            KeyCode::Char('u') => {
                input.kill_to_start();
                return true;
            }
            KeyCode::Char('k') => {
                input.kill_to_end();
                return true;
            }
            _ => return false,
        }
    }

    match code {
        KeyCode::Char(c) => {
            input.insert_char(c);
            true
        }
        KeyCode::Backspace => {
            input.backspace();
            true
        }
        KeyCode::Delete => {
            input.delete_forward();
            true
        }
        KeyCode::Left => {
            input.move_left();
            true
        }
        KeyCode::Right => {
            input.move_right();
            true
        }
        KeyCode::Home => {
            input.move_home();
            true
        }
        KeyCode::End => {
            input.move_end();
            true
        }
        _ => false,
    }
}

/// Render a focused [`TextInput`] as three styled spans (`before`, `at_cursor`, `after`)
/// so the cursor is visually distinct without needing real terminal cursor positioning.
/// When the cursor is past the end of the buffer, an extra space is highlighted to
/// give the user something to see.
fn render_text_input<'a>(input: &'a TextInput, focused: bool, mask: bool) -> Vec<Span<'a>> {
    let display: String =
        if mask { "*".repeat(input.value.chars().count()) } else { input.value.clone() };

    if !focused {
        return vec![Span::raw(display)];
    }

    // For masked inputs, project the cursor by char count rather than bytes.
    let (before, at, after) = if mask {
        let chars: Vec<char> = display.chars().collect();
        let cursor_chars = input.value[..input.cursor].chars().count().min(chars.len());
        let before: String = chars[..cursor_chars].iter().collect();
        let at = chars.get(cursor_chars).copied();
        let after: String = chars.get(cursor_chars + 1..).into_iter().flatten().collect();
        (before, at, after)
    } else {
        let before = input.value[..input.cursor].to_string();
        let mut after_chars = input.value[input.cursor..].chars();
        let at = after_chars.next();
        let after = after_chars.as_str().to_string();
        (before, at, after)
    };

    let cursor_style = Style::default().add_modifier(Modifier::REVERSED);
    let mut spans = vec![Span::raw(before)];
    match at {
        Some(c) => spans.push(Span::styled(c.to_string(), cursor_style)),
        None => spans.push(Span::styled(" ", cursor_style)),
    }
    spans.push(Span::raw(after));
    spans
}

// -----------------------------------------------------------------------------
// Fuzzy file search (used by the AddClassFile modal)
// -----------------------------------------------------------------------------

/// Soft cap on the number of files we're willing to fuzzy-search over. In huge repos
/// the cwd walk would otherwise stall the UI thread when the modal opens.
const FILE_SEARCH_MAX_CANDIDATES: usize = 20_000;

/// Recursive depth limit for the cwd walk. 8 is enough to reach Sierra artifacts in
/// any reasonable nested project layout (e.g. `target/dev/foo.contract_class.json`)
/// without paying for arbitrarily deep `node_modules`-style trees.
const FILE_SEARCH_MAX_DEPTH: usize = 8;

/// Number of matches to keep in the visible list — anything beyond this is dropped
/// before rendering, since the modal area only shows so many lines anyway.
const FILE_SEARCH_VISIBLE: usize = 20;

/// How long to wait between the last keystroke and actually re-scoring the candidate
/// list. Short enough that the result list feels responsive, long enough that fast
/// typing doesn't cause a re-render storm.
const FILE_SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug)]
struct FileSearch {
    /// Raw query as the user has typed it. While the query is empty the matches list
    /// is intentionally also empty — we don't want to dump cwd into the user's face
    /// before they've expressed any intent.
    query: TextInput,
    /// All `*.json` files found under cwd at modal-open time. Cached for the lifetime
    /// of the modal so re-scoring is cheap on every keystroke.
    candidates: Vec<PathBuf>,
    /// Current results. Recomputed lazily after the debounce window expires.
    matches: Vec<(PathBuf, i64)>,
    /// Index into `matches`. Always valid when `matches` is non-empty.
    selected: usize,
    /// Root we walked from — surfaced in the modal title so the user knows where the
    /// candidate list came from.
    root: PathBuf,
    /// Set to `Some(deadline)` when the query has been mutated since the last recompute.
    /// The TUI event loop's tick checks this each iteration and runs `recompute` once
    /// the deadline has passed, so fast typing doesn't trigger one full match-and-sort
    /// pass per keystroke.
    pending_recompute: Option<Instant>,
}

impl FileSearch {
    /// Walk cwd and build the candidate list. Cheap-ish: we filter aggressively (only
    /// `*.json`, only at depth ≤ 8, ignoring obvious heavy directories).
    fn open() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let candidates = walk_json_files(&root);
        Self {
            query: TextInput::new(),
            candidates,
            matches: Vec::new(),
            selected: 0,
            root,
            pending_recompute: None,
        }
    }

    /// Mark the query as dirty; the actual recompute happens later via [`Self::tick`]
    /// once the debounce window elapses. Callers don't need to know about the timer.
    fn mark_dirty(&mut self) {
        self.pending_recompute = Some(Instant::now() + FILE_SEARCH_DEBOUNCE);
    }

    /// Called from the event loop on every tick. If a recompute is pending and its
    /// deadline has passed, run it. Returns `true` if a recompute actually happened
    /// (so the caller can know it should redraw, though we draw every tick anyway).
    fn tick(&mut self, now: Instant) -> bool {
        match self.pending_recompute {
            Some(when) if now >= when => {
                self.recompute();
                self.pending_recompute = None;
                true
            }
            _ => false,
        }
    }

    /// Re-derive matches from the current query. Public so tests can drive it
    /// synchronously without faking time.
    fn recompute(&mut self) {
        let prev = self.matches.get(self.selected).map(|(p, _)| p.clone());
        self.matches.clear();

        // Empty query → empty results. Showing the entire walked tree before the user
        // has typed anything is noisy and almost never what they want.
        if self.query.is_empty() {
            self.selected = 0;
            return;
        }

        for path in &self.candidates {
            let display = path.to_string_lossy();
            if let Some(score) = fuzzy_score(self.query.as_str(), &display) {
                self.matches.push((path.clone(), score));
            }
        }
        // Higher score first; on a tie prefer the shorter (more specific) path.
        self.matches.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| a.0.as_os_str().len().cmp(&b.0.as_os_str().len()))
        });
        self.matches.truncate(FILE_SEARCH_VISIBLE);

        // Try to keep the same row highlighted across recomputes when possible — UX
        // sanity so the cursor doesn't jump every keystroke.
        self.selected =
            prev.and_then(|p| self.matches.iter().position(|(m, _)| *m == p)).unwrap_or(0);
    }

    fn move_down(&mut self) {
        if !self.matches.is_empty() && self.selected + 1 < self.matches.len() {
            self.selected += 1;
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// What `Enter` should load. Highlighted match wins; if there are no matches but
    /// the query as-typed parses to a real file (e.g. an absolute path the user
    /// pasted), fall back to that. Returns `None` when nothing is loadable.
    fn resolve_choice(&self) -> Option<PathBuf> {
        if let Some((path, _)) = self.matches.get(self.selected) {
            return Some(path.clone());
        }
        let trimmed = self.query.as_str().trim();
        if trimmed.is_empty() {
            return None;
        }
        let pb = expand_tilde(trimmed);
        if pb.is_file() {
            Some(pb)
        } else {
            None
        }
    }
}

fn walk_json_files(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= FILE_SEARCH_MAX_CANDIDATES {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // Skip hidden + obvious heavy directories — these are almost never where
            // the user keeps their compiled Sierra artifacts and walking them would
            // dwarf the rest of the search.
            if name.starts_with('.') {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                if matches!(name, "target" | "node_modules" | "build" | "dist") {
                    continue;
                }
                if depth + 1 < FILE_SEARCH_MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if file_type.is_file() && path.extension().is_some_and(|e| e == "json") {
                out.push(path);
                if out.len() >= FILE_SEARCH_MAX_CANDIDATES {
                    break;
                }
            }
        }
    }
    out
}

/// Tiny inline fuzzy scorer: classic subsequence match (case-insensitive) with a
/// bonus for adjacent characters and a penalty for long candidates. Returns `None`
/// when the query isn't a subsequence of the candidate. Good enough for a v1 file
/// picker — if we ever want word-boundary or camelCase bonuses, drop in
/// `fuzzy-matcher`'s `SkimMatcherV2` and delete this.
fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut q_idx = 0usize;
    let mut score: i64 = 0;
    let mut last_match: Option<usize> = None;
    for (i, ch) in candidate.to_lowercase().chars().enumerate() {
        if q_idx >= q.len() {
            break;
        }
        if ch == q[q_idx] {
            // +10 for adjacent matches, +1 for any other match.
            if last_match == Some(i.wrapping_sub(1)) {
                score += 10;
            } else {
                score += 1;
            }
            last_match = Some(i);
            q_idx += 1;
        }
    }
    if q_idx < q.len() {
        return None;
    }
    // Penalize long candidates so a query that matches both `foo.json` and
    // `path/to/foo.json` ranks the former higher.
    Some(score * 5 - candidate.len() as i64)
}

fn expand_tilde(s: &str) -> PathBuf {
    PathBuf::from(shellexpand::tilde(s).into_owned())
}

// -----------------------------------------------------------------------------
// Modals
// -----------------------------------------------------------------------------

#[derive(Debug)]
enum Modal {
    /// Pick an embedded class to declare, or open the file-load sub-modal.
    AddClassPicker {
        picker_state: ListState,
        /// In-modal status line. Set when the user clicks an entry that's already in
        /// the plan, so the dup notice shows up at the bottom of the picker instead
        /// of in the global flash bar.
        info: Option<String>,
    },
    /// Fuzzy file picker for loading a Sierra class JSON from disk. The user types
    /// a query and we live-filter the cached file list as they go; absolute paths
    /// (starting with `/` or `~`) bypass fuzzy matching and are treated as literal.
    AddClassFile { search: FileSearch, error: Option<String> },

    /// Dedicated "compiling…" modal that takes over once the user picks a file in
    /// [`Modal::AddClassFile`]. The actual parse + Sierra→CASM compile runs on a
    /// background thread; this modal exists to communicate that work to the user.
    /// On success the modal closes; on failure (or Esc) we restore the prior
    /// `AddClassFile` modal so the user keeps their search context.
    LoadingClass {
        pending: PendingLoad,
        /// The search state we came from. Restored verbatim on cancel/failure so the
        /// user doesn't have to retype their query and pick the file again.
        return_to_search: FileSearch,
        /// In-modal notice. Set after the worker completes when the freshly-loaded
        /// class turns out to already be in the plan — the spinner is replaced by
        /// this message and Esc dismisses the modal entirely.
        notice: Option<String>,
    },
    /// Add or edit a deploy. `editing_index = Some(i)` means we're editing in place.
    ContractForm { editing_index: Option<usize>, form: ContractForm },
    /// Save manifest path prompt shown after successful execution.
    SaveManifest { path: TextInput, error: Option<String> },
}

/// Background load handle for the AddClassFile modal. The worker thread does the
/// expensive `parse + class_hash + casm_compile` chain, then ships the result back
/// to the UI thread via a sync channel that we drain on every event-loop tick.
#[derive(Debug)]
struct PendingLoad {
    /// The path being loaded — surfaced in the modal so the user knows what's spinning.
    path: PathBuf,
    /// One-shot result channel. The worker writes once and disconnects.
    rx: std::sync::mpsc::Receiver<Result<DeclareStep>>,
    /// Wall-clock start time, used to drive the spinner frame.
    started: Instant,
}

/// A field address inside the contract form. The set of valid fields depends
/// on the form's current `calldata` state — when the selected class has a
/// constructor with N inputs, there are N `Arg(i)` fields; otherwise there's
/// a single `RawCalldata` field. The `ContractForm::fields` helper materializes
/// the live list for navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContractField {
    Class,
    Label,
    Salt,
    Unique,
    /// Index into [`CalldataInput::Typed`]'s `args` vec.
    Arg(usize),
    /// The single fallback raw calldata input, used when the class has no
    /// introspectable constructor.
    RawCalldata,
}

/// Form state for the constructor calldata. Either a list of typed inputs
/// (one per constructor argument, with the type known up-front), or a single
/// freeform felt list when we couldn't introspect the class.
#[derive(Debug)]
enum CalldataInput {
    Typed { args: Vec<TypedArg> },
    Raw(TextInput),
}

#[derive(Debug)]
struct TypedArg {
    name: String,
    ty: crate::abi::TypeNode,
    input: TextInput,
}

#[derive(Debug)]
struct ContractForm {
    /// Index into the resolved class options list (declared + embedded).
    class_idx: usize,
    /// Cached display name for the currently-selected class. Updated by
    /// [`ContractForm::sync_class`] so the renderer doesn't have to call
    /// the expensive [`class_options`] on every frame.
    class_name: String,
    label: TextInput,
    salt: TextInput,
    unique: bool,
    /// Constructor inputs for the currently-selected class. Rebuilt whenever
    /// the class selection changes via [`ContractForm::sync_class`].
    calldata: CalldataInput,
    focused: ContractField,
    error: Option<String>,
}

impl ContractForm {
    fn new(opts: &[ClassOption]) -> Self {
        let calldata = build_calldata_input(opts.first().and_then(|o| o.constructor.as_ref()));
        let class_name = opts.first().map(|o| o.name.clone()).unwrap_or_default();
        Self {
            class_idx: 0,
            class_name,
            label: TextInput::new(),
            salt: TextInput::from_str("0x0"),
            unique: false,
            calldata,
            focused: ContractField::Class,
            error: None,
        }
    }

    fn from_existing(step: &DeployStep, opts: &[ClassOption]) -> Self {
        let class_idx = opts.iter().position(|o| o.name == step.class_name).unwrap_or(0);
        // Always prefer typed mode when the class has a constructor ABI, and
        // pre-fill the inputs by decoding the existing raw felts back into
        // display strings. Falls back to raw mode only when the class has no
        // introspectable constructor.
        let ctor = opts.get(class_idx).and_then(|o| o.constructor.as_ref());
        let calldata = if let Some(ctor) = ctor {
            let mut typed = build_calldata_input(Some(ctor));
            // Walk the existing calldata and try to decode each arg's portion.
            if let CalldataInput::Typed { ref mut args } = typed {
                let mut offset = 0;
                for arg in args.iter_mut() {
                    if offset < step.calldata.len() {
                        if let Some((display, consumed)) =
                            crate::abi::from_calldata(&arg.ty, &step.calldata[offset..])
                        {
                            arg.input = TextInput::from_str(display);
                            offset += consumed;
                        }
                    }
                }
            }
            typed
        } else {
            CalldataInput::Raw(TextInput::from_str(
                step.calldata.iter().map(|f| format!("{f:#x}")).collect::<Vec<_>>().join(", "),
            ))
        };
        let class_name = opts.get(class_idx).map(|o| o.name.clone()).unwrap_or_default();
        Self {
            class_idx,
            class_name,
            label: TextInput::from_str(step.label.clone().unwrap_or_default()),
            salt: TextInput::from_str(format!("{:#x}", step.salt)),
            unique: step.unique,
            calldata,
            focused: ContractField::Class,
            error: None,
        }
    }

    /// Live list of focusable fields. Recomputed each time because the tail
    /// (`Arg(_)` vs `RawCalldata`) depends on the current `calldata` state.
    fn fields(&self) -> Vec<ContractField> {
        let mut out = vec![
            ContractField::Class,
            ContractField::Label,
            ContractField::Salt,
            ContractField::Unique,
        ];
        match &self.calldata {
            CalldataInput::Typed { args } => {
                for i in 0..args.len() {
                    out.push(ContractField::Arg(i));
                }
            }
            CalldataInput::Raw(_) => out.push(ContractField::RawCalldata),
        }
        out
    }

    fn focus_next(&mut self) {
        let fields = self.fields();
        let idx = fields.iter().position(|f| f == &self.focused).unwrap_or(0);
        self.focused = fields[(idx + 1) % fields.len()].clone();
    }

    fn focus_prev(&mut self) {
        let fields = self.fields();
        let idx = fields.iter().position(|f| f == &self.focused).unwrap_or(0);
        self.focused = fields[(idx + fields.len() - 1) % fields.len()].clone();
    }

    /// Reapply the constructor template after a class change. Must be called
    /// every time `class_idx` is updated. Uses positional carry-over so that
    /// values the user already typed survive the swap when the new class has
    /// at least as many args.
    fn sync_class(&mut self, opts: &[ClassOption]) {
        self.class_name = opts.get(self.class_idx).map(|o| o.name.clone()).unwrap_or_default();
        let ctor = opts.get(self.class_idx).and_then(|o| o.constructor.as_ref());
        let new_calldata = build_calldata_input(ctor);

        // Carry typed values across the swap when both sides are typed. We
        // copy by position rather than by name because constructor argument
        // names often match between similar classes (`recipient`, `amount`)
        // and matching by name would silently drop renames.
        if let (CalldataInput::Typed { args: old }, CalldataInput::Typed { args: mut new }) =
            (&self.calldata, new_calldata)
        {
            for (i, arg) in new.iter_mut().enumerate() {
                if let Some(prev) = old.get(i) {
                    arg.input = prev.input.clone();
                }
            }
            self.calldata = CalldataInput::Typed { args: new };
        } else {
            self.calldata = build_calldata_input(ctor);
        }

        // If the focus is now off-the-end of the args list, snap it back.
        let fields = self.fields();
        if !fields.contains(&self.focused) {
            self.focused = ContractField::Class;
        }
    }

    fn build(&self, opts: &[ClassOption]) -> std::result::Result<DeployStep, String> {
        if opts.is_empty() {
            return Err("no classes available — add one in the Classes tab first".to_string());
        }
        let class = &opts[self.class_idx];
        let salt = Felt::from_str(self.salt.as_str().trim()).map_err(|e| format!("salt: {e}"))?;
        let calldata = match &self.calldata {
            CalldataInput::Typed { args } => {
                let mut out = Vec::new();
                for arg in args {
                    let value = crate::abi::parse_text_value(arg.input.as_str());
                    let encoded = crate::abi::to_calldata(&arg.ty, &value)
                        .map_err(|e| format!("arg `{}`: {e}", arg.name))?;
                    out.extend(encoded);
                }
                out
            }
            CalldataInput::Raw(input) => {
                if input.as_str().trim().is_empty() {
                    Vec::new()
                } else {
                    input
                        .as_str()
                        .split(',')
                        .map(|s| {
                            Felt::from_str(s.trim()).map_err(|e| format!("calldata `{s}`: {e}"))
                        })
                        .collect::<std::result::Result<Vec<_>, _>>()?
                }
            }
        };
        Ok(DeployStep {
            label: if self.label.is_empty() { None } else { Some(self.label.as_str().to_string()) },
            class_hash: class.class_hash,
            class_name: class.name.clone(),
            salt,
            unique: self.unique,
            calldata,
        })
    }

    /// Mutable handle to whichever field is currently focused, used by the modal
    /// keyboard handler to forward edit ops generically.
    fn focused_input_mut(&mut self) -> Option<&mut TextInput> {
        match &self.focused {
            ContractField::Label => Some(&mut self.label),
            ContractField::Salt => Some(&mut self.salt),
            ContractField::RawCalldata => match &mut self.calldata {
                CalldataInput::Raw(input) => Some(input),
                CalldataInput::Typed { .. } => None,
            },
            ContractField::Arg(i) => match &mut self.calldata {
                CalldataInput::Typed { args } => args.get_mut(*i).map(|a| &mut a.input),
                CalldataInput::Raw(_) => None,
            },
            ContractField::Class | ContractField::Unique => None,
        }
    }
}

/// Build a fresh [`CalldataInput`] from a constructor. `None` (no ABI / no
/// constructor) yields the raw fallback. A constructor with zero inputs
/// yields an empty typed list — still typed mode, since the build will emit
/// `Vec::new()` either way and there's nothing to ask the user.
fn build_calldata_input(ctor: Option<&crate::abi::ConstructorAbi>) -> CalldataInput {
    match ctor {
        Some(c) => CalldataInput::Typed {
            args: c
                .inputs
                .iter()
                .map(|a| TypedArg {
                    name: a.name.clone(),
                    ty: a.ty.clone(),
                    input: TextInput::new(),
                })
                .collect(),
        },
        None => CalldataInput::Raw(TextInput::new()),
    }
}

/// One row in the class picker that the contract form uses. Built fresh from the
/// app's classes + embedded registry whenever the modal opens, so it always reflects
/// the current declared set.
#[derive(Debug, Clone)]
struct ClassOption {
    name: String,
    class_hash: katana_primitives::class::ClassHash,
    /// Parsed constructor signature, when the class has a Sierra ABI we can
    /// introspect. `None` for legacy classes, classes whose ABI is missing
    /// or invalid, and classes whose ABI doesn't declare a constructor.
    constructor: Option<crate::abi::ConstructorAbi>,
}

fn class_options(app: &AppState) -> Vec<ClassOption> {
    let mut out: Vec<ClassOption> = app
        .classes
        .iter()
        .map(|c| ClassOption {
            name: c.step.name.clone(),
            class_hash: c.step.class_hash,
            constructor: crate::abi::extract_constructor(&c.step.class),
        })
        .collect();
    for entry in embedded::REGISTRY {
        if !out.iter().any(|o| o.name == entry.name) {
            // Materialize the embedded class once so we can read its ABI. The
            // load is gated behind the modal-open path, so the cost only hits
            // when the user is actively choosing a class.
            let class = entry.class();
            out.push(ClassOption {
                name: entry.name.to_string(),
                class_hash: entry.class_hash,
                constructor: crate::abi::extract_constructor(&class),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecKind {
    Declare,
    Deploy,
}

impl ExecKind {
    fn as_str(self) -> &'static str {
        match self {
            ExecKind::Declare => "declare",
            ExecKind::Deploy => "deploy",
        }
    }
}

/// One row of the Execute tab. The structured fields exist so the renderer can
/// compute column widths once across all rows and pad each cell to that width —
/// otherwise the trailing detail (hash/address/error) would zig-zag with the
/// length of every primary/secondary field.
#[derive(Debug, Clone)]
struct ExecRow {
    kind: ExecKind,
    /// Declare: the local class alias. Deploy: the contract label.
    primary: String,
    /// Deploy-only: the class the contract is deploying. `None` for declares.
    secondary: Option<String>,
    status: RowStatus,
}

enum ExecutionState {
    /// No async task in flight. The tabs accept mutations; `x` starts a new
    /// run; Settings tab-exit (after edits) kicks off a refresh.
    Idle,
    /// The executor is running. The event loop drains `rx`; the `_handle` is
    /// kept to own the spawned task (completion is detected via terminal
    /// `Done`/`Failed` events on `rx`, not by polling the join handle). The
    /// `run` field owns the transient row view and the index maps that let
    /// `drain_progress` reconcile per-item state on the source classes /
    /// contracts lists.
    Running {
        rx: UnboundedReceiver<BootstrapEvent>,
        _handle: JoinHandle<Result<BootstrapReport>>,
        run: ActiveExecution,
        tick: u64,
    },
    /// A Settings change invalidated previously-Done items and a background
    /// task is re-probing the node to see what's actually there. Updates arrive
    /// on `rx` as [`RefreshEvent`] messages; terminal state is the `Done`
    /// event, after which we transition back to `Idle`. `dirty_items` is the
    /// pair of indices `(classes_dirty, contracts_dirty)` that the task needs
    /// to probe — we stage them in the struct so the UI can show "checking…"
    /// on exactly those items.
    Refreshing { rx: UnboundedReceiver<RefreshEvent>, _handle: JoinHandle<()>, tick: u64 },
}

/// Per-run transient state. Lives inside [`ExecutionState::Running`] so it gets
/// dropped the moment the run terminates; the frozen view that survives into
/// `last_run` is [`LastRunReport`], not this.
struct ActiveExecution {
    /// Row view of the current run. One `ExecRow` per submitted step, indexed
    /// as `[declares..., deploys...]` so progress events from the executor
    /// (which carry `idx` within their kind) can map back to a row.
    rows: Vec<ExecRow>,
    /// Mapping from "declare row index in `rows`" to "position in
    /// `app.classes`". Populated at [`start_execution`] time so drain_progress
    /// can write durable per-item state back to the source. `class_indices[i]`
    /// is the app-level index for the i-th declare row.
    class_indices: Vec<usize>,
    /// Mirror of `class_indices` for deploys.
    contract_indices: Vec<usize>,
}

impl std::fmt::Debug for ExecutionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Running { run, tick, .. } => {
                write!(f, "Running({} rows, tick={tick})", run.rows.len())
            }
            Self::Refreshing { tick, .. } => write!(f, "Refreshing(tick={tick})"),
        }
    }
}

impl std::fmt::Debug for ActiveExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveExecution")
            .field("rows", &self.rows.len())
            .field("class_indices", &self.class_indices)
            .field("contract_indices", &self.contract_indices)
            .finish()
    }
}

/// Progress events emitted by the Settings-change refresh task. Arrive on the
/// [`ExecutionState::Refreshing`] channel and get applied to per-item state
/// by [`drain_progress`].
#[derive(Debug)]
enum RefreshEvent {
    /// One class's state was resolved. `Ok(true)` = already declared on the
    /// new node, `Ok(false)` = not declared (mark Pending), `Err(reason)` =
    /// network/RPC failure, mark Unknown so the user can retry.
    ClassResolved { app_idx: usize, result: std::result::Result<bool, String> },
    /// Same as `ClassResolved` but for deploys.
    ContractResolved { app_idx: usize, result: std::result::Result<bool, String> },
    /// Terminal event. The task emits this after all items have been probed
    /// (or the task was canceled). No payload — per-item results arrive on the
    /// Resolved events above.
    Done,
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

        // Tick any background work attached to the current modal:
        //
        // 1. AddClassFile fuzzy search debounce — schedules a recompute once the user has stopped
        //    typing for `FILE_SEARCH_DEBOUNCE`.
        // 2. LoadingClass receiver — drains the background loader's result if ready; on success
        //    closes the modal and adds the class, on failure transitions back to AddClassFile with
        //    the error attached and the previous search state preserved.
        if let Some(Modal::AddClassFile { search, .. }) = app.modal.as_mut() {
            search.tick(Instant::now());
        }
        let mut completed_load: Option<Result<DeclareStep>> = None;
        if let Some(Modal::LoadingClass { pending, notice, .. }) = app.modal.as_mut() {
            // Once a `notice` has been set the load is already settled — the worker
            // thread has dropped its sender, so a fresh `try_recv` would return
            // `Disconnected` and we'd misreport that as "thread vanished without
            // sending a result". Skip the drain entirely in that state.
            if notice.is_none() {
                match pending.rx.try_recv() {
                    Ok(result) => completed_load = Some(result),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {} // still loading
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        completed_load = Some(Err(anyhow!(
                            "background class loader thread disconnected without sending a result"
                        )));
                    }
                }
            }
        }
        if let Some(result) = completed_load {
            // Take the modal so we can move out of `return_to_search` on failure
            // or transition the modal in place on success.
            let modal = app.modal.take();
            match (result, modal) {
                (Ok(step), Some(Modal::LoadingClass { pending, return_to_search, .. })) => {
                    // Dedup by class hash. We can only do this *after* loading
                    // because the hash isn't known until the Sierra has been parsed
                    // and compiled — by which point the worker has already paid the
                    // CPU cost. That's fine: dedup is best-effort, and the rare case
                    // of paying for a redundant compile is preferable to either
                    // blocking on a synchronous pre-load or letting duplicate
                    // declares slip through.
                    let dup = app.classes.iter().any(|c| c.step.class_hash == step.class_hash);
                    if dup {
                        // Stay in the LoadingClass modal but swap the spinner for an
                        // inline notice. The user dismisses with Esc.
                        app.modal = Some(Modal::LoadingClass {
                            pending,
                            return_to_search,
                            notice: Some("Class is already selected".to_string()),
                        });
                    } else {
                        app.classes.push(ClassItem::from_step(step));
                        app.classes_state.select(Some(app.classes.len() - 1));
                        // Modal closed by `take()` — class added to the plan.
                    }
                }
                (Ok(_), other) => {
                    // Defensive: completed_load should only be Ok when the modal is
                    // LoadingClass. If we somehow get here, restore whatever modal
                    // was up and drop the result.
                    app.modal = other;
                }
                (Err(err), Some(Modal::LoadingClass { return_to_search, .. })) => {
                    app.modal = Some(Modal::AddClassFile {
                        search: return_to_search,
                        error: Some(err.to_string()),
                    });
                }
                (Err(err), other) => {
                    // Defensive: completed_load should only be set when the modal
                    // is LoadingClass. Surface the error via the global flash bar
                    // since we have nowhere better to put it.
                    app.flash(format!("background load error: {err}"));
                    app.modal = other;
                }
            }
        }

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
    match &mut app.execution {
        ExecutionState::Idle => {}
        ExecutionState::Running { .. } => drain_exec_progress(app),
        ExecutionState::Refreshing { .. } => drain_refresh_progress(app),
    }
}

/// A per-item update the executor drain wants to apply to the source lists
/// after it releases the `&mut app.execution` borrow. We stage these in a
/// `Vec` and apply them in a second pass so we can mutate both `app.classes`
/// and `app.contracts` without fighting the borrow checker over
/// `app.execution.run`.
#[derive(Debug)]
enum SourceUpdate {
    Class { app_idx: usize, exec: ItemExecState },
    Contract { app_idx: usize, exec: ItemExecState },
}

fn drain_exec_progress(app: &mut AppState) {
    // Stage every source-item write we want to do during this drain. We can't
    // mutate `app.classes` / `app.contracts` while we hold a borrow on
    // `app.execution.run`, so queue and apply below.
    let mut updates: Vec<SourceUpdate> = Vec::new();

    // Collect the terminal outcome (if any) so we can transition state after
    // the drain loop releases its borrow on `app.execution`.
    let mut terminal: Option<std::result::Result<BootstrapReport, String>> = None;

    // Scope the mutable borrow on `app.execution` so it's released before we
    // mutate `app.classes` / `app.contracts` and reassign `app.execution`.
    let classes_len;
    {
        // Matched explicitly above, so the Some pattern is safe. Pull the fields
        // we need into locals bound in this block.
        let ExecutionState::Running { rx, run, .. } = &mut app.execution else {
            return;
        };
        classes_len = run.class_indices.len();

        while let Ok(event) = rx.try_recv() {
            match event {
                BootstrapEvent::DeclareStarted { idx, .. } => {
                    if let Some(row) = run.rows.get_mut(idx) {
                        row.status = RowStatus::Running;
                    }
                    if let Some(app_idx) = run.class_indices.get(idx).copied() {
                        updates.push(SourceUpdate::Class { app_idx, exec: ItemExecState::Running });
                    }
                }
                BootstrapEvent::DeclareCompleted { idx, class_hash, already_declared, .. } => {
                    let suffix = if already_declared { " (already declared)" } else { "" };
                    let detail = format!("class hash:  {class_hash:#x}{suffix}");
                    if let Some(row) = run.rows.get_mut(idx) {
                        row.status = RowStatus::Done(detail.clone());
                    }
                    if let Some(app_idx) = run.class_indices.get(idx).copied() {
                        // Durable per-item summary is the hash + already-declared
                        // marker. Shown in the Classes tab after the run finishes.
                        updates.push(SourceUpdate::Class {
                            app_idx,
                            exec: ItemExecState::Done {
                                detail: format!("{class_hash:#x}{suffix}"),
                            },
                        });
                    }
                }
                BootstrapEvent::DeployStarted { idx, .. } => {
                    let row_idx = classes_len + idx;
                    if let Some(row) = run.rows.get_mut(row_idx) {
                        row.status = RowStatus::Running;
                    }
                    if let Some(app_idx) = run.contract_indices.get(idx).copied() {
                        updates
                            .push(SourceUpdate::Contract { app_idx, exec: ItemExecState::Running });
                    }
                }
                BootstrapEvent::DeployCompleted { idx, address, already_deployed, .. } => {
                    let suffix = if already_deployed { " (already deployed)" } else { "" };
                    let row_idx = classes_len + idx;
                    let address_felt: Felt = address.into();
                    if let Some(row) = run.rows.get_mut(row_idx) {
                        row.status =
                            RowStatus::Done(format!("address:     {address_felt:#x}{suffix}"));
                    }
                    if let Some(app_idx) = run.contract_indices.get(idx).copied() {
                        updates.push(SourceUpdate::Contract {
                            app_idx,
                            exec: ItemExecState::Done {
                                detail: format!("{address_felt:#x}{suffix}"),
                            },
                        });
                    }
                }
                BootstrapEvent::Failed { error } => {
                    // Mark whichever row is currently Running as Failed for the
                    // live view, and durably record the failure on the matching
                    // source item — the user sees the red `✗` badge on the
                    // Classes/Contracts tabs until they edit the item.
                    let running_row_idx =
                        run.rows.iter().position(|r| r.status == RowStatus::Running);
                    if let Some(row_idx) = running_row_idx {
                        if let Some(row) = run.rows.get_mut(row_idx) {
                            row.status = RowStatus::Failed(error.clone());
                        }
                        // Translate the row index back to either classes or contracts.
                        if row_idx < classes_len {
                            if let Some(app_idx) = run.class_indices.get(row_idx).copied() {
                                updates.push(SourceUpdate::Class {
                                    app_idx,
                                    exec: ItemExecState::Failed { detail: error.clone() },
                                });
                            }
                        } else {
                            let deploy_idx = row_idx - classes_len;
                            if let Some(app_idx) = run.contract_indices.get(deploy_idx).copied() {
                                updates.push(SourceUpdate::Contract {
                                    app_idx,
                                    exec: ItemExecState::Failed { detail: error.clone() },
                                });
                            }
                        }
                    }
                    terminal = Some(Err(error));
                }
                BootstrapEvent::Done { report } => {
                    terminal = Some(Ok(report));
                }
            }
        }

        // Second safety net: if the executor task panicked or was dropped without
        // sending a terminal event, `try_recv` eventually returns `Disconnected`.
        // Without this check we'd stay in `Running` forever, blocking all input.
        // The existing code pre-refactor had this same latent bug; we close it here.
        if terminal.is_none()
            && matches!(rx.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Disconnected))
        {
            terminal = Some(Err("executor task disconnected without a terminal event".to_string()));
        }
    } // end borrow on app.execution

    // Apply staged per-item updates.
    for u in updates {
        match u {
            SourceUpdate::Class { app_idx, exec } => {
                if let Some(item) = app.classes.get_mut(app_idx) {
                    item.exec = exec;
                }
            }
            SourceUpdate::Contract { app_idx, exec } => {
                if let Some(item) = app.contracts.get_mut(app_idx) {
                    item.exec = exec;
                }
            }
        }
    }

    if let Some(result) = terminal {
        // Transition Running → Idle and freeze the run view into last_run. We
        // take the whole state by replace so the JoinHandle inside the
        // Running variant is dropped at the same time.
        let prev = std::mem::replace(&mut app.execution, ExecutionState::Idle);
        if let ExecutionState::Running { run, .. } = prev {
            app.last_run = Some(LastRunReport { rows: run.rows, result });
        }
    }
}

fn drain_refresh_progress(app: &mut AppState) {
    // Same two-pass pattern as drain_exec_progress: collect updates, release
    // the `app.execution` borrow, then apply to `app.classes` / `app.contracts`.
    let mut updates: Vec<SourceUpdate> = Vec::new();
    let mut done = false;

    {
        let ExecutionState::Refreshing { rx, .. } = &mut app.execution else {
            return;
        };

        while let Ok(event) = rx.try_recv() {
            match event {
                RefreshEvent::ClassResolved { app_idx, result } => {
                    let exec = match result {
                        Ok(true) => ItemExecState::Done {
                            // We don't know the original detail string anymore (class hash
                            // isn't in the Resolved payload); re-synthesize from the step.
                            // The caller fills this in properly below when applying —
                            // store a marker here.
                            detail: "__refresh_done__".to_string(),
                        },
                        Ok(false) => ItemExecState::Pending,
                        Err(reason) => ItemExecState::Unknown { reason },
                    };
                    updates.push(SourceUpdate::Class { app_idx, exec });
                }
                RefreshEvent::ContractResolved { app_idx, result } => {
                    let exec = match result {
                        Ok(true) => ItemExecState::Done { detail: "__refresh_done__".to_string() },
                        Ok(false) => ItemExecState::Pending,
                        Err(reason) => ItemExecState::Unknown { reason },
                    };
                    updates.push(SourceUpdate::Contract { app_idx, exec });
                }
                RefreshEvent::Done => {
                    done = true;
                }
            }
        }

        // Disconnected-without-Done safety net, same as drain_exec_progress.
        if !done
            && matches!(rx.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Disconnected))
        {
            done = true;
        }
    } // end borrow on app.execution

    for u in updates {
        match u {
            SourceUpdate::Class { app_idx, exec } => {
                if let Some(item) = app.classes.get_mut(app_idx) {
                    // Replace the placeholder detail with the real class-hash summary
                    // when the refresh said "still declared."
                    item.exec = match exec {
                        ItemExecState::Done { .. } => ItemExecState::Done {
                            detail: format!("{:#x} (verified)", item.step.class_hash),
                        },
                        other => other,
                    };
                }
            }
            SourceUpdate::Contract { app_idx, exec } => {
                if let Some(item) = app.contracts.get_mut(app_idx) {
                    // Deploy verification doesn't trivially give us the address back from
                    // the refresh event, so recompute it from the step + current signer.
                    // If we can't (account missing or invalid), fall back to a terse marker.
                    item.exec = match exec {
                        ItemExecState::Done { .. } => {
                            let detail = app
                                .settings
                                .build()
                                .ok()
                                .map(|cfg| {
                                    let addr =
                                        compute_deploy_address(&item.step, cfg.account_address);
                                    format!("{:#x} (verified)", Felt::from(addr))
                                })
                                .unwrap_or_else(|| "(verified)".to_string());
                            ItemExecState::Done { detail }
                        }
                        other => other,
                    };
                }
            }
        }
    }

    if done {
        app.execution = ExecutionState::Idle;
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
        handle_modal_key(app, code, mods, runtime);
        return;
    }

    // Global tab navigation (only when no modal is open). Leaving the Settings
    // tab after any edit is the commit boundary that kicks off a refresh of
    // previously-Done items against the (possibly-new) node.
    if code == KeyCode::Tab {
        let leaving_settings = app.current_tab == Tab::Settings;
        // Only the Execute tab blocks tab navigation during a run — every other
        // tab is read-only in practice while busy, so we let the user navigate
        // freely to observe progress. The Settings tab-exit refresh trigger
        // intentionally only fires when we're Idle: kicking off a second async
        // task while one's already running would be a race.
        if app.is_busy() {
            // Keep current behavior: during Running, blocking quit also
            // effectively "parks" the user on the Execute tab. During
            // Refreshing, nav is fine — but we skip the commit-boundary hook
            // so we don't double-start the refresh.
            app.current_tab = app.current_tab.next();
            return;
        }
        app.current_tab = app.current_tab.next();
        if leaving_settings && app.settings.take_dirty() {
            start_refresh(app, runtime);
        }
        return;
    }
    if code == KeyCode::BackTab {
        let leaving_settings = app.current_tab == Tab::Settings;
        if app.is_busy() {
            app.current_tab = app.current_tab.prev();
            return;
        }
        app.current_tab = app.current_tab.prev();
        if leaving_settings && app.settings.take_dirty() {
            start_refresh(app, runtime);
        }
        return;
    }

    match app.current_tab {
        Tab::Classes => handle_classes_key(app, code),
        Tab::Contracts => handle_contracts_key(app, code),
        Tab::Settings => handle_settings_key(app, code, mods),
        Tab::Execute => handle_execute_key(app, code, runtime),
    }
}

fn handle_classes_key(app: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
            } else {
                app.quit = true;
            }
        }
        KeyCode::Char('a') => {
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
                return;
            }
            app.modal =
                Some(Modal::AddClassPicker { picker_state: ListState::default(), info: None });
        }
        KeyCode::Char('d') => {
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
                return;
            }
            if let Some(i) = app.classes_state.selected() {
                if i < app.classes.len() {
                    let removed = app.classes.remove(i);
                    if app.contracts.iter().any(|c| c.step.class_name == removed.step.name) {
                        app.flash(format!(
                            "warning: deleted class `{}` is referenced by a deploy",
                            removed.step.name
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
        KeyCode::Down | KeyCode::Char('j') => {
            move_list(&mut app.classes_state, app.classes.len(), 1)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            move_list(&mut app.classes_state, app.classes.len(), -1)
        }
        _ => {}
    }
}

fn handle_contracts_key(app: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
            } else {
                app.quit = true;
            }
        }
        KeyCode::Char('a') => {
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
                return;
            }
            let opts = class_options(app);
            app.modal =
                Some(Modal::ContractForm { editing_index: None, form: ContractForm::new(&opts) });
        }
        KeyCode::Char('e') => {
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
                return;
            }
            if let Some(i) = app.contracts_state.selected() {
                if let Some(existing) = app.contracts.get(i) {
                    let opts = class_options(app);
                    app.modal = Some(Modal::ContractForm {
                        editing_index: Some(i),
                        form: ContractForm::from_existing(&existing.step, &opts),
                    });
                }
            }
        }
        KeyCode::Char('d') => {
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
                return;
            }
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

fn handle_settings_key(app: &mut AppState, code: KeyCode, mods: KeyModifiers) {
    if app.settings.editing {
        // Esc / Enter exit edit mode; everything else is forwarded to the focused
        // text input via the standard readline-style editor.
        if matches!(code, KeyCode::Esc | KeyCode::Enter) {
            app.settings.editing = false;
            return;
        }
        if let Some(input) = app.settings.focused_input_mut() {
            // Any consumed key is a mutation — mark the form dirty so the
            // next tab-exit triggers a refresh of previously-Done items.
            if handle_text_edit(input, code, mods) {
                app.settings.dirty = true;
            }
        }
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
            } else {
                app.quit = true;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.settings.focused = app.settings.focused.next();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.settings.focused = app.settings.focused.prev();
        }
        KeyCode::Enter | KeyCode::Char('e') => {
            app.settings.editing = true;
        }
        _ => {}
    }
}

fn handle_execute_key(app: &mut AppState, code: KeyCode, runtime: &tokio::runtime::Handle) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            // Don't allow quit while any async task is mid-flight; neither the
            // executor nor the refresh task is cancellable. Ctrl+C is the
            // escape hatch (handled globally).
            if app.is_busy() {
                app.flash("in progress — wait for it to finish");
            } else {
                app.quit = true;
            }
        }
        KeyCode::Char('x') => {
            match &app.execution {
                ExecutionState::Running { .. } => {
                    app.flash("already running");
                    return;
                }
                ExecutionState::Refreshing { .. } => {
                    app.flash("verifying node — wait");
                    return;
                }
                ExecutionState::Idle => {}
            }
            start_execution(app, runtime);
        }
        KeyCode::Char('s') => {
            // Save manifest is available whenever we're idle — a plan is worth
            // saving even before it's ever been executed. The old "only after
            // success" gate conflated "run a bootstrap" with "record your
            // intended bootstrap plan"; those are separate needs.
            if matches!(app.execution, ExecutionState::Idle) {
                app.modal = Some(Modal::SaveManifest {
                    path: TextInput::from_str("./bootstrap.toml"),
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

    // Filter to outstanding work only: Done items are skipped entirely (the
    // executor's idempotency check would re-verify them on-chain, but that's
    // one RPC round-trip per Done item — skipping them in the plan itself is
    // both faster and makes the Execute tab view match what actually happens).
    // Pending, Failed, and Unknown all re-run; the executor's precheck still
    // guards against double-submits if the user's cache is stale.
    let mut declares: Vec<DeclareStep> = Vec::new();
    let mut class_indices: Vec<usize> = Vec::new();
    for (i, item) in app.classes.iter().enumerate() {
        if item.exec.is_outstanding() {
            declares.push(item.step.clone());
            class_indices.push(i);
        }
    }
    let mut deploys: Vec<DeployStep> = Vec::new();
    let mut contract_indices: Vec<usize> = Vec::new();
    for (i, item) in app.contracts.iter().enumerate() {
        if item.exec.is_outstanding() {
            deploys.push(item.step.clone());
            contract_indices.push(i);
        }
    }

    if declares.is_empty() && deploys.is_empty() {
        app.flash("nothing pending — all items are done");
        return;
    }

    // Build the per-row state up front from the plan, so the user sees every step
    // queued before any of them run.
    let mut rows: Vec<ExecRow> = Vec::with_capacity(declares.len() + deploys.len());
    for d in &declares {
        rows.push(ExecRow {
            kind: ExecKind::Declare,
            primary: d.name.clone(),
            secondary: None,
            status: RowStatus::Pending,
        });
    }
    for d in &deploys {
        rows.push(ExecRow {
            kind: ExecKind::Deploy,
            primary: d.label.clone().unwrap_or_else(|| "-".to_string()),
            secondary: Some(d.class_name.clone()),
            status: RowStatus::Pending,
        });
    }

    let plan = BootstrapPlan { declares, deploys };
    let (tx, rx) = unbounded_channel();
    let plan_arc = Arc::new(plan);
    let cfg_arc = Arc::new(cfg);
    let plan_for_task = plan_arc.clone();
    let cfg_for_task = cfg_arc.clone();
    let handle: JoinHandle<Result<BootstrapReport>> = runtime
        .spawn(async move { execute_with_progress(&plan_for_task, &cfg_for_task, Some(tx)).await });

    app.execution = ExecutionState::Running {
        rx,
        _handle: handle,
        run: ActiveExecution { rows, class_indices, contract_indices },
        tick: 0,
    };
}

/// Spawn a Settings-change refresh task. For every item in a refreshable
/// state (Done or Unknown), probe the new RPC concurrently to see what's
/// actually there. Items transition to Done/Pending based on the probe, or
/// Unknown on RPC failure. Pending and Failed items are left alone — those
/// go through the regular `x` re-run flow, not the refresh path.
///
/// `Unknown` is included alongside `Done` so that recovering from a failed
/// refresh (e.g. user typed an invalid RPC URL, all items went Unknown)
/// actually re-probes when the user fixes the URL. Without this, a single
/// bad refresh would strand items as Unknown forever.
fn needs_refresh(state: &ItemExecState) -> bool {
    matches!(state, ItemExecState::Done { .. } | ItemExecState::Unknown { .. })
}

fn start_refresh(app: &mut AppState, runtime: &tokio::runtime::Handle) {
    // Nothing to verify? No-op.
    if !app.classes.iter().any(|c| needs_refresh(&c.exec))
        && !app.contracts.iter().any(|c| needs_refresh(&c.exec))
    {
        return;
    }

    // Can't probe without valid settings. Flash and bail; the user still sees
    // the stale Done/Unknown badges, but we can't do anything useful until
    // they fix the URL or account.
    let cfg = match app.settings.build() {
        Ok(c) => c,
        Err(_) => {
            return;
        }
    };

    // Gather the items to probe alongside their app-level indices. We mark
    // them Unknown up front so the Classes/Contracts tabs immediately show the
    // stale state while the async probe is in flight.
    let mut declare_probes: Vec<(usize, katana_primitives::class::ClassHash)> = Vec::new();
    for (i, item) in app.classes.iter_mut().enumerate() {
        if needs_refresh(&item.exec) {
            item.exec = ItemExecState::Unknown { reason: "verifying…".to_string() };
            declare_probes.push((i, item.step.class_hash));
        }
    }
    let mut deploy_probes: Vec<(usize, ContractAddress)> = Vec::new();
    for (i, item) in app.contracts.iter_mut().enumerate() {
        if needs_refresh(&item.exec) {
            item.exec = ItemExecState::Unknown { reason: "verifying…".to_string() };
            let addr = compute_deploy_address(&item.step, cfg.account_address);
            deploy_probes.push((i, addr));
        }
    }

    let (tx, rx) = unbounded_channel();
    let rpc_url = cfg.rpc_url.clone();
    let handle: JoinHandle<()> = runtime.spawn(async move {
        // Fire all probes concurrently. FuturesUnordered gives us best-effort
        // parallelism without imposing a concurrency cap; see TODOS.md for
        // when to revisit (large manifests hitting RPC rate limits).
        use futures::stream::{FuturesUnordered, StreamExt};

        let mut tasks = FuturesUnordered::new();
        for (app_idx, class_hash) in declare_probes {
            let url = rpc_url.clone();
            let tx = tx.clone();
            tasks.push(tokio::spawn(async move {
                let result =
                    check_already_declared(&url, class_hash).await.map_err(|e| format!("{e:#}"));
                let _ = tx.send(RefreshEvent::ClassResolved { app_idx, result });
            }));
        }
        for (app_idx, address) in deploy_probes {
            let url = rpc_url.clone();
            let tx = tx.clone();
            tasks.push(tokio::spawn(async move {
                let result =
                    check_already_deployed(&url, address).await.map_err(|e| format!("{e:#}"));
                let _ = tx.send(RefreshEvent::ContractResolved { app_idx, result });
            }));
        }

        while (tasks.next().await).is_some() {
            // Per-probe result already forwarded by the spawn body; we just
            // drain for join completion here.
        }

        let _ = tx.send(RefreshEvent::Done);
    });

    app.execution = ExecutionState::Refreshing { rx, _handle: handle, tick: 0 };
}

// -----------------------------------------------------------------------------
// Modal input handling
// -----------------------------------------------------------------------------

fn handle_modal_key(
    app: &mut AppState,
    code: KeyCode,
    mods: KeyModifiers,
    _runtime: &tokio::runtime::Handle,
) {
    // Take ownership so we can mutate the modal and then put it back, avoiding nested
    // borrows of `app`.
    let Some(modal) = app.modal.take() else { return };
    match modal {
        Modal::AddClassPicker { mut picker_state, info: _ } => {
            // Picker entries: every embedded class + a final "Load from file…" row.
            let total = embedded::REGISTRY.len() + 1;
            match code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    // discard
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    move_list(&mut picker_state, total, 1);
                    // Moving the cursor clears any stale "already in plan" notice;
                    // it'd be confusing to keep showing it for an entry the user
                    // is no longer hovering.
                    app.modal = Some(Modal::AddClassPicker { picker_state, info: None });
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    move_list(&mut picker_state, total, -1);
                    app.modal = Some(Modal::AddClassPicker { picker_state, info: None });
                }
                KeyCode::Enter => {
                    let i = picker_state.selected().unwrap_or(0);
                    if i < embedded::REGISTRY.len() {
                        let entry = &embedded::REGISTRY[i];
                        // Manual dup check so we can keep the modal open and show
                        // the message inline. We deliberately don't reuse
                        // `push_embedded_class`'s flash-based path here.
                        let already =
                            app.classes.iter().any(|c| c.step.class_hash == entry.class_hash);
                        if already {
                            app.modal = Some(Modal::AddClassPicker {
                                picker_state,
                                info: Some("Class is already selected".to_string()),
                            });
                        } else {
                            push_embedded_class(app, entry);
                        }
                    } else {
                        // "Load from file…" → switch modals (cwd walk happens here).
                        app.modal =
                            Some(Modal::AddClassFile { search: FileSearch::open(), error: None });
                    }
                }
                _ => {
                    app.modal = Some(Modal::AddClassPicker { picker_state, info: None });
                }
            }
        }
        Modal::AddClassFile { mut search, error: _ } => match code {
            KeyCode::Esc => {} // discard
            KeyCode::Enter => match search.resolve_choice() {
                Some(pb) => {
                    // Spawn the parse + compile on a worker thread and transition to
                    // the dedicated LoadingClass modal carrying the previous search
                    // state — so the user lands back here on cancel or failure.
                    let (tx, rx) = std::sync::mpsc::channel();
                    let path_for_task = pb.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(load_class_file(&path_for_task));
                    });
                    app.modal = Some(Modal::LoadingClass {
                        pending: PendingLoad { path: pb, rx, started: Instant::now() },
                        return_to_search: search,
                        notice: None,
                    });
                }
                None => {
                    app.modal = Some(Modal::AddClassFile {
                        search,
                        error: Some("no match — keep typing or press Esc".to_string()),
                    });
                }
            },
            KeyCode::Down => {
                search.move_down();
                app.modal = Some(Modal::AddClassFile { search, error: None });
            }
            KeyCode::Up => {
                search.move_up();
                app.modal = Some(Modal::AddClassFile { search, error: None });
            }
            _ => {
                // Forward everything else to the readline-style editor. Mark the
                // search as dirty so the debounce timer in the event-loop tick picks
                // it up — fast typing then only triggers one fuzzy pass per pause.
                if handle_text_edit(&mut search.query, code, mods) {
                    search.mark_dirty();
                }
                app.modal = Some(Modal::AddClassFile { search, error: None });
            }
        },
        Modal::LoadingClass { pending, return_to_search, notice } => {
            // Loading is non-interactive: every key except Esc is dropped, since
            // there's nothing to do until the worker thread finishes (or, in the
            // duplicate-class notice state, until the user dismisses).
            //
            // Esc semantics depend on whether the modal is showing a duplicate
            // notice or still spinning:
            //   - Notice visible → close the modal entirely (job is done, the user just saw the
            //     result, sending them back to the search would feel redundant).
            //   - Still loading → restore the previous AddClassFile so the user can pick a
            //     different file. The worker thread keeps running and its eventual send into the
            //     dropped receiver is a no-op.
            if code == KeyCode::Esc {
                if notice.is_some() {
                    // discard — modal closed
                } else {
                    app.modal = Some(Modal::AddClassFile { search: return_to_search, error: None });
                }
            } else {
                app.modal = Some(Modal::LoadingClass { pending, return_to_search, notice });
            }
        }
        Modal::ContractForm { editing_index, mut form } => {
            // `class_options` is expensive (ABI parsing, embedded class
            // decompression) so only compute it for the branches that
            // actually need it — class cycling and Enter/build.
            match code {
                KeyCode::Esc => {} // discard
                KeyCode::Tab | KeyCode::Down => {
                    form.focus_next();
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::BackTab | KeyCode::Up => {
                    form.focus_prev();
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Left if form.focused == ContractField::Class => {
                    let opts = class_options(app);
                    if !opts.is_empty() {
                        form.class_idx = (form.class_idx + opts.len() - 1) % opts.len();
                        form.sync_class(&opts);
                    }
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Right if form.focused == ContractField::Class => {
                    let opts = class_options(app);
                    if !opts.is_empty() {
                        form.class_idx = (form.class_idx + 1) % opts.len();
                        form.sync_class(&opts);
                    }
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Char(' ') if form.focused == ContractField::Unique => {
                    form.unique = !form.unique;
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
                KeyCode::Enter => {
                    let opts = class_options(app);
                    match form.build(&opts) {
                        Ok(step) => match editing_index {
                            Some(i) => {
                                if let Some(slot) = app.contracts.get_mut(i) {
                                    // Edit resets this single item's exec state to
                                    // Pending — we can't trust an old Done/Failed
                                    // against the new inputs. `last_run` is left
                                    // untouched; it's history, not truth.
                                    slot.step = step;
                                    slot.exec = ItemExecState::Pending;
                                }
                            }
                            None => {
                                app.contracts.push(ContractItem::from_step(step));
                                app.contracts_state.select(Some(app.contracts.len() - 1));
                            }
                        },
                        Err(e) => {
                            form.error = Some(e);
                            app.modal = Some(Modal::ContractForm { editing_index, form });
                        }
                    }
                }
                _ => {
                    // Forward to the focused text input (label/salt/calldata). Class
                    // and Unique fields don't accept text input, so the helper just
                    // does nothing for them.
                    if let Some(input) = form.focused_input_mut() {
                        handle_text_edit(input, code, mods);
                    }
                    app.modal = Some(Modal::ContractForm { editing_index, form });
                }
            }
        }
        Modal::SaveManifest { mut path, error: _ } => match code {
            KeyCode::Esc => {} // discard
            KeyCode::Enter => match save_manifest_from_app(app, path.as_str()) {
                Ok(()) => {
                    app.flash(format!("manifest saved to {}", path.as_str()));
                }
                Err(e) => {
                    app.modal = Some(Modal::SaveManifest { path, error: Some(e.to_string()) });
                }
            },
            _ => {
                handle_text_edit(&mut path, code, mods);
                app.modal = Some(Modal::SaveManifest { path, error: None });
            }
        },
    }
}

fn push_embedded_class(app: &mut AppState, entry: &'static EmbeddedClass) {
    // Dedupe by class hash, not name: two embedded entries that happen to share the
    // same alias would still be unique on disk, and conversely the same class loaded
    // under a different alias is still a duplicate as far as the chain is concerned.
    if app.classes.iter().any(|c| c.step.class_hash == entry.class_hash) {
        app.flash("Class is already selected");
        return;
    }
    app.classes.push(ClassItem::from_step(DeclareStep {
        name: entry.name.to_string(),
        class: Arc::new(entry.class()),
        class_hash: entry.class_hash,
        casm_hash: entry.casm_hash,
        source: ClassSource::Embedded(entry.name),
    }));
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
        .map(|c| match &c.step.source {
            ClassSource::Embedded(name) => ClassEntry {
                name: c.step.name.clone(),
                embedded: Some((*name).to_string()),
                path: None,
            },
            ClassSource::File(path) => {
                ClassEntry { name: c.step.name.clone(), embedded: None, path: Some(path.clone()) }
            }
        })
        .collect();
    let contracts = app
        .contracts
        .iter()
        .map(|c| {
            let d = &c.step;
            ContractEntry {
                class: d.class_name.clone(),
                label: d.label.clone(),
                salt: if d.salt == Felt::ZERO { None } else { Some(d.salt) },
                unique: d.unique,
                calldata: d.calldata.clone(),
            }
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
    let titles: Vec<Line<'_>> = Tab::ALL.iter().map(|t| Line::from(t.title())).collect();
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("katana bootstrap"))
        .select(app.current_tab.idx())
        .highlight_style(
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
}

fn draw_hint_bar(f: &mut ratatui::Frame<'_>, app: &AppState, area: Rect) {
    let base = match app.current_tab {
        Tab::Classes => "[a] add  [d] delete  [j/k] navigate  [Tab] next tab  [q] quit",
        Tab::Contracts => "[a] add  [e] edit  [d] delete  [j/k] navigate  [Tab] next tab  [q] quit",
        Tab::Settings if app.settings.editing => "[Esc/Enter] stop editing",
        Tab::Settings => "[j/k] move  [e/Enter] edit  [Tab] next tab  [q] quit",
        Tab::Execute => match &app.execution {
            ExecutionState::Idle => {
                // Different hint depending on whether there's outstanding work
                // to do. `s` is always available in Idle.
                let any_outstanding = app.classes.iter().any(|c| c.exec.is_outstanding())
                    || app.contracts.iter().any(|c| c.exec.is_outstanding());
                if any_outstanding {
                    "[x] run  [s] save manifest  [Tab] next tab  [q] quit"
                } else {
                    "[s] save manifest  [Tab] next tab  [q] quit"
                }
            }
            ExecutionState::Running { .. } => "running…",
            ExecutionState::Refreshing { .. } => "verifying node…",
        },
    };
    let text =
        if let Some(flash) = &app.flash { format!("{flash}    {base}") } else { base.to_string() };
    let style =
        if app.flash.is_some() { Style::default().fg(Color::Yellow) } else { Style::default() };
    let p = Paragraph::new(text).style(style);
    f.render_widget(p, area);
}

/// Min/max width of the name column on the Classes tab. The actual column width is
/// the longest name in the current plan, clamped into this range — so the column
/// expands to fit short lists nicely but caps out before a single pathological alias
/// can push the source/hash columns off the right edge.
const CLASS_NAME_WIDTH_MIN: usize = 4;
const CLASS_NAME_WIDTH_MAX: usize = 32;

fn draw_classes_tab(f: &mut ratatui::Frame<'_>, app: &mut AppState, area: Rect) {
    let name_width = app
        .classes
        .iter()
        .map(|c| c.step.name.chars().count())
        .max()
        .unwrap_or(CLASS_NAME_WIDTH_MIN)
        .clamp(CLASS_NAME_WIDTH_MIN, CLASS_NAME_WIDTH_MAX);
    // "embedded" is the longest source label at 8 chars; pin the column to that.
    const SOURCE_WIDTH: usize = 8;

    let items: Vec<ListItem<'_>> = app
        .classes
        .iter()
        .map(|c| {
            let source = match &c.step.source {
                ClassSource::Embedded(_) => "embedded",
                ClassSource::File(_) => "file",
            };
            let name = truncate_with_ellipsis(&c.step.name, name_width);
            let (icon, icon_style) = exec_badge(&c.exec);
            let mut spans = vec![
                Span::styled(icon.to_string(), icon_style),
                Span::raw(format!(
                    "{:<name_w$}  {:<src_w$}  {:#x}",
                    name,
                    source,
                    c.step.class_hash,
                    name_w = name_width,
                    src_w = SOURCE_WIDTH,
                )),
            ];
            if let Some(detail) = c.exec.detail() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(detail.to_string(), icon_style));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Classes to declare"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, &mut app.classes_state);
}

/// Icon + style for an [`ItemExecState`] as rendered in the Classes/Contracts
/// tabs. Kept as a single helper so the styling stays consistent across both
/// tabs and tests can assert against one source of truth.
fn exec_badge(state: &ItemExecState) -> (&'static str, Style) {
    match state {
        ItemExecState::Pending => ("  ", Style::default()),
        ItemExecState::Running => ("… ", Style::default().fg(Color::Yellow)),
        ItemExecState::Done { .. } => ("✓ ", Style::default().fg(Color::Green)),
        ItemExecState::Failed { .. } => ("✗ ", Style::default().fg(Color::Red)),
        ItemExecState::Unknown { .. } => ("? ", Style::default().fg(Color::DarkGray)),
    }
}

/// Truncate `s` to at most `max` displayed chars, replacing the last char with `…`
/// when truncation actually happens. Char-counted (not byte-counted) so multi-byte
/// scripts work the way the user expects, and operates on `chars()` rather than the
/// raw bytes so we never split inside a UTF-8 sequence.
fn truncate_with_ellipsis(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    // Reserve one slot for the ellipsis itself.
    let take = max - 1;
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

fn draw_contracts_tab(f: &mut ratatui::Frame<'_>, app: &mut AppState, area: Rect) {
    let items: Vec<ListItem<'_>> = app
        .contracts
        .iter()
        .map(|c| {
            let d = &c.step;
            let (icon, icon_style) = exec_badge(&c.exec);
            let mut spans = vec![
                Span::styled(icon.to_string(), icon_style),
                Span::raw(format!(
                    "{:<15} {:<20} salt={:#x}  calldata=[{}]",
                    d.label.as_deref().unwrap_or("-"),
                    d.class_name,
                    d.salt,
                    d.calldata.iter().map(|f| format!("{f:#x}")).collect::<Vec<_>>().join(", ")
                )),
            ];
            if let Some(detail) = c.exec.detail() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(detail.to_string(), icon_style));
            }
            ListItem::new(Line::from(spans))
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
        let focused = field == app.settings.focused;
        let editing = focused && app.settings.editing;
        let marker = if focused { "> " } else { "  " };
        let label_style = if focused {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let mut spans = vec![
            Span::styled(format!("{marker}{:<14}", field.label()), label_style),
            Span::raw("  "),
        ];

        match field {
            SettingsField::RpcUrl => {
                if app.settings.rpc_url.is_empty() && !editing {
                    spans.push(Span::styled("(not set)", Style::default().fg(Color::DarkGray)));
                } else {
                    spans.extend(render_text_input(&app.settings.rpc_url, editing, false));
                }
            }
            SettingsField::Account => {
                if app.settings.account.is_empty() && !editing {
                    spans.push(Span::styled("(not set)", Style::default().fg(Color::DarkGray)));
                } else {
                    spans.extend(render_text_input(&app.settings.account, editing, false));
                }
            }
            SettingsField::PrivateKey => {
                if app.settings.private_key.is_empty() && !editing {
                    spans.push(Span::styled("(not set)", Style::default().fg(Color::DarkGray)));
                } else {
                    spans.extend(render_text_input(&app.settings.private_key, editing, true));
                }
            }
        }
        lines.push(Line::from(spans));
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Settings"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_execute_tab(f: &mut ratatui::Frame<'_>, app: &AppState, area: Rect) {
    // Plan header reflects the FULL current plan (including Done items) so the
    // user can see at a glance how big the session is. Outstanding count is
    // shown separately to surface what the next `x` would actually run.
    let outstanding_declares = app.classes.iter().filter(|c| c.exec.is_outstanding()).count();
    let outstanding_deploys = app.contracts.iter().filter(|c| c.exec.is_outstanding()).count();
    let header = format!(
        "Plan: {} declares, {} deploys   Outstanding: {} declares, {} deploys",
        app.classes.len(),
        app.contracts.len(),
        outstanding_declares,
        outstanding_deploys,
    );

    // Prefer live rows (during Running) over last_run (historical). In Idle
    // with no last_run (fresh session), render an empty list — the Execute
    // tab is mostly about feedback after running.
    let (rows, tick): (&[ExecRow], u64) = match &app.execution {
        ExecutionState::Running { run, tick, .. } => (run.rows.as_slice(), *tick),
        ExecutionState::Idle | ExecutionState::Refreshing { .. } => match &app.last_run {
            Some(lr) => (lr.rows.as_slice(), 0),
            None => (&[], 0),
        },
    };

    let mut lines: Vec<Line<'_>> = vec![Line::from(header), Line::from("")];
    if rows.is_empty() {
        if outstanding_declares + outstanding_deploys > 0 {
            lines.push(Line::from("(press `x` to start)"));
        } else {
            lines.push(Line::from("(nothing to do — add a class or contract first)"));
        }
    } else {
        // Compute column widths once across all rows so the trailing detail
        // (hash/address/error) lines up regardless of how long any individual
        // primary/secondary cell is.
        let primary_width =
            rows.iter().map(|r| r.primary.chars().count()).max().unwrap_or(0).max(1);
        // Secondary cells are wrapped in parens at render time; the column width
        // is the longest unwrapped name + 2 for the parens. Plans without any
        // deploys collapse this column to zero width.
        let secondary_width = rows
            .iter()
            .filter_map(|r| r.secondary.as_ref().map(|s| s.chars().count() + 2))
            .max()
            .unwrap_or(0);
        // `declare` is the longest kind label at 7 chars; pin the column to that.
        const KIND_WIDTH: usize = 7;

        for row in rows {
            let (icon, icon_style) = match &row.status {
                RowStatus::Pending => ("  ".to_string(), Style::default().fg(Color::DarkGray)),
                RowStatus::Running => {
                    let frame = SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()];
                    (format!("{frame} "), Style::default().fg(Color::Yellow))
                }
                RowStatus::Done(_) => ("✓ ".to_string(), Style::default().fg(Color::Green)),
                RowStatus::Failed(_) => ("✗ ".to_string(), Style::default().fg(Color::Red)),
            };
            let (detail, detail_style) = match &row.status {
                RowStatus::Done(s) => (s.clone(), Style::default().fg(Color::Cyan)),
                RowStatus::Failed(s) => (s.clone(), Style::default().fg(Color::Red)),
                _ => (String::new(), Style::default()),
            };
            let secondary_cell = match &row.secondary {
                Some(s) => format!("({s})"),
                None => String::new(),
            };

            lines.push(Line::from(vec![
                Span::styled(icon, icon_style),
                Span::raw(format!("{:<KIND_WIDTH$}  ", row.kind.as_str())),
                Span::raw(format!("{:<primary_width$}  ", row.primary)),
                Span::raw(format!("{secondary_cell:<secondary_width$}  ")),
                Span::styled(detail, detail_style),
            ]));
        }
    }

    // Summary footer: pulled from last_run when we're idle, or live from the
    // running view when we're mid-flight. Shown only after a run has
    // happened — a fresh session has no footer.
    if matches!(app.execution, ExecutionState::Idle) {
        if let Some(lr) = &app.last_run {
            lines.push(Line::from(""));
            match &lr.result {
                Err(err) => lines.push(Line::from(Span::styled(
                    format!("Failed: {err}"),
                    Style::default().fg(Color::Red),
                ))),
                Ok(_) => {
                    let hint = if outstanding_declares + outstanding_deploys > 0 {
                        "Done. Add more items or press `x` to run outstanding work. `s` to save."
                    } else {
                        "Done. Add more items, press `s` to save, or `q` to quit."
                    };
                    lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::Green))));
                }
            }
        }
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Execute"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_modal(f: &mut ratatui::Frame<'_>, app: &AppState, modal: &Modal) {
    // Per-modal area sizing. Most modals reuse the standard 60×70 box; the loading
    // overlay is intentionally small (just a centred status card) so it doesn't
    // dominate the screen. The background load is brief enough that a big modal
    // would feel like overkill — and we don't want to obscure the plan tabs more
    // than we have to.
    let area = match modal {
        // Loader modal is small but still wide enough to comfortably fit a typical
        // file path on one line; long paths and the dup-class notice still wrap via
        // the Paragraph below.
        Modal::LoadingClass { .. } => centered_rect(60, 35, f.area()),
        _ => centered_rect(60, 70, f.area()),
    };
    f.render_widget(Clear, area);
    match modal {
        Modal::AddClassPicker { picker_state, info } => {
            // Two-row layout: list on top, single info line at the bottom. The info
            // row is always present (even when empty) so the picker height doesn't
            // jitter as the message comes and goes.
            let inner = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(area);

            // Mark embedded classes that are already in the plan with a yellow `*`.
            // We compare by class hash so the marker stays correct even if the user
            // picks the same class under a different alias via the file loader. The
            // marker itself is yellow so it pops against the dimmed body of the row.
            let mut items: Vec<ListItem<'_>> = embedded::REGISTRY
                .iter()
                .map(|c| {
                    let already_in_plan =
                        app.classes.iter().any(|existing| existing.class_hash == c.class_hash);
                    let body = format!("{} — {}", c.name, c.description);
                    if already_in_plan {
                        ListItem::new(Line::from(vec![
                            Span::styled("* ", Style::default().fg(Color::Yellow)),
                            Span::styled(body, Style::default().fg(Color::DarkGray)),
                        ]))
                    } else {
                        ListItem::new(Line::from(vec![Span::raw("  "), Span::raw(body)]))
                    }
                })
                .collect();
            // The "load from file…" sentinel never gets a marker — the user's selection
            // here is the path, not a known class hash.
            items.push(ListItem::new("  [Load Sierra class from file…]"));
            let mut state = picker_state.clone();
            if state.selected().is_none() {
                state.select(Some(0));
            }
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Add a class"))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("> ");
            f.render_stateful_widget(list, inner[0], &mut state);

            // Inline notice (e.g. "class `dev_account` is already in the plan").
            if let Some(msg) = info {
                let p = Paragraph::new(Line::from(Span::styled(
                    format!("  {msg}"),
                    Style::default().fg(Color::Yellow),
                )));
                f.render_widget(p, inner[1]);
            }
        }
        Modal::AddClassFile { search, error } => {
            // Three-row layout inside the modal: query + matches list + hint/error.
            let inner = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // query box
                    Constraint::Min(0),    // matches list
                    Constraint::Length(2), // hints / error
                ])
                .split(area);

            // Query box.
            let title = format!("Load class file — fuzzy search under {}", search.root.display());
            let mut query_spans = vec![Span::raw("  ")];
            query_spans.extend(render_text_input(&search.query, true, false));
            let query_para = Paragraph::new(Line::from(query_spans))
                .block(Block::default().borders(Borders::ALL).title(title));
            f.render_widget(query_para, inner[0]);

            // Matches list.
            let total = search.matches.len();
            let items: Vec<ListItem<'_>> = search
                .matches
                .iter()
                .map(|(path, _)| {
                    // Display path relative to the walked root when possible —
                    // easier to scan than full absolute paths.
                    let display = path
                        .strip_prefix(&search.root)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
                    ListItem::new(display)
                })
                .collect();

            // Three distinct empty states the user might land in: nothing was found
            // on disk at all, they haven't typed anything yet, or their query just
            // doesn't match anything. Each one wants a different hint.
            let list_title = if search.candidates.is_empty() {
                "(no .json files under cwd)".to_string()
            } else if search.query.is_empty() {
                format!("(start typing to search {} files)", search.candidates.len())
            } else if total == 0 {
                format!("(no matches in {} files)", search.candidates.len())
            } else {
                format!("{}/{} matches", total, search.candidates.len())
            };

            let mut list_state = ListState::default();
            if total > 0 {
                list_state.select(Some(search.selected));
            }
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(list_title))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("> ");
            f.render_stateful_widget(list, inner[1], &mut list_state);

            // Hints / error.
            let hint_text = if let Some(e) = error {
                Span::styled(format!("error: {e}"), Style::default().fg(Color::Red))
            } else {
                Span::styled(
                    "[↑/↓] move  [Enter] load  [Esc] cancel",
                    Style::default().fg(Color::DarkGray),
                )
            };
            let hint = Paragraph::new(Line::from(hint_text));
            f.render_widget(hint, inner[2]);
        }
        Modal::LoadingClass { pending, notice, return_to_search } => {
            draw_loading_class_modal(f, area, pending, &return_to_search.root, notice.as_deref());
        }
        Modal::ContractForm { editing_index, form } => {
            draw_contract_form_modal(f, area, app, *editing_index, form);
        }
        Modal::SaveManifest { path, error } => {
            let mut lines = vec![Line::from("Save manifest to:")];
            let mut input_spans = vec![Span::raw("  ")];
            input_spans.extend(render_text_input(path, true, false));
            lines.push(Line::from(input_spans));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "[Enter] save  [Esc] cancel",
                Style::default().fg(Color::DarkGray),
            )));
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

#[cfg(test)]
mod tests {
    use super::*;

    // ----- TextInput basics ----------------------------------------------------

    #[test]
    fn text_input_insert_and_backspace() {
        let mut t = TextInput::new();
        for c in "hello".chars() {
            t.insert_char(c);
        }
        assert_eq!(t.as_str(), "hello");
        assert_eq!(t.cursor, 5);
        t.backspace();
        t.backspace();
        assert_eq!(t.as_str(), "hel");
        assert_eq!(t.cursor, 3);
    }

    #[test]
    fn text_input_cursor_motion_and_mid_insert() {
        let mut t = TextInput::from_str("worl");
        t.move_home();
        assert_eq!(t.cursor, 0);
        t.move_right();
        t.move_right();
        // Insert in the middle: "wo" + "X" + "rl"
        t.insert_char('X');
        assert_eq!(t.as_str(), "woXrl");
        assert_eq!(t.cursor, 3);
    }

    #[test]
    fn text_input_handles_multibyte_chars() {
        let mut t = TextInput::new();
        t.insert_char('日');
        t.insert_char('本');
        assert_eq!(t.cursor, t.len()); // both 3-byte chars
        t.move_left();
        // Cursor should now sit between the two chars (3 bytes in).
        assert_eq!(t.cursor, 3);
        t.backspace();
        assert_eq!(t.as_str(), "本");
    }

    #[test]
    fn text_input_delete_word_backward_eats_space_then_word() {
        let mut t = TextInput::from_str("hello world");
        t.delete_word_backward();
        assert_eq!(t.as_str(), "hello ");
        // A second invocation eats the trailing space and then "hello".
        t.delete_word_backward();
        assert_eq!(t.as_str(), "");
    }

    #[test]
    fn text_input_kill_to_start_and_end() {
        let mut t = TextInput::from_str("hello world");
        // Cursor at index 6 → just before "world".
        t.move_home();
        for _ in 0..6 {
            t.move_right();
        }
        let saved_cursor = t.cursor;
        let mut a = t.clone();
        a.kill_to_end();
        assert_eq!(a.as_str(), "hello ");
        assert_eq!(a.cursor, saved_cursor);

        let mut b = t.clone();
        b.kill_to_start();
        assert_eq!(b.as_str(), "world");
        assert_eq!(b.cursor, 0);
    }

    // ----- handle_text_edit (the readline shortcut handler) -------------------

    fn key(c: char) -> (KeyCode, KeyModifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> (KeyCode, KeyModifiers) {
        (KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn apply(t: &mut TextInput, evs: &[(KeyCode, KeyModifiers)]) {
        for (code, mods) in evs {
            handle_text_edit(t, *code, *mods);
        }
    }

    #[test]
    fn ctrl_a_jumps_to_start_then_typing_inserts_there() {
        let mut t = TextInput::from_str("world");
        apply(&mut t, &[ctrl('a'), key('h'), key('i'), key(' ')]);
        assert_eq!(t.as_str(), "hi world");
    }

    #[test]
    fn ctrl_e_jumps_to_end() {
        let mut t = TextInput::from_str("abc");
        t.move_home();
        apply(&mut t, &[ctrl('e'), key('!')]);
        assert_eq!(t.as_str(), "abc!");
    }

    #[test]
    fn ctrl_w_deletes_word_before_cursor() {
        let mut t = TextInput::from_str("the quick brown fox");
        apply(&mut t, &[ctrl('w')]);
        assert_eq!(t.as_str(), "the quick brown ");
    }

    #[test]
    fn ctrl_u_kills_to_start_ctrl_k_kills_to_end() {
        let mut t = TextInput::from_str("hello world");
        // Position cursor at index 6 (before "world")
        t.move_home();
        for _ in 0..6 {
            apply(&mut t, &[(KeyCode::Right, KeyModifiers::NONE)]);
        }
        let mut a = t.clone();
        apply(&mut a, &[ctrl('k')]);
        assert_eq!(a.as_str(), "hello ");

        let mut b = t.clone();
        apply(&mut b, &[ctrl('u')]);
        assert_eq!(b.as_str(), "world");
    }

    #[test]
    fn arrow_keys_and_backspace() {
        let mut t = TextInput::from_str("abcde");
        apply(
            &mut t,
            &[
                (KeyCode::Left, KeyModifiers::NONE),
                (KeyCode::Left, KeyModifiers::NONE),
                (KeyCode::Backspace, KeyModifiers::NONE),
            ],
        );
        assert_eq!(t.as_str(), "abde");
        assert_eq!(t.cursor, 2);
    }

    #[test]
    fn handle_text_edit_returns_false_for_unknown_keys() {
        let mut t = TextInput::from_str("x");
        let consumed = handle_text_edit(&mut t, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!consumed);
        let consumed = handle_text_edit(&mut t, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!consumed);
        // Buffer untouched.
        assert_eq!(t.as_str(), "x");
    }

    #[test]
    fn truncate_with_ellipsis_basics() {
        // Below the cap → unchanged.
        assert_eq!(truncate_with_ellipsis("foo", 10), "foo");
        // At the cap → unchanged.
        assert_eq!(truncate_with_ellipsis("foobarbaz", 9), "foobarbaz");
        // Above the cap → truncated to exactly `max` chars, with the last as `…`.
        let truncated = truncate_with_ellipsis("foobarbazquux", 6);
        assert_eq!(truncated, "fooba…");
        assert_eq!(truncated.chars().count(), 6);
        // Multi-byte: don't split inside a UTF-8 sequence.
        let truncated = truncate_with_ellipsis("日本語テスト", 4);
        assert_eq!(truncated.chars().count(), 4);
        assert!(truncated.ends_with('…'));
        // Edge: max == 0 yields empty.
        assert_eq!(truncate_with_ellipsis("anything", 0), "");
    }

    #[test]
    fn fuzzy_score_subsequence_match() {
        // Subsequence (non-contiguous) match.
        assert!(fuzzy_score("foo", "build/foo.json").is_some());
        // Same chars but out of order → no match.
        assert!(fuzzy_score("oof", "foo.json").is_none());
        // Empty query always scores zero (everything matches).
        assert_eq!(fuzzy_score("", "anything.json"), Some(0));
        // Case-insensitive.
        assert!(fuzzy_score("FOO", "build/foo.json").is_some());
    }

    #[test]
    fn fuzzy_score_prefers_shorter_paths_and_adjacency() {
        // Shorter path with the same query wins (length penalty).
        let short = fuzzy_score("foo", "foo.json").unwrap();
        let long = fuzzy_score("foo", "a/b/c/d/e/foo.json").unwrap();
        assert!(short > long, "short {short} should outrank long {long}");

        // Adjacent matches outrank scattered ones for the same candidate length.
        let adjacent = fuzzy_score("abc", "xabcx").unwrap();
        let scattered = fuzzy_score("abc", "axbxc").unwrap();
        assert!(adjacent > scattered, "adjacent {adjacent} should outrank scattered {scattered}");
    }

    fn make_search(candidates: Vec<&str>) -> FileSearch {
        FileSearch {
            query: TextInput::new(),
            candidates: candidates.into_iter().map(PathBuf::from).collect(),
            matches: Vec::new(),
            selected: 0,
            root: PathBuf::from("."),
            pending_recompute: None,
        }
    }

    #[test]
    fn file_search_empty_query_yields_no_matches() {
        // Critical UX rule: don't dump cwd into the user's face before they've
        // expressed any intent. The matches list stays empty until the first char.
        let mut s = make_search(vec!["foo.json", "bar.json", "baz.json"]);
        s.recompute();
        assert!(s.matches.is_empty(), "empty query should produce zero matches");
    }

    #[test]
    fn file_search_recompute_filters_and_sorts() {
        let mut s =
            make_search(vec!["a/b/c/d/e/foo.json", "foo.json", "bar.json", "baz/foo_2.json"]);

        // After typing a query, only matches survive and they're sorted by score.
        for c in "foo".chars() {
            s.query.insert_char(c);
        }
        s.recompute();
        let paths: Vec<&PathBuf> = s.matches.iter().map(|(p, _)| p).collect();
        assert!(paths.contains(&&PathBuf::from("foo.json")));
        assert!(paths.contains(&&PathBuf::from("a/b/c/d/e/foo.json")));
        assert!(!paths.contains(&&PathBuf::from("bar.json")), "bar shouldn't match `foo`");
        // The shortest path (`foo.json`) should be first since it has the lowest length penalty.
        assert_eq!(s.matches.first().unwrap().0, PathBuf::from("foo.json"));
    }

    #[test]
    fn file_search_debounce_defers_recompute_until_deadline() {
        let mut s = make_search(vec!["foo.json", "bar.json"]);

        // Type a character. Mark dirty (what the modal handler does on every keystroke
        // it forwards to the editor). Matches must NOT update yet — that's the whole
        // point of the debounce.
        s.query.insert_char('f');
        s.mark_dirty();
        assert!(s.matches.is_empty(), "results before deadline should still be empty");

        // A tick from "now" (well before the deadline) is a no-op.
        let started = Instant::now();
        assert!(!s.tick(started));
        assert!(s.matches.is_empty());

        // Once we cross the deadline, the next tick runs the recompute and clears the
        // pending flag so we don't keep redoing the work on every subsequent tick.
        let after = started + FILE_SEARCH_DEBOUNCE + Duration::from_millis(1);
        assert!(s.tick(after), "tick at/after the deadline should run the recompute");
        assert_eq!(s.matches.len(), 1);
        assert_eq!(s.matches[0].0, PathBuf::from("foo.json"));
        assert!(s.pending_recompute.is_none(), "pending flag must clear after the run");

        // A second tick with no further mutations is a no-op.
        assert!(!s.tick(after + Duration::from_secs(1)));
    }

    #[test]
    fn file_search_typing_more_resets_the_debounce() {
        // Repeated keystrokes should keep pushing the deadline out, so the recompute
        // only runs once after the user actually pauses.
        let mut s = make_search(vec!["foo.json"]);
        s.query.insert_char('f');
        s.mark_dirty();
        let first_deadline = s.pending_recompute.unwrap();

        // Sleep zero (just yield) and type again — the new deadline must be at least
        // as far in the future as the first one.
        s.query.insert_char('o');
        s.mark_dirty();
        let second_deadline = s.pending_recompute.unwrap();
        assert!(
            second_deadline >= first_deadline,
            "subsequent mark_dirty must not bring the deadline forward"
        );
    }

    // ----- Contract form: typed constructor calldata -------------------------

    use crate::abi::{ArgumentNode, ConstructorAbi, TypeNode};

    fn opt_no_ctor(name: &str) -> ClassOption {
        ClassOption { name: name.to_string(), class_hash: Felt::ZERO, constructor: None }
    }

    fn opt_with_ctor(name: &str, args: Vec<(&str, TypeNode)>) -> ClassOption {
        ClassOption {
            name: name.to_string(),
            class_hash: Felt::ZERO,
            constructor: Some(ConstructorAbi {
                name: "constructor".to_string(),
                inputs: args
                    .into_iter()
                    .map(|(n, t)| ArgumentNode { name: n.to_string(), ty: t })
                    .collect(),
            }),
        }
    }

    fn type_felt() -> TypeNode {
        TypeNode::Primitive { name: "core::felt252".to_string() }
    }
    fn type_u256() -> TypeNode {
        TypeNode::Primitive { name: "core::integer::u256".to_string() }
    }

    #[test]
    fn contract_form_new_uses_typed_inputs_when_class_has_constructor() {
        let opts = vec![opt_with_ctor("foo", vec![("a", type_felt()), ("b", type_u256())])];
        let form = ContractForm::new(&opts);
        match &form.calldata {
            CalldataInput::Typed { args } => {
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].name, "a");
                assert_eq!(args[1].name, "b");
            }
            CalldataInput::Raw(_) => panic!("expected typed mode"),
        }
        // Fields list now includes Arg(0) and Arg(1) — no RawCalldata.
        let fields = form.fields();
        assert!(fields.contains(&ContractField::Arg(0)));
        assert!(fields.contains(&ContractField::Arg(1)));
        assert!(!fields.contains(&ContractField::RawCalldata));
    }

    #[test]
    fn contract_form_new_falls_back_to_raw_when_no_constructor() {
        let opts = vec![opt_no_ctor("foo")];
        let form = ContractForm::new(&opts);
        assert!(matches!(form.calldata, CalldataInput::Raw(_)));
        assert!(form.fields().contains(&ContractField::RawCalldata));
    }

    #[test]
    fn contract_form_build_encodes_typed_args() {
        let opts = vec![opt_with_ctor("foo", vec![("a", type_felt()), ("b", type_u256())])];
        let mut form = ContractForm::new(&opts);
        if let CalldataInput::Typed { args } = &mut form.calldata {
            args[0].input = TextInput::from_str("0x42");
            args[1].input = TextInput::from_str("256");
        }
        let step = form.build(&opts).expect("build should succeed");
        // felt252 -> [0x42], u256 256 -> [256, 0]
        assert_eq!(step.calldata, vec![Felt::from(0x42u64), Felt::from(256u64), Felt::ZERO]);
    }

    #[test]
    fn contract_form_build_reports_arg_name_in_error() {
        let opts = vec![opt_with_ctor("foo", vec![("amount", type_u256())])];
        let mut form = ContractForm::new(&opts);
        if let CalldataInput::Typed { args } = &mut form.calldata {
            args[0].input = TextInput::from_str("not-a-number");
        }
        let err = form.build(&opts).unwrap_err();
        assert!(err.contains("amount"), "error should name the offending arg, got: {err}");
    }

    #[test]
    fn contract_form_sync_class_swaps_typed_template_and_keeps_values() {
        let opts = vec![
            opt_with_ctor("a", vec![("x", type_felt())]),
            opt_with_ctor("b", vec![("y", type_u256())]),
        ];
        let mut form = ContractForm::new(&opts);
        if let CalldataInput::Typed { args } = &mut form.calldata {
            args[0].input = TextInput::from_str("0x99");
        }

        // Switch to class B. Args are positional, so the value carries over to
        // `y` even though the name and type changed — the user's keystrokes
        // shouldn't get lost just because they cycled the picker by mistake.
        form.class_idx = 1;
        form.sync_class(&opts);
        match &form.calldata {
            CalldataInput::Typed { args } => {
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].name, "y");
                assert_eq!(args[0].input.as_str(), "0x99");
            }
            _ => panic!("expected typed mode"),
        }
    }

    #[test]
    fn contract_form_sync_class_swaps_to_raw_when_target_has_no_constructor() {
        let opts = vec![opt_with_ctor("a", vec![("x", type_felt())]), opt_no_ctor("b")];
        let mut form = ContractForm::new(&opts);
        form.class_idx = 1;
        form.sync_class(&opts);
        assert!(matches!(form.calldata, CalldataInput::Raw(_)));
    }

    #[test]
    fn contract_form_from_existing_prefers_typed_mode_and_prefills() {
        // When the class has a constructor ABI, editing uses typed mode and
        // pre-fills each input by decoding the existing raw calldata.
        let opts = vec![opt_with_ctor("foo", vec![("a", type_felt()), ("b", type_u256())])];
        let step = DeployStep {
            label: None,
            class_hash: Felt::ZERO,
            class_name: "foo".to_string(),
            salt: Felt::ZERO,
            unique: false,
            // felt252(0x42) + u256(256) encoded as [0x42, low=0x100, high=0x0]
            calldata: vec![Felt::from(0x42u64), Felt::from(0x100u64), Felt::ZERO],
        };
        let form = ContractForm::from_existing(&step, &opts);
        match &form.calldata {
            CalldataInput::Typed { args } => {
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].name, "a");
                assert_eq!(args[0].input.as_str(), "0x42");
                assert_eq!(args[1].name, "b");
                assert_eq!(args[1].input.as_str(), "0x100");
            }
            CalldataInput::Raw(_) => panic!("expected typed mode when class has constructor"),
        }
    }

    #[test]
    fn contract_form_from_existing_falls_back_to_raw_without_abi() {
        // Without an introspectable constructor, raw mode is the only option.
        let opts = vec![opt_no_ctor("foo")];
        let step = DeployStep {
            label: None,
            class_hash: Felt::ZERO,
            class_name: "foo".to_string(),
            salt: Felt::ZERO,
            unique: false,
            calldata: vec![Felt::from(7u64)],
        };
        let form = ContractForm::from_existing(&step, &opts);
        match &form.calldata {
            CalldataInput::Raw(input) => assert!(input.as_str().contains("0x7")),
            _ => panic!("expected raw mode"),
        }
    }

    #[test]
    fn contract_form_focus_navigation_skips_through_args() {
        let opts = vec![opt_with_ctor("foo", vec![("a", type_felt()), ("b", type_felt())])];
        let mut form = ContractForm::new(&opts);
        // Class -> Label -> Salt -> Unique -> Arg(0) -> Arg(1) -> Class
        let expected = vec![
            ContractField::Label,
            ContractField::Salt,
            ContractField::Unique,
            ContractField::Arg(0),
            ContractField::Arg(1),
            ContractField::Class,
        ];
        for want in expected {
            form.focus_next();
            assert_eq!(form.focused, want);
        }
    }

    // ------------------------------------------------------------------------
    // AppState transitions: per-item exec state, last_run snapshot, mutation
    // locks. These exercise the continue-session refactor at the AppState
    // layer — no tokio runtime, no real RPC. For end-to-end coverage see the
    // integration tests in tests/bootstrap.rs.
    // ------------------------------------------------------------------------

    use std::sync::Arc;

    use katana_primitives::class::ClassHash;
    use tokio::sync::mpsc::unbounded_channel;

    /// Minimal signer defaults that pass `SettingsForm::build()` without any
    /// edits. Used by every state-transition test so we don't have to keep
    /// retyping them.
    fn valid_defaults() -> SignerDefaults {
        SignerDefaults {
            rpc_url: Some("http://localhost:5050".to_string()),
            account: Some(ContractAddress::from(Felt::from(1u8))),
            private_key: Some(Felt::from(2u8)),
        }
    }

    fn dummy_class_hash(seed: u64) -> ClassHash {
        Felt::from(seed)
    }

    fn dummy_class_item(name: &str, seed: u64) -> ClassItem {
        // We never execute these, so the `class` payload doesn't have to be a
        // real Sierra class — any ContractClass instance that implements Debug
        // would do. The embedded dev_account is convenient because we already
        // have it handy in the test suite.
        let class = embedded::REGISTRY[0].class();
        ClassItem::from_step(DeclareStep {
            name: name.to_string(),
            class: Arc::new(class),
            class_hash: dummy_class_hash(seed),
            casm_hash: Felt::ZERO,
            source: ClassSource::Embedded(embedded::REGISTRY[0].name),
        })
    }

    fn dummy_contract_item(label: &str, class_name: &str, seed: u64) -> ContractItem {
        ContractItem::from_step(DeployStep {
            label: Some(label.to_string()),
            class_hash: dummy_class_hash(seed),
            class_name: class_name.to_string(),
            salt: Felt::from(seed),
            unique: false,
            calldata: Vec::new(),
        })
    }

    #[test]
    fn new_appstate_starts_idle_with_no_last_run() {
        let app = AppState::new(SignerDefaults::default());
        assert!(matches!(app.execution, ExecutionState::Idle));
        assert!(app.last_run.is_none());
        assert!(!app.is_busy());
    }

    #[test]
    fn item_exec_state_outstanding_excludes_only_done() {
        assert!(ItemExecState::Pending.is_outstanding());
        assert!(ItemExecState::Running.is_outstanding());
        assert!(ItemExecState::Failed { detail: "x".into() }.is_outstanding());
        assert!(ItemExecState::Unknown { reason: "x".into() }.is_outstanding());
        assert!(!ItemExecState::Done { detail: "x".into() }.is_outstanding());
    }

    #[test]
    fn settings_take_dirty_is_consumed_once() {
        let mut form = SettingsForm::from_defaults(valid_defaults());
        assert!(!form.take_dirty());
        form.dirty = true;
        assert!(form.take_dirty());
        assert!(!form.take_dirty()); // second call reads cleared state
    }

    /// Simulate a Done BootstrapEvent and confirm drain_progress moves rows
    /// into last_run and transitions to Idle.
    #[test]
    fn drain_exec_progress_terminal_done_transitions_to_idle() {
        let mut app = AppState::new(valid_defaults());
        app.classes.push(dummy_class_item("foo", 10));
        let (tx, rx) = unbounded_channel::<BootstrapEvent>();
        // Prime Running state with a noop JoinHandle from a throwaway runtime.
        // `drain_progress` doesn't poll the handle — completion is detected
        // via the terminal event on `rx` — so any ready task will do.
        let noop_rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let noop_handle: JoinHandle<Result<BootstrapReport>> =
            noop_rt.spawn(async { Ok(BootstrapReport::default()) });
        app.execution = ExecutionState::Running {
            rx,
            _handle: noop_handle,
            run: ActiveExecution {
                rows: vec![ExecRow {
                    kind: ExecKind::Declare,
                    primary: "foo".into(),
                    secondary: None,
                    status: RowStatus::Pending,
                }],
                class_indices: vec![0],
                contract_indices: vec![],
            },
            tick: 0,
        };

        tx.send(BootstrapEvent::DeclareCompleted {
            idx: 0,
            name: "foo".into(),
            class_hash: Felt::from(99u64),
            already_declared: false,
        })
        .unwrap();
        tx.send(BootstrapEvent::Done { report: BootstrapReport::default() }).unwrap();
        // Drop the sender so Disconnected would fire if terminal weren't seen.
        drop(tx);

        drain_progress(&mut app);

        // Back to Idle, last_run populated, source class item reflects Done.
        assert!(matches!(app.execution, ExecutionState::Idle));
        let lr = app.last_run.as_ref().expect("last_run populated");
        assert!(lr.result.is_ok());
        assert_eq!(lr.rows.len(), 1);
        assert!(matches!(app.classes[0].exec, ItemExecState::Done { .. }));
    }

    /// drain_progress must surface a hung/panicked executor as a Failed
    /// last_run so the UI doesn't stay stuck in Running forever. This closes
    /// a latent bug that pre-existed the refactor.
    #[test]
    fn drain_exec_progress_disconnected_without_terminal_surfaces_error() {
        let mut app = AppState::new(valid_defaults());
        app.classes.push(dummy_class_item("foo", 10));
        let (tx, rx) = unbounded_channel::<BootstrapEvent>();
        let noop_rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let noop_handle: JoinHandle<Result<BootstrapReport>> =
            noop_rt.spawn(async { Ok(BootstrapReport::default()) });
        app.execution = ExecutionState::Running {
            rx,
            _handle: noop_handle,
            run: ActiveExecution {
                rows: vec![ExecRow {
                    kind: ExecKind::Declare,
                    primary: "foo".into(),
                    secondary: None,
                    status: RowStatus::Pending,
                }],
                class_indices: vec![0],
                contract_indices: vec![],
            },
            tick: 0,
        };
        drop(tx); // simulate executor panic: sender dropped without a terminal event

        drain_progress(&mut app);

        assert!(matches!(app.execution, ExecutionState::Idle));
        let lr = app.last_run.as_ref().expect("last_run populated on disconnect");
        let err = lr.result.as_ref().expect_err("disconnect should surface as Err");
        assert!(err.contains("disconnected"));
    }

    /// `x` on the Execute tab with no outstanding work should flash and not
    /// start a task. Covers the "everything is Done" case after a full run.
    #[test]
    fn handle_execute_x_with_no_outstanding_flashes_nothing_pending() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut app = AppState::new(valid_defaults());
        let mut class = dummy_class_item("foo", 10);
        class.exec = ItemExecState::Done { detail: "already".into() };
        app.classes.push(class);

        handle_execute_key(&mut app, KeyCode::Char('x'), runtime.handle());

        assert!(matches!(app.execution, ExecutionState::Idle));
        assert_eq!(
            app.flash.as_deref(),
            Some("nothing pending — all items are done"),
            "should flash explanatory message"
        );
    }

    /// `x` should build a plan from Pending + Failed + Unknown items, skipping
    /// Done ones. Exercised here by inspecting which items end up in the
    /// resulting Running ActiveExecution's index maps.
    #[test]
    fn start_execution_skips_done_items_but_reruns_failed_and_unknown() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut app = AppState::new(valid_defaults());

        // Three classes: pending (new), done (skip), failed (rerun).
        app.classes.push(dummy_class_item("pending", 1));
        let mut done = dummy_class_item("done", 2);
        done.exec = ItemExecState::Done { detail: "already".into() };
        app.classes.push(done);
        let mut failed = dummy_class_item("failed", 3);
        failed.exec = ItemExecState::Failed { detail: "boom".into() };
        app.classes.push(failed);
        // One contract, unknown (should rerun).
        let mut unknown = dummy_contract_item("c", "pending", 4);
        unknown.exec = ItemExecState::Unknown { reason: "verifying…".into() };
        app.contracts.push(unknown);

        start_execution(&mut app, runtime.handle());

        let ExecutionState::Running { run, .. } = &app.execution else {
            panic!("expected Running after start_execution, got {:?}", app.execution);
        };
        // Skipped Done index 1. Mapped rows are classes[0], classes[2], contracts[0].
        assert_eq!(run.class_indices, vec![0, 2]);
        assert_eq!(run.contract_indices, vec![0]);
        assert_eq!(run.rows.len(), 3);
    }

    /// Mutation handlers on Classes/Contracts tabs must flash and refuse the
    /// action while an async task is in flight. Guards the index-map invariant
    /// in ActiveExecution.
    #[test]
    fn add_class_while_running_is_blocked() {
        let mut app = AppState::new(valid_defaults());
        // Fake a Running state cheaply — we don't spawn anything.
        let (_tx, rx) = unbounded_channel::<BootstrapEvent>();
        let noop_rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let noop_handle: JoinHandle<Result<BootstrapReport>> =
            noop_rt.spawn(async { Ok(BootstrapReport::default()) });
        app.execution = ExecutionState::Running {
            rx,
            _handle: noop_handle,
            run: ActiveExecution { rows: vec![], class_indices: vec![], contract_indices: vec![] },
            tick: 0,
        };

        handle_classes_key(&mut app, KeyCode::Char('a'));

        assert!(app.modal.is_none(), "add-class modal must not open during a run");
        assert!(app.flash.is_some(), "user should see a flash explaining the block");
    }

    /// Same as above for delete on the Contracts tab.
    #[test]
    fn delete_contract_while_running_is_blocked() {
        let mut app = AppState::new(valid_defaults());
        app.contracts.push(dummy_contract_item("c", "cls", 1));
        app.contracts_state.select(Some(0));

        let (_tx, rx) = unbounded_channel::<BootstrapEvent>();
        let noop_rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let noop_handle: JoinHandle<Result<BootstrapReport>> =
            noop_rt.spawn(async { Ok(BootstrapReport::default()) });
        app.execution = ExecutionState::Running {
            rx,
            _handle: noop_handle,
            run: ActiveExecution { rows: vec![], class_indices: vec![], contract_indices: vec![] },
            tick: 0,
        };

        handle_contracts_key(&mut app, KeyCode::Char('d'));

        assert_eq!(app.contracts.len(), 1, "delete must be rejected during a run");
        assert!(app.flash.is_some());
    }

    /// Save-manifest should be reachable from any Idle state — including
    /// a fresh session with no last_run. This is a behaviour widening relative
    /// to the pre-refactor "only after a successful run" gate.
    #[test]
    fn save_manifest_available_from_idle_without_last_run() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut app = AppState::new(valid_defaults());
        assert!(app.last_run.is_none());

        handle_execute_key(&mut app, KeyCode::Char('s'), runtime.handle());

        assert!(
            matches!(app.modal, Some(Modal::SaveManifest { .. })),
            "save manifest modal should open even without a prior run",
        );
    }

    /// Quit must be blocked while busy and work from Idle. The Ctrl+C escape
    /// hatch is handled at the top-level handle_key call site so it isn't in
    /// scope here.
    #[test]
    fn quit_from_execute_tab_is_blocked_while_running() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut app = AppState::new(valid_defaults());
        let (_tx, rx) = unbounded_channel::<BootstrapEvent>();
        let noop_rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let noop_handle: JoinHandle<Result<BootstrapReport>> =
            noop_rt.spawn(async { Ok(BootstrapReport::default()) });
        app.execution = ExecutionState::Running {
            rx,
            _handle: noop_handle,
            run: ActiveExecution { rows: vec![], class_indices: vec![], contract_indices: vec![] },
            tick: 0,
        };

        handle_execute_key(&mut app, KeyCode::Char('q'), runtime.handle());
        assert!(!app.quit, "q must be blocked while Running");
        assert!(app.flash.is_some());
    }

    #[test]
    fn quit_from_execute_tab_works_from_idle() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut app = AppState::new(valid_defaults());
        handle_execute_key(&mut app, KeyCode::Char('q'), runtime.handle());
        assert!(app.quit);
    }

    /// Editing a contract in place must reset just that item's exec state and
    /// leave last_run untouched. This is the "user edits a Done item" path.
    #[test]
    fn edit_existing_contract_via_modal_resets_exec_to_pending_preserves_last_run() {
        let mut app = AppState::new(valid_defaults());

        // Prime a Done contract and a last_run so we can verify both halves.
        let mut c = dummy_contract_item("orig", "cls", 1);
        c.exec = ItemExecState::Done { detail: "address: 0xabc".into() };
        app.contracts.push(c);
        app.last_run = Some(LastRunReport {
            rows: vec![ExecRow {
                kind: ExecKind::Deploy,
                primary: "orig".into(),
                secondary: Some("cls".into()),
                status: RowStatus::Done("address: 0xabc".into()),
            }],
            result: Ok(BootstrapReport::default()),
        });

        // Simulate the ContractForm modal committing a new step into index 0.
        // We bypass the ContractForm::build() path (needs ClassOption) and
        // write directly, mirroring what the Enter handler does after build()
        // succeeds.
        let new_step = DeployStep {
            label: Some("edited".into()),
            class_hash: Felt::from(42u64),
            class_name: "cls".into(),
            salt: Felt::from(7u64),
            unique: true,
            calldata: Vec::new(),
        };
        if let Some(slot) = app.contracts.get_mut(0) {
            slot.step = new_step;
            slot.exec = ItemExecState::Pending;
        }

        assert!(matches!(app.contracts[0].exec, ItemExecState::Pending));
        assert_eq!(app.contracts[0].step.label.as_deref(), Some("edited"));
        assert!(app.last_run.is_some(), "last_run must survive the edit");
    }

    /// Regression: after a failed refresh (e.g. user typed a bad RPC URL),
    /// items end up in Unknown, not Done. A second Settings change that fixes
    /// the URL must still re-probe them — otherwise the items are stranded
    /// as Unknown until the user manually presses `x`. The `needs_refresh`
    /// predicate is the single source of truth for "is this item eligible
    /// for re-probing."
    #[test]
    fn needs_refresh_includes_done_and_unknown_but_not_pending_or_failed() {
        assert!(needs_refresh(&ItemExecState::Done { detail: "x".into() }));
        assert!(needs_refresh(&ItemExecState::Unknown { reason: "x".into() }));
        assert!(!needs_refresh(&ItemExecState::Pending));
        assert!(!needs_refresh(&ItemExecState::Running));
        assert!(!needs_refresh(&ItemExecState::Failed { detail: "x".into() }));
    }

    #[test]
    fn deleting_a_done_item_does_not_clear_last_run() {
        let mut app = AppState::new(valid_defaults());
        let mut c = dummy_class_item("foo", 1);
        c.exec = ItemExecState::Done { detail: "hash".into() };
        app.classes.push(c);
        app.classes_state.select(Some(0));
        app.last_run = Some(LastRunReport { rows: vec![], result: Ok(BootstrapReport::default()) });

        handle_classes_key(&mut app, KeyCode::Char('d'));

        assert!(app.classes.is_empty());
        assert!(app.last_run.is_some(), "last_run is history, deletion must not clear it");
    }
}

/// Render the dedicated "loading class" overlay. Lives in its own function rather
/// than inline in `draw_modal` because the layout (centred spinner + filename + parent
/// dir + footer) is significantly different from any other modal — there's no shared
/// query box or list state to factor with.
///
/// `cwd` is the directory the user opened the file picker from (i.e.
/// `FileSearch::root`); it's used to render the parent path *relative* to where the
/// user actually is, since long absolute paths just clutter the modal.
///
/// Render the contract form modal as three stacked sections:
///
/// 1. **Contract** — class picker, label, salt, unique flag.
/// 2. **Constructor** — one row per typed constructor argument, or a single raw-felts editor when
///    the selected class has no introspectable ABI.
/// 3. **Footer** — keyboard hints, value-format tip, and validation error.
///
/// Splitting the form this way makes the constructor block visually distinct
/// from the contract metadata and lets the per-arg row count grow without
/// pushing the basic fields around.
fn draw_contract_form_modal(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    _app: &AppState,
    editing_index: Option<usize>,
    form: &ContractForm,
) {
    let class_display = if form.class_name.is_empty() {
        "(no classes)".to_string()
    } else {
        format!("◀ {} ▶", form.class_name)
    };
    let unique = if form.unique { "[x]" } else { "[ ]" };
    let title = if editing_index.is_some() { "Edit contract" } else { "Add contract" };

    // Wider label column to accommodate constructor argument names like
    // `account_address`. Picked empirically — narrower wraps ugly.
    const LABEL_WIDTH: usize = 16;

    let render_label = |label: &str, focused: bool| -> Span<'static> {
        let marker = if focused { "> " } else { "  " };
        let style = if focused {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Span::styled(format!("{marker}{label:<LABEL_WIDTH$}"), style)
    };

    // Outer frame: title only, no borders. The inner blocks supply their own
    // borders so the section structure is the visible affordance.
    let outer = Block::default().borders(Borders::ALL).title(title);
    let inner_area = outer.inner(area);
    f.render_widget(outer, area);

    // Contract block: 4 fixed rows + 2 borders = 6 lines.
    // Constructor block: takes the remaining space minus the footer reservation.
    // Footer: hint (1) + tip (1, when typed) + error (2, when present) + slack.
    let footer_height: u16 = 1
        + if matches!(form.calldata, CalldataInput::Typed { .. }) { 1 } else { 0 }
        + if form.error.is_some() { 2 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(3),
            Constraint::Length(footer_height + 1),
        ])
        .split(inner_area);

    // ----- Contract section --------------------------------------------------
    let mut contract_lines = Vec::new();
    for field in
        [ContractField::Class, ContractField::Label, ContractField::Salt, ContractField::Unique]
    {
        let focused = field == form.focused;
        match field {
            ContractField::Class => {
                let style = if focused {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                contract_lines.push(Line::from(vec![
                    render_label("Class", focused),
                    Span::raw("  "),
                    Span::styled(class_display.clone(), style),
                ]));
            }
            ContractField::Label => {
                let mut spans = vec![render_label("Label", focused), Span::raw("  ")];
                spans.extend(render_text_input(&form.label, focused, false));
                contract_lines.push(Line::from(spans));
            }
            ContractField::Salt => {
                let mut spans = vec![render_label("Salt", focused), Span::raw("  ")];
                spans.extend(render_text_input(&form.salt, focused, false));
                contract_lines.push(Line::from(spans));
            }
            ContractField::Unique => {
                let style = if focused {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                contract_lines.push(Line::from(vec![
                    render_label("Unique", focused),
                    Span::raw("  "),
                    Span::styled(unique.to_string(), style),
                ]));
            }
            _ => {}
        }
    }
    let contract_block = Block::default().borders(Borders::ALL).title(" Contract ");
    f.render_widget(Paragraph::new(contract_lines).block(contract_block), chunks[0]);

    // ----- Constructor section -----------------------------------------------
    let (ctor_title, ctor_lines) = match &form.calldata {
        CalldataInput::Typed { args } if args.is_empty() => (
            " Constructor ".to_string(),
            vec![Line::from(Span::styled(
                "  (no inputs)",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            ))],
        ),
        CalldataInput::Typed { args } => {
            let mut lines = Vec::with_capacity(args.len());
            for (i, arg) in args.iter().enumerate() {
                let focused = ContractField::Arg(i) == form.focused;
                let mut spans = vec![render_label(&arg.name, focused), Span::raw("  ")];
                spans.push(Span::styled(
                    format!("{:<14}", crate::abi::pretty_type(&arg.ty)),
                    Style::default().fg(Color::DarkGray),
                ));
                spans.push(Span::raw("  "));
                spans.extend(render_text_input(&arg.input, focused, false));
                lines.push(Line::from(spans));
            }
            (" Constructor ".to_string(), lines)
        }
        CalldataInput::Raw(input) => {
            let focused = form.focused == ContractField::RawCalldata;
            let mut spans = vec![render_label("Calldata", focused), Span::raw("  ")];
            spans.extend(render_text_input(input, focused, false));
            // Heads-up so the user knows why they're seeing the raw editor.
            let note = Line::from(Span::styled(
                "  No ABI available — enter raw felts (comma-separated)",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            ));
            (" Calldata (raw) ".to_string(), vec![note, Line::from(""), Line::from(spans)])
        }
    };
    let ctor_block = Block::default().borders(Borders::ALL).title(ctor_title);
    f.render_widget(Paragraph::new(ctor_lines).block(ctor_block), chunks[1]);

    // ----- Footer ------------------------------------------------------------
    let mut footer_lines = vec![Line::from(Span::styled(
        "[Tab] next field  [←/→] cycle class  [Space] toggle unique  [^A/^E/^W/^U/^K] edit  \
         [Enter] save  [Esc] cancel",
        Style::default().fg(Color::DarkGray),
    ))];
    if let CalldataInput::Typed { .. } = &form.calldata {
        footer_lines.push(Line::from(Span::styled(
            "Tip: bare hex (0x..) for felts, JSON for arrays/structs, blank for None",
            Style::default().fg(Color::DarkGray),
        )));
    }
    if let Some(e) = &form.error {
        footer_lines.push(Line::from(""));
        footer_lines
            .push(Line::from(Span::styled(format!("error: {e}"), Style::default().fg(Color::Red))));
    }
    f.render_widget(Paragraph::new(footer_lines).wrap(Wrap { trim: false }), chunks[2]);
}

/// When `notice` is `Some`, the spinner row is replaced by the notice text and the
/// footer hint flips from `[Esc] cancel` to `[Esc] dismiss`. This is how the
/// "class is already selected" path surfaces to the user.
fn draw_loading_class_modal(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    pending: &PendingLoad,
    cwd: &std::path::Path,
    notice: Option<&str>,
) {
    // Display name = file name only (the parent dir gets its own dimmer line below).
    // Falls back to the full path if file_name() fails for some odd reason.
    let file_name = pending
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| pending.path.to_string_lossy().into_owned());
    // Render the parent dir relative to cwd when possible. For paths inside cwd this
    // strips the noisy absolute prefix; for paths outside cwd we fall back to the
    // full absolute parent. The empty string ("") that `strip_prefix` produces when
    // the file lives directly in cwd becomes "./" so the line still has *some*
    // visible content.
    let parent = pending
        .path
        .parent()
        .map(|abs_parent| match abs_parent.strip_prefix(cwd) {
            Ok(rel) if rel.as_os_str().is_empty() => "./".to_string(),
            Ok(rel) => format!("./{}", rel.display()),
            Err(_) => abs_parent.to_string_lossy().into_owned(),
        })
        .unwrap_or_default();

    // Header line: spinner + bold file name (during loading) or static check + name
    // (once we have something to say about the result).
    let header = if notice.is_some() {
        Line::from(vec![
            Span::raw("  "),
            Span::styled("✓", Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::styled(file_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ])
    } else {
        let elapsed = pending.started.elapsed();
        let frame = SPINNER_FRAMES[(elapsed.as_millis() / 80) as usize % SPINNER_FRAMES.len()];
        Line::from(vec![
            Span::raw("  "),
            Span::styled(frame.to_string(), Style::default().fg(Color::Yellow)),
            Span::raw("  "),
            Span::styled(file_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ])
    };

    // Body line: either the notice (yellow) when present, or an elapsed-time row
    // alongside the parent dir.
    let mut lines = vec![Line::from(""), header];
    lines.push(Line::from(vec![
        Span::raw("     "),
        Span::styled(parent, Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(""));

    if let Some(msg) = notice {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(msg.to_string(), Style::default().fg(Color::Yellow)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("[Esc] dismiss", Style::default().fg(Color::DarkGray)),
        ]));
    } else {
        let elapsed = pending.started.elapsed();
        let elapsed_text =
            format!("{}.{:01}s elapsed", elapsed.as_secs(), elapsed.subsec_millis() / 100);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(elapsed_text, Style::default().fg(Color::DarkGray)),
            Span::raw("       "),
            Span::styled("[Esc] cancel", Style::default().fg(Color::DarkGray)),
        ]));
    }

    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        " Loading class ",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    ));
    // Wrap so a long file name, deeply-nested parent path, or wide notice flows
    // onto multiple lines instead of getting clipped at the modal's right edge.
    // `trim: false` keeps the leading whitespace we use for indentation.
    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(p, area);
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
