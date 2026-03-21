use std::io;

use anyhow::Result;
use clap::Args;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use katana_db::abstraction::{Database, DbCursor, DbTx};
use katana_db::tables::{self, Tables};

/// Tables to display in the inspector (excludes state trie tables).
const INSPECT_TABLES: &[Tables] = &[
    Tables::Headers,
    Tables::BlockStateUpdates,
    Tables::BlockHashes,
    Tables::BlockNumbers,
    Tables::BlockBodyIndices,
    Tables::BlockStatusses,
    Tables::TxNumbers,
    Tables::TxBlocks,
    Tables::TxHashes,
    Tables::TxTraces,
    Tables::Transactions,
    Tables::Receipts,
    Tables::CompiledClassHashes,
    Tables::Classes,
    Tables::ContractInfo,
    Tables::ContractStorage,
    Tables::ClassDeclarationBlock,
    Tables::ClassDeclarations,
    Tables::MigratedCompiledClassHashes,
    Tables::ContractInfoChangeSet,
    Tables::NonceChangeHistory,
    Tables::ClassChangeHistory,
    Tables::StorageChangeHistory,
    Tables::StorageChangeSet,
    Tables::StageExecutionCheckpoints,
    Tables::StagePruningCheckpoints,
    Tables::StateHistoryRetention,
    Tables::MigrationCheckpoints,
];
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;

use crate::cli::db::open_db_ro;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct InspectArgs {
    /// Path to the database directory.
    #[arg(short, long)]
    #[arg(default_value = "~/.katana/db")]
    pub path: String,
}

/// Dispatch only `tx.entries::<T>()` without creating a cursor.
/// Returns `None` if the table doesn't exist in the database (e.g. older DB version).
macro_rules! try_count_entries {
    ($tables_variant:expr, $tx:expr) => {
        match $tables_variant {
            Tables::Headers => $tx.entries::<tables::Headers>().ok(),
            Tables::BlockStateUpdates => $tx.entries::<tables::BlockStateUpdates>().ok(),
            Tables::BlockHashes => $tx.entries::<tables::BlockHashes>().ok(),
            Tables::BlockNumbers => $tx.entries::<tables::BlockNumbers>().ok(),
            Tables::BlockBodyIndices => $tx.entries::<tables::BlockBodyIndices>().ok(),
            Tables::BlockStatusses => $tx.entries::<tables::BlockStatusses>().ok(),
            Tables::TxNumbers => $tx.entries::<tables::TxNumbers>().ok(),
            Tables::TxBlocks => $tx.entries::<tables::TxBlocks>().ok(),
            Tables::TxHashes => $tx.entries::<tables::TxHashes>().ok(),
            Tables::TxTraces => $tx.entries::<tables::TxTraces>().ok(),
            Tables::Transactions => $tx.entries::<tables::Transactions>().ok(),
            Tables::Receipts => $tx.entries::<tables::Receipts>().ok(),
            Tables::CompiledClassHashes => $tx.entries::<tables::CompiledClassHashes>().ok(),
            Tables::Classes => $tx.entries::<tables::Classes>().ok(),
            Tables::ContractInfo => $tx.entries::<tables::ContractInfo>().ok(),
            Tables::ContractStorage => $tx.entries::<tables::ContractStorage>().ok(),
            Tables::ClassDeclarationBlock => $tx.entries::<tables::ClassDeclarationBlock>().ok(),
            Tables::ClassDeclarations => $tx.entries::<tables::ClassDeclarations>().ok(),
            Tables::MigratedCompiledClassHashes => {
                $tx.entries::<tables::MigratedCompiledClassHashes>().ok()
            }
            Tables::ContractInfoChangeSet => $tx.entries::<tables::ContractInfoChangeSet>().ok(),
            Tables::NonceChangeHistory => $tx.entries::<tables::NonceChangeHistory>().ok(),
            Tables::ClassChangeHistory => $tx.entries::<tables::ClassChangeHistory>().ok(),
            Tables::StorageChangeHistory => $tx.entries::<tables::StorageChangeHistory>().ok(),
            Tables::StorageChangeSet => $tx.entries::<tables::StorageChangeSet>().ok(),
            Tables::StageExecutionCheckpoints => {
                $tx.entries::<tables::StageExecutionCheckpoints>().ok()
            }
            Tables::StagePruningCheckpoints => {
                $tx.entries::<tables::StagePruningCheckpoints>().ok()
            }
            Tables::StateHistoryRetention => $tx.entries::<tables::StateHistoryRetention>().ok(),
            Tables::MigrationCheckpoints => $tx.entries::<tables::MigrationCheckpoints>().ok(),
            // State trie tables are excluded from the inspector
            _ => None,
        }
    };
}

/// Fetch a page of entries from a table as formatted string pairs.
/// Keys use Display format where available, Debug otherwise. Values always use Debug.
/// Returns an empty vec if the table doesn't exist in the database.
fn fetch_entries<Tx: DbTx>(
    tx: &Tx,
    table: Tables,
    offset: usize,
    limit: usize,
) -> Vec<(String, String)> {
    /// Walk a table cursor, formatting keys with Display.
    fn walk_display<Tx: DbTx, T: tables::Table>(
        tx: &Tx,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(String, String)>, katana_db::error::DatabaseError>
    where
        T::Key: std::fmt::Display,
    {
        let mut cursor = tx.cursor::<T>()?;
        let mut entries = Vec::with_capacity(limit);
        let mut walker = cursor.walk(None)?;
        for _ in 0..offset {
            if walker.next().is_none() {
                return Ok(entries);
            }
        }
        for item in walker.take(limit) {
            let Ok((key, value)) = item else { break };
            entries.push((format!("{key}"), format!("{value:#?}")));
        }
        Ok(entries)
    }

    /// Walk a table cursor, formatting keys with Debug (fallback for types without Display).
    fn walk_debug<Tx: DbTx, T: tables::Table>(
        tx: &Tx,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(String, String)>, katana_db::error::DatabaseError> {
        let mut cursor = tx.cursor::<T>()?;
        let mut entries = Vec::with_capacity(limit);
        let mut walker = cursor.walk(None)?;
        for _ in 0..offset {
            if walker.next().is_none() {
                return Ok(entries);
            }
        }
        for item in walker.take(limit) {
            let Ok((key, value)) = item else { break };
            entries.push((format!("{key:?}"), format!("{value:#?}")));
        }
        Ok(entries)
    }

    macro_rules! fetch {
        ($t:ty) => {
            walk_display::<Tx, $t>(tx, offset, limit).unwrap_or_default()
        };
        (debug $t:ty) => {
            walk_debug::<Tx, $t>(tx, offset, limit).unwrap_or_default()
        };
    }

    match table {
        Tables::Headers => fetch!(tables::Headers),
        Tables::BlockStateUpdates => fetch!(tables::BlockStateUpdates),
        Tables::BlockHashes => fetch!(tables::BlockHashes),
        Tables::BlockNumbers => fetch!(tables::BlockNumbers),
        Tables::BlockBodyIndices => fetch!(tables::BlockBodyIndices),
        Tables::BlockStatusses => fetch!(tables::BlockStatusses),
        Tables::TxNumbers => fetch!(tables::TxNumbers),
        Tables::TxBlocks => fetch!(tables::TxBlocks),
        Tables::TxHashes => fetch!(tables::TxHashes),
        Tables::TxTraces => fetch!(tables::TxTraces),
        Tables::Transactions => fetch!(tables::Transactions),
        Tables::Receipts => fetch!(tables::Receipts),
        Tables::CompiledClassHashes => fetch!(tables::CompiledClassHashes),
        Tables::Classes => fetch!(tables::Classes),
        Tables::ContractInfo => fetch!(tables::ContractInfo),
        Tables::ContractStorage => fetch!(debug tables::ContractStorage),
        Tables::ClassDeclarationBlock => fetch!(tables::ClassDeclarationBlock),
        Tables::ClassDeclarations => fetch!(tables::ClassDeclarations),
        Tables::MigratedCompiledClassHashes => fetch!(tables::MigratedCompiledClassHashes),
        Tables::ContractInfoChangeSet => fetch!(tables::ContractInfoChangeSet),
        Tables::NonceChangeHistory => fetch!(tables::NonceChangeHistory),
        Tables::ClassChangeHistory => fetch!(tables::ClassChangeHistory),
        Tables::StorageChangeHistory => fetch!(debug tables::StorageChangeHistory),
        Tables::StorageChangeSet => fetch!(debug tables::StorageChangeSet),
        Tables::StageExecutionCheckpoints => fetch!(tables::StageExecutionCheckpoints),
        Tables::StagePruningCheckpoints => fetch!(tables::StagePruningCheckpoints),
        Tables::StateHistoryRetention => fetch!(tables::StateHistoryRetention),
        Tables::MigrationCheckpoints => fetch!(tables::MigrationCheckpoints),
        // State trie tables are excluded from the inspector
        _ => Vec::new(),
    }
}

// -- Application state --

enum Screen {
    TableList,
    EntryView,
}

struct App {
    screen: Screen,
    /// Table list state
    table_list: ListState,
    /// Entry counts per table (indexed same as INSPECT_TABLES).
    /// `None` means the table doesn't exist in the database.
    table_counts: Vec<Option<usize>>,
    /// Currently loaded entries for the open table (key, value) Debug strings
    entries: Vec<(String, String)>,
    /// Total entry count for the currently open table
    current_table_count: usize,
    /// Current offset into the table for pagination
    entry_offset: usize,
    /// Entry list selection state
    entry_list: ListState,
    /// Scroll offset for the value panel
    value_scroll: u16,
    /// Should quit
    quit: bool,
}

const PAGE_SIZE: usize = 500;

impl App {
    fn new(table_counts: Vec<Option<usize>>) -> Self {
        let mut table_list = ListState::default();
        table_list.select(Some(0));
        Self {
            screen: Screen::TableList,
            table_list,
            table_counts,
            entries: Vec::new(),
            current_table_count: 0,
            entry_offset: 0,
            entry_list: ListState::default(),
            value_scroll: 0,
            quit: false,
        }
    }

    fn selected_table_index(&self) -> usize {
        self.table_list.selected().unwrap_or(0)
    }

    fn selected_entry_index(&self) -> usize {
        self.entry_list.selected().unwrap_or(0)
    }

    /// Open a table: load its first page of entries.
    /// Does nothing if the table doesn't exist in the database.
    fn open_table<Tx: DbTx>(&mut self, tx: &Tx) {
        let idx = self.selected_table_index();
        let Some(count) = self.table_counts[idx] else {
            return; // Table doesn't exist
        };
        let table = INSPECT_TABLES[idx];
        self.current_table_count = count;
        self.entry_offset = 0;
        self.entries = fetch_entries(tx, table, 0, PAGE_SIZE);
        self.entry_list = ListState::default();
        if !self.entries.is_empty() {
            self.entry_list.select(Some(0));
        }
        self.value_scroll = 0;
        self.screen = Screen::EntryView;
    }

    /// Ensure the selected entry is within the loaded page, re-fetching if needed.
    fn ensure_entry_loaded<Tx: DbTx>(&mut self, tx: &Tx, absolute_index: usize) {
        let page_end = self.entry_offset + self.entries.len();
        if absolute_index >= page_end || absolute_index < self.entry_offset {
            // Re-fetch a page centered around the target
            let new_offset = absolute_index.saturating_sub(PAGE_SIZE / 4);
            let idx = self.selected_table_index();
            let table = INSPECT_TABLES[idx];
            self.entries = fetch_entries(tx, table, new_offset, PAGE_SIZE);
            self.entry_offset = new_offset;
        }
    }

    fn move_entry_selection<Tx: DbTx>(&mut self, tx: &Tx, delta: isize) {
        if self.current_table_count == 0 {
            return;
        }
        let current = self.entry_offset + self.selected_entry_index();
        let new_abs = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            (current + delta as usize).min(self.current_table_count - 1)
        };
        self.ensure_entry_loaded(tx, new_abs);
        self.entry_list.select(Some(new_abs - self.entry_offset));
        self.value_scroll = 0;
    }

    fn jump_entry_first<Tx: DbTx>(&mut self, tx: &Tx) {
        if self.current_table_count == 0 {
            return;
        }
        self.ensure_entry_loaded(tx, 0);
        self.entry_list.select(Some(0));
        self.value_scroll = 0;
    }

    fn jump_entry_last<Tx: DbTx>(&mut self, tx: &Tx) {
        if self.current_table_count == 0 {
            return;
        }
        let last = self.current_table_count - 1;
        self.ensure_entry_loaded(tx, last);
        self.entry_list.select(Some(last - self.entry_offset));
        self.value_scroll = 0;
    }
}

impl InspectArgs {
    pub fn execute(self) -> Result<()> {
        let db = open_db_ro(&self.path)?;

        // Warn about version mismatch and let user decide whether to continue
        if db.require_migration() {
            let current = db.version();
            let latest = katana_db::version::LATEST_DB_VERSION;

            eprintln!(
                "WARNING: Database version ({current}) is older than the current version \
                 ({latest}). Some tables may be missing or incompatible."
            );

            let proceed = inquire::Confirm::new("Continue anyway?").with_default(true).prompt()?;

            if !proceed {
                return Ok(());
            }
        }
        let tx = db.tx()?;

        // Collect entry counts for all tables (None if table doesn't exist)
        let mut table_counts = Vec::with_capacity(INSPECT_TABLES.len());
        for &table in INSPECT_TABLES {
            table_counts.push(try_count_entries!(table, tx));
        }

        let mut app = App::new(table_counts);

        // Setup terminal
        enable_raw_mode()?;
        io::stdout().execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        let result = run_event_loop(&mut terminal, &mut app, &tx);

        // Restore terminal
        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;

        result
    }
}

fn run_event_loop<Tx: DbTx>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    tx: &Tx,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if app.quit {
            return Ok(());
        }

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Global quit
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.quit = true;
                continue;
            }

            match app.screen {
                Screen::TableList => match key.code {
                    KeyCode::Char('q') => app.quit = true,
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = app.selected_table_index();
                        if i + 1 < INSPECT_TABLES.len() {
                            app.table_list.select(Some(i + 1));
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = app.selected_table_index();
                        app.table_list.select(Some(i.saturating_sub(1)));
                    }
                    KeyCode::Char('g') => {
                        app.table_list.select(Some(0));
                    }
                    KeyCode::Char('G') => {
                        app.table_list.select(Some(INSPECT_TABLES.len() - 1));
                    }
                    KeyCode::Enter => {
                        app.open_table(tx);
                    }
                    KeyCode::Esc => app.quit = true,
                    _ => {}
                },
                Screen::EntryView => match key.code {
                    KeyCode::Char('q') => app.quit = true,
                    KeyCode::Esc => {
                        app.screen = Screen::TableList;
                        app.entries.clear();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.move_entry_selection(tx, 1);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.move_entry_selection(tx, -1);
                    }
                    KeyCode::Char('g') => {
                        app.jump_entry_first(tx);
                    }
                    KeyCode::Char('G') => {
                        app.jump_entry_last(tx);
                    }
                    KeyCode::Char('h') | KeyCode::Left => {
                        app.value_scroll = app.value_scroll.saturating_sub(4);
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        app.value_scroll = app.value_scroll.saturating_add(4);
                    }
                    _ => {}
                },
            }
        }
    }
}

fn draw(f: &mut ratatui::Frame<'_>, app: &mut App) {
    match app.screen {
        Screen::TableList => draw_table_list(f, app),
        Screen::EntryView => draw_entry_view(f, app),
    }
}

fn draw_table_list(f: &mut ratatui::Frame<'_>, app: &mut App) {
    let area = f.area();

    let items: Vec<ListItem<'_>> = INSPECT_TABLES
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let count_str = match app.table_counts[i] {
                Some(count) => format_number(count),
                None => "-".to_string(),
            };
            let content = format!("{:<40} {:>10}", t.name(), count_str);
            let style = if app.table_counts[i].is_none() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(content)).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" katana db inspect ")
                .title_bottom(Line::from(" q:quit  \u{2191}\u{2193}:nav  Enter:open ").centered()),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, area, &mut app.table_list);
}

fn draw_entry_view(f: &mut ratatui::Frame<'_>, app: &mut App) {
    let area = f.area();
    let idx = app.selected_table_index();
    let table = INSPECT_TABLES[idx];
    let count = app.current_table_count;

    let title = format!(" {} ({} entries) ", table.name(), format_number(count));

    let outer_block = Block::default().borders(Borders::ALL).title(title).title_bottom(
        Line::from(" Esc:back  \u{2191}\u{2193}:nav  h/l:scroll value  g/G:first/last  q:quit ")
            .centered(),
    );

    let inner = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Split vertically: 1-line header + remaining content
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    // Split both header and content into the same column proportions
    let header_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(rows[0]);

    let content_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(rows[1]);

    // Render column headers
    let header_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let num_width = 6u16;
    let key_header_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(num_width), Constraint::Min(1)])
        .split(header_cols[0]);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled("    # ", header_style))),
        key_header_cols[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("Key", header_style))),
        key_header_cols[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("Value", header_style))),
        header_cols[1],
    );

    draw_key_panel(f, app, content_cols[0]);
    draw_value_panel(f, app, content_cols[1]);
}

fn draw_key_panel(f: &mut ratatui::Frame<'_>, app: &mut App, area: Rect) {
    // Split into fixed-width line number column and key list
    let num_width = 6u16; // enough for "99999 "
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(num_width), Constraint::Min(1)])
        .split(area);

    // Line numbers (rendered as plain Paragraph, scrolled in sync with the key list)
    let selected = app.selected_entry_index();
    let num_lines: Vec<Line<'_>> = app
        .entries
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let abs_index = app.entry_offset + i;
            let style = if i == selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(Span::styled(format!("{abs_index:>5} "), style))
        })
        .collect();

    // Offset the paragraph to keep line numbers in sync with the list scroll.
    // ListState handles its own viewport scroll; we replicate the offset here.
    let visible_height = cols[0].height as usize;
    let scroll_offset = selected.saturating_sub(visible_height.saturating_sub(1));
    let num_paragraph = Paragraph::new(num_lines).scroll((scroll_offset as u16, 0));
    f.render_widget(num_paragraph, cols[0]);

    // Key list
    let key_area = cols[1];
    let items: Vec<ListItem<'_>> = app
        .entries
        .iter()
        .map(|(key, _)| {
            let display = truncate_str(key, key_area.width.saturating_sub(4) as usize);
            ListItem::new(Line::from(display))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::RIGHT))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, key_area, &mut app.entry_list);
}

fn draw_value_panel(f: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let selected = app.selected_entry_index();
    let value_text = app.entries.get(selected).map(|(_, v)| v.as_str()).unwrap_or("");

    // Apply horizontal scroll by trimming each line
    let lines: Vec<Line<'_>> = value_text
        .lines()
        .map(|line| {
            let scroll = app.value_scroll as usize;
            let visible = if scroll < line.len() { &line[scroll..] } else { "" };
            Line::from(Span::raw(visible.to_string()))
        })
        .collect();

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

/// Truncate a string to fit within `max_len` characters, adding "..." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}

/// Format a number with comma separators.
fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
