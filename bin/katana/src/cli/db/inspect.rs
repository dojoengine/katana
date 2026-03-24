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

// -- Data fetching --

/// Result of fetching entries from a table.
struct FetchResult {
    /// Column headers (key column(s) + value column(s)).
    columns: Vec<&'static str>,
    /// Each row is a vec of cell strings, matching `columns`.
    rows: Vec<Vec<String>>,
    /// Whether to display in tabular mode or detail (split-panel) mode.
    tabular: bool,
    /// The value type name (shown above the field table in tabular mode).
    value_type: &'static str,
}

/// Fetch a page of entries from a table with per-table formatting.
fn fetch_table_data<Tx: DbTx>(tx: &Tx, table: Tables, offset: usize, limit: usize) -> FetchResult {
    /// Walk a cursor and format each entry using the provided closure.
    fn walk<Tx: DbTx, T: tables::Table, F: Fn(&T::Key, &T::Value) -> Vec<String>>(
        tx: &Tx,
        offset: usize,
        limit: usize,
        fmt: F,
    ) -> Vec<Vec<String>> {
        let result: Result<_, katana_db::error::DatabaseError> = (|| {
            let mut cursor = tx.cursor::<T>()?;
            let mut rows = Vec::with_capacity(limit);
            let mut walker = cursor.walk(None)?;
            for _ in 0..offset {
                if walker.next().is_none() {
                    return Ok(rows);
                }
            }
            for item in walker.take(limit) {
                let Ok((key, value)) = item else { break };
                rows.push(fmt(&key, &value));
            }
            Ok(rows)
        })();
        result.unwrap_or_default()
    }

    /// Helper to build a tabular FetchResult.
    macro_rules! tabular {
        ($t:ty, $vtype:expr, [$($col:expr),+], |$k:ident, $v:ident| $body:expr) => {{
            let rows = walk::<Tx, $t, _>(tx, offset, limit, |$k, $v| $body);
            FetchResult {
                columns: vec![$($col),+],
                rows,
                tabular: true,
                value_type: $vtype,
            }
        }};
    }

    /// Helper to build a detail (split-panel) FetchResult.
    macro_rules! detail {
        ($t:ty) => {{
            let rows = walk::<Tx, $t, _>(tx, offset, limit, |k, v| {
                vec![format!("{k}"), format!("{v:#?}")]
            });
            FetchResult { columns: vec!["Key", "Value"], rows, tabular: false, value_type: "" }
        }};
        (debug $t:ty) => {{
            let rows = walk::<Tx, $t, _>(tx, offset, limit, |k, v| {
                vec![format!("{k:?}"), format!("{v:#?}")]
            });
            FetchResult { columns: vec!["Key", "Value"], rows, tabular: false, value_type: "" }
        }};
    }

    match table {
        // -- Simple key-value (scalar value) --
        Tables::BlockHashes => {
            tabular!(tables::BlockHashes, "BlockHash", ["number", "value"], |k, v| vec![
                format!("{k}"),
                format!("{v}")
            ])
        }
        Tables::BlockNumbers => {
            tabular!(tables::BlockNumbers, "BlockNumber", ["hash", "value"], |k, v| vec![
                format!("{k}"),
                format!("{v}")
            ])
        }
        Tables::BlockStatusses => {
            tabular!(tables::BlockStatusses, "FinalityStatus", ["number", "value"], |k, v| vec![
                format!("{k}"),
                format!("{v:?}")
            ])
        }
        Tables::TxNumbers => {
            tabular!(tables::TxNumbers, "TxNumber", ["tx_hash", "value"], |k, v| vec![
                format!("{k}"),
                format!("{v}")
            ])
        }
        Tables::TxBlocks => {
            tabular!(tables::TxBlocks, "BlockNumber", ["tx_number", "value"], |k, v| vec![
                format!("{k}"),
                format!("{v}")
            ])
        }
        Tables::TxHashes => {
            tabular!(tables::TxHashes, "TxHash", ["tx_number", "value"], |k, v| vec![
                format!("{k}"),
                format!("{v}")
            ])
        }
        Tables::CompiledClassHashes => tabular!(
            tables::CompiledClassHashes,
            "CompiledClassHash",
            ["class_hash", "value"],
            |k, v| vec![format!("{k}"), format!("{v}")]
        ),
        Tables::ClassDeclarationBlock => tabular!(
            tables::ClassDeclarationBlock,
            "BlockNumber",
            ["class_hash", "value"],
            |k, v| vec![format!("{k}"), format!("{v}")]
        ),
        Tables::ClassDeclarations => {
            tabular!(tables::ClassDeclarations, "ClassHash", ["number", "value"], |k, v| vec![
                format!("{k}"),
                format!("{v}")
            ])
        }

        // -- Struct values (few simple fields) --
        Tables::BlockBodyIndices => tabular!(
            tables::BlockBodyIndices,
            "StoredBlockBodyIndices",
            ["number", "tx_offset", "tx_count"],
            |k, v| vec![format!("{k}"), format!("{}", v.tx_offset), format!("{}", v.tx_count)]
        ),
        Tables::ContractInfo => tabular!(
            tables::ContractInfo,
            "GenericContractInfo",
            ["contract_address", "nonce", "class_hash"],
            |k, v| vec![format!("{k}"), format!("{}", v.nonce), format!("{}", v.class_hash)]
        ),
        Tables::ContractStorage => tabular!(
            tables::ContractStorage,
            "StorageEntry",
            ["contract_address", "key", "value"],
            |k, v| vec![format!("{k}"), format!("{}", v.key), format!("{}", v.value)]
        ),
        Tables::MigratedCompiledClassHashes => tabular!(
            tables::MigratedCompiledClassHashes,
            "MigratedCompiledClassHash",
            ["number", "class_hash", "compiled_class_hash"],
            |k, v| vec![
                format!("{k}"),
                format!("{}", v.class_hash),
                format!("{}", v.compiled_class_hash)
            ]
        ),
        Tables::NonceChangeHistory => tabular!(
            tables::NonceChangeHistory,
            "ContractNonceChange",
            ["number", "contract_address", "nonce"],
            |k, v| vec![format!("{k}"), format!("{}", v.contract_address), format!("{}", v.nonce)]
        ),
        Tables::ClassChangeHistory => tabular!(
            tables::ClassChangeHistory,
            "ContractClassChange",
            ["number", "type", "contract_address", "class_hash"],
            |k, v| vec![
                format!("{k}"),
                format!("{:?}", v.r#type),
                format!("{}", v.contract_address),
                format!("{}", v.class_hash)
            ]
        ),
        Tables::StorageChangeHistory => tabular!(
            tables::StorageChangeHistory,
            "ContractStorageEntry",
            ["number", "key.contract_address", "key.key", "value"],
            |k, v| vec![
                format!("{k}"),
                format!("{}", v.key.contract_address),
                format!("{}", v.key.key),
                format!("{}", v.value)
            ]
        ),
        Tables::StageExecutionCheckpoints => tabular!(
            tables::StageExecutionCheckpoints,
            "ExecutionCheckpoint",
            ["stage_id", "block"],
            |k, v| vec![format!("{k}"), format!("{}", v.block)]
        ),
        Tables::StagePruningCheckpoints => tabular!(
            tables::StagePruningCheckpoints,
            "PruningCheckpoint",
            ["stage_id", "block"],
            |k, v| vec![format!("{k}"), format!("{}", v.block)]
        ),
        Tables::StateHistoryRetention => tabular!(
            tables::StateHistoryRetention,
            "HistoricalStateRetention",
            ["key", "earliest_available_block"],
            |k, v| vec![format!("{k}"), format!("{}", v.earliest_available_block)]
        ),
        Tables::MigrationCheckpoints => tabular!(
            tables::MigrationCheckpoints,
            "MigrationCheckpoint",
            ["stage_id", "last_key_migrated"],
            |k, v| vec![format!("{k}"), format!("{}", v.last_key_migrated)]
        ),

        // -- Header (convert VersionedHeader to Header for field access) --
        Tables::Headers => tabular!(
            tables::Headers,
            "Header",
            [
                "number",
                "parent_hash",
                "state_root",
                "transaction_count",
                "events_count",
                "state_diff_length",
                "timestamp",
                "sequencer_address",
                "l1_gas_prices.eth",
                "l1_gas_prices.strk",
                "l1_data_gas_prices.eth",
                "l1_data_gas_prices.strk",
                "l2_gas_prices.eth",
                "l2_gas_prices.strk",
                "l1_da_mode",
                "starknet_version"
            ],
            |k, v| {
                let h = katana_primitives::block::Header::from(v.clone());
                vec![
                    format!("{k}"),
                    format!("{}", h.parent_hash),
                    format!("{}", h.state_root),
                    format!("{}", h.transaction_count),
                    format!("{}", h.events_count),
                    format!("{}", h.state_diff_length),
                    format!("{}", h.timestamp),
                    format!("{}", h.sequencer_address),
                    format!("{}", h.l1_gas_prices.eth),
                    format!("{}", h.l1_gas_prices.strk),
                    format!("{}", h.l1_data_gas_prices.eth),
                    format!("{}", h.l1_data_gas_prices.strk),
                    format!("{}", h.l2_gas_prices.eth),
                    format!("{}", h.l2_gas_prices.strk),
                    format!("{:?}", h.l1_da_mode),
                    format!("{}", h.starknet_version),
                ]
            }
        ),

        // -- Complex values (detail / split-panel mode) --
        Tables::BlockStateUpdates => detail!(tables::BlockStateUpdates),
        Tables::Transactions => detail!(tables::Transactions),
        Tables::TxTraces => detail!(tables::TxTraces),
        Tables::Receipts => detail!(tables::Receipts),
        Tables::Classes => detail!(tables::Classes),
        Tables::ContractInfoChangeSet => detail!(tables::ContractInfoChangeSet),
        Tables::StorageChangeSet => detail!(debug tables::StorageChangeSet),

        // State trie tables are excluded
        _ => FetchResult { columns: vec![], rows: vec![], tabular: true, value_type: "" },
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
    /// Column headers for the currently open table
    columns: Vec<&'static str>,
    /// Row data for the currently open table
    rows: Vec<Vec<String>>,
    /// Whether the current table uses tabular display
    tabular: bool,
    /// Value type name (for tabular display header)
    value_type: &'static str,
    /// Total entry count for the currently open table
    current_table_count: usize,
    /// Current offset into the table for pagination
    entry_offset: usize,
    /// Selection state for key list
    entry_list: ListState,
    /// Scroll offset for the value panel (detail mode only)
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
            columns: Vec::new(),
            rows: Vec::new(),
            tabular: false,
            value_type: "",
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

    fn select_entry(&mut self, index: usize) {
        self.entry_list.select(Some(index));
        self.value_scroll = 0;
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
        let data = fetch_table_data(tx, table, 0, PAGE_SIZE);
        self.columns = data.columns;
        self.rows = data.rows;
        self.tabular = data.tabular;
        self.value_type = data.value_type;
        self.entry_list = ListState::default();
        if !self.rows.is_empty() {
            self.select_entry(0);
        }
        self.screen = Screen::EntryView;
    }

    /// Ensure the selected entry is within the loaded page, re-fetching if needed.
    fn ensure_entry_loaded<Tx: DbTx>(&mut self, tx: &Tx, absolute_index: usize) {
        let page_end = self.entry_offset + self.rows.len();
        if absolute_index >= page_end || absolute_index < self.entry_offset {
            let new_offset = absolute_index.saturating_sub(PAGE_SIZE / 4);
            let idx = self.selected_table_index();
            let table = INSPECT_TABLES[idx];
            let data = fetch_table_data(tx, table, new_offset, PAGE_SIZE);
            self.rows = data.rows;
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
        self.select_entry(new_abs - self.entry_offset);
    }

    fn jump_entry_first<Tx: DbTx>(&mut self, tx: &Tx) {
        if self.current_table_count == 0 {
            return;
        }
        self.ensure_entry_loaded(tx, 0);
        self.select_entry(0);
    }

    fn jump_entry_last<Tx: DbTx>(&mut self, tx: &Tx) {
        if self.current_table_count == 0 {
            return;
        }
        let last = self.current_table_count - 1;
        self.ensure_entry_loaded(tx, last);
        self.select_entry(last - self.entry_offset);
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
                        app.rows.clear();
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
                    KeyCode::Char('h') | KeyCode::Left if !app.tabular => {
                        app.value_scroll = app.value_scroll.saturating_sub(4);
                    }
                    KeyCode::Char('l') | KeyCode::Right if !app.tabular => {
                        app.value_scroll = app.value_scroll.saturating_add(4);
                    }
                    _ => {}
                },
            }
        }
    }
}

// -- Drawing --

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

    let footer = if app.tabular {
        " Esc:back  \u{2191}\u{2193}:nav  g/G:first/last  q:quit "
    } else {
        " Esc:back  \u{2191}\u{2193}:nav  h/l:scroll value  g/G:first/last  q:quit "
    };

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(footer).centered());

    let inner = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Split vertically: 1-line header + remaining content
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    // Split both header and content into the same column proportions
    let header_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(vchunks[0]);

    let content_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(vchunks[1]);

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
    let num_width = 6u16;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(num_width), Constraint::Min(1)])
        .split(area);

    // Line numbers
    let selected = app.selected_entry_index();
    let num_lines: Vec<Line<'_>> = app
        .rows
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

    let visible_height = cols[0].height as usize;
    let scroll_offset = selected.saturating_sub(visible_height.saturating_sub(1));
    let num_paragraph = Paragraph::new(num_lines).scroll((scroll_offset as u16, 0));
    f.render_widget(num_paragraph, cols[0]);

    // Key list (first column of each row)
    let key_area = cols[1];
    let items: Vec<ListItem<'_>> = app
        .rows
        .iter()
        .map(|row| {
            let key = row.first().map(|s| s.as_str()).unwrap_or("");
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

    if app.tabular {
        // Render value type name + field table
        let Some(row) = app.rows.get(selected) else {
            return;
        };

        let mut table = crate::cli::db::table();
        table.set_header(vec!["Field", "Value"]);
        for (col, val) in app.columns.iter().skip(1).zip(row.iter().skip(1)) {
            table.add_row(vec![col.to_string(), val.clone()]);
        }

        let text = format!("{}\n{table}", app.value_type);
        let paragraph = Paragraph::new(text);
        f.render_widget(paragraph, area);
    } else {
        // Render full Debug output with horizontal scroll
        let value_text =
            app.rows.get(selected).and_then(|row| row.get(1)).map(|s| s.as_str()).unwrap_or("");

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
}

// -- Helpers --

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
