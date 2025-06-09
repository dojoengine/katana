use std::fmt::Write as _;
use std::io::{self, stdout, Stdout};
use std::path::{self};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::Table;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use katana_db::abstraction::{Database, DbCursor, DbDupSortCursor, DbTx};
use katana_db::mdbx::{DbEnv, DbEnvKind};
use katana_db::tables::{Tables, NUM_TABLES};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};
use ratatui::{Frame, Terminal};

/// Create a human-readable byte unit string (eg. 16.00 KiB)
macro_rules! byte_unit {
    ($size:expr) => {
        format!(
            "{:.2}",
            byte_unit::Byte::from_u64($size as u64)
                .get_appropriate_unit(byte_unit::UnitType::Binary)
        )
    };
}

#[derive(Args)]
pub struct DbArgs {
    #[arg(short, long)]
    #[arg(global = true)]
    #[arg(help = "Path to the database directory")]
    #[arg(default_value = "~/.katana/db")]
    path: String,

    #[command(subcommand)]
    commands: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Retrieves database statistics")]
    Stats,
    #[command(about = "Browse database tables with interactive terminal UI")]
    Inspect(InspectArgs),
}

#[derive(Args)]
struct InspectArgs {
    #[arg(short, long)]
    #[arg(help = "Specific table to inspect")]
    table: Option<String>,
}

impl DbArgs {
    pub(crate) fn execute(self) -> Result<()> {
        match self.commands {
            Commands::Stats => {
                let db = open_db_ro(&self.path)?;
                let stats = db.stats()?;

                let mut table = table();
                let mut rows = Vec::with_capacity(NUM_TABLES);
                // total size of all tables (incl. freelist)
                let mut total_size = 0;

                table.set_header(vec![
                    "Table",
                    "Entries",
                    "Depth",
                    "Branch Pages",
                    "Leaf Pages",
                    "Overflow Pages",
                    "Size",
                ]);

                // page size is equal across all tables, so we can just get it from the first table
                // and use it to calculate for the freelist table.
                let mut pagesize: usize = 0;

                for (name, stat) in stats.table_stats().iter() {
                    let entries = stat.entries();
                    let depth = stat.depth();
                    let branch_pages = stat.branch_pages();
                    let leaf_pages = stat.leaf_pages();
                    let overflow_pages = stat.overflow_pages();
                    let size = stat.total_size();

                    rows.push(vec![
                        name.to_string(),
                        entries.to_string(),
                        depth.to_string(),
                        branch_pages.to_string(),
                        leaf_pages.to_string(),
                        overflow_pages.to_string(),
                        byte_unit!(size),
                    ]);

                    // increment the size of all tables
                    total_size += size;

                    if pagesize == 0 {
                        pagesize = stat.page_size() as usize;
                    }
                }

                // sort the rows by the table name
                rows.sort_by(|a, b| a[0].cmp(&b[0]));
                table.add_rows(rows);

                // add special row for the freelist table
                let freelist_size = stats.freelist() * pagesize;
                total_size += freelist_size;

                table.add_row(vec![
                    "Freelist".to_string(),
                    stats.freelist().to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    byte_unit!(freelist_size),
                ]);

                // add the last row for the total size
                table.add_row(vec![
                    "Total Size".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    byte_unit!(total_size),
                ]);

                println!("{table}");
            }
            Commands::Inspect(args) => {
                let db = open_db_ro(&self.path)?;
                run_inspector(db, args)?;
            }
        }

        Ok(())
    }
}

/// Open the database at `path` in read-only mode.
///
/// The path is expanded and resolved to an absolute path before opening the database for clearer
/// error messages.
fn open_db_ro(path: &str) -> Result<DbEnv> {
    let path = path::absolute(shellexpand::full(path)?.into_owned())?;
    DbEnv::open(&path, DbEnvKind::RO).with_context(|| {
        format!("Opening database file in read-only mode at path {}", path.display())
    })
}

/// Create a table with the default UTF-8 full border and rounded corners.
fn table() -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
    table
}

fn run_inspector(db: DbEnv, args: InspectArgs) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_tui(&mut terminal, db, args);
    restore_terminal(terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    let mut stdout = stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

struct InspectorApp {
    table_list_state: ListState,
    current_table: Option<Tables>,
    keys: Vec<String>,
    values: Vec<String>,
    key_list_state: ListState,
    value_list_state: ListState,
    focused_panel: FocusedPanel,
}

#[derive(PartialEq)]
enum FocusedPanel {
    Tables,
    Keys,
    Values,
}

impl InspectorApp {
    fn new(initial_table: Option<String>) -> Self {
        let mut app = Self {
            table_list_state: ListState::default(),
            current_table: None,
            keys: Vec::new(),
            values: Vec::new(),
            key_list_state: ListState::default(),
            value_list_state: ListState::default(),
            focused_panel: FocusedPanel::Tables,
        };

        if let Some(table_name) = initial_table {
            if let Ok(table) = table_name.parse::<Tables>() {
                app.current_table = Some(table);
                app.focused_panel = FocusedPanel::Keys;
            }
        }

        app.table_list_state.select(Some(0));
        app
    }

    fn next_table(&mut self) {
        let i = match self.table_list_state.selected() {
            Some(i) => {
                if i >= Tables::ALL.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_list_state.select(Some(i));
    }

    fn previous_table(&mut self) {
        let i = match self.table_list_state.selected() {
            Some(i) => {
                if i == 0 {
                    Tables::ALL.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_list_state.select(Some(i));
    }

    fn select_current_table(&mut self) {
        if let Some(i) = self.table_list_state.selected() {
            self.current_table = Some(Tables::ALL[i]);
            self.focused_panel = FocusedPanel::Keys;
            self.key_list_state.select(Some(0));
            self.value_list_state.select(Some(0));
        }
    }

    fn next_key(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        let i = match self.key_list_state.selected() {
            Some(i) => {
                if i >= self.keys.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.key_list_state.select(Some(i));
        self.value_list_state.select(Some(i));
    }

    fn previous_key(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        let i = match self.key_list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.keys.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.key_list_state.select(Some(i));
        self.value_list_state.select(Some(i));
    }

    fn back_to_tables(&mut self) {
        self.focused_panel = FocusedPanel::Tables;
        self.current_table = None;
        self.keys.clear();
        self.values.clear();
    }
}

fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    db: DbEnv,
    args: InspectArgs,
) -> Result<()> {
    let mut app = InspectorApp::new(args.table);

    loop {
        if let Some(table) = app.current_table {
            let (keys, values) = load_table_key_values(&db, table)?;
            app.keys = keys;
            app.values = values;
        }

        terminal.draw(|f| ui(f, &mut app))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Esc => {
                            if matches!(app.focused_panel, FocusedPanel::Keys | FocusedPanel::Values) {
                                app.back_to_tables();
                            } else {
                                return Ok(());
                            }
                        }
                        KeyCode::Tab => {
                            match app.focused_panel {
                                FocusedPanel::Tables => {
                                    if app.current_table.is_none() {
                                        // Auto-select the current table when tabbing to Keys panel
                                        app.select_current_table();
                                    } else {
                                        app.focused_panel = FocusedPanel::Keys;
                                    }
                                }
                                FocusedPanel::Keys => {
                                    app.focused_panel = FocusedPanel::Values;
                                }
                                FocusedPanel::Values => {
                                    app.focused_panel = FocusedPanel::Tables;
                                }
                            }
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            match app.focused_panel {
                                FocusedPanel::Tables => {
                                    if app.current_table.is_none() {
                                        // Auto-select the current table when navigating to Keys panel
                                        app.select_current_table();
                                    } else {
                                        app.focused_panel = FocusedPanel::Keys;
                                    }
                                }
                                FocusedPanel::Keys => {
                                    app.focused_panel = FocusedPanel::Values;
                                }
                                FocusedPanel::Values => {
                                    // Stay on Values (rightmost panel)
                                }
                            }
                        }
                        KeyCode::Left | KeyCode::Char('h') => {
                            app.focused_panel = match app.focused_panel {
                                FocusedPanel::Tables => FocusedPanel::Tables, // Stay on Tables
                                FocusedPanel::Keys => FocusedPanel::Tables,
                                FocusedPanel::Values => FocusedPanel::Keys,
                            };
                        }
                        KeyCode::Down | KeyCode::Char('j') => match app.focused_panel {
                            FocusedPanel::Tables => app.next_table(),
                            FocusedPanel::Keys | FocusedPanel::Values => app.next_key(),
                        },
                        KeyCode::Up | KeyCode::Char('k') => match app.focused_panel {
                            FocusedPanel::Tables => app.previous_table(),
                            FocusedPanel::Keys | FocusedPanel::Values => app.previous_key(),
                        },
                        KeyCode::Enter => {
                            if app.focused_panel == FocusedPanel::Tables {
                                app.select_current_table();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut InspectorApp) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)].as_ref())
        .split(f.area());

    draw_table_list(f, chunks[0], app);
    
    if app.current_table.is_some() {
        let key_value_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(chunks[1]);
            
        draw_keys(f, key_value_chunks[0], app);
        draw_values(f, key_value_chunks[1], app);
    } else {
        draw_empty_panel(f, chunks[1], app);
    }
}

fn draw_table_list(f: &mut Frame, area: Rect, app: &mut InspectorApp) {
    let items: Vec<ListItem> = Tables::ALL
        .iter()
        .map(|table| {
            let content = vec![Line::from(Span::raw(table.name()))];
            ListItem::new(content)
        })
        .collect();

    let title =
        if app.focused_panel == FocusedPanel::Tables { "Tables (focused)" } else { "Tables" };

    let style = if app.focused_panel == FocusedPanel::Tables {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, area, &mut app.table_list_state);

    if app.focused_panel == FocusedPanel::Tables {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(Tables::ALL.len())
            .position(app.table_list_state.selected().unwrap_or(0));
        f.render_stateful_widget(
            scrollbar,
            area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }
}

fn draw_keys(f: &mut Frame, area: Rect, app: &mut InspectorApp) {
    let title = if app.focused_panel == FocusedPanel::Keys {
        "Keys (focused)"
    } else {
        "Keys"
    };

    if app.keys.is_empty() {
        let block = Block::default().borders(Borders::ALL).title(title);
        let paragraph = Paragraph::new("No keys found").block(block);
        f.render_widget(paragraph, area);
    } else {
        let items: Vec<ListItem> = app
            .keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let content = vec![Line::from(vec![
                    Span::styled(format!("{:4}: ", i), Style::default().fg(Color::DarkGray)),
                    Span::raw(key),
                ])];
                ListItem::new(content)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol(">> ");

        f.render_stateful_widget(list, area, &mut app.key_list_state);

        if app.focused_panel == FocusedPanel::Keys {
            let scrollbar = Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));
            let mut scrollbar_state = ScrollbarState::default()
                .content_length(app.keys.len())
                .position(app.key_list_state.selected().unwrap_or(0));
            f.render_stateful_widget(
                scrollbar,
                area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }),
                &mut scrollbar_state,
            );
        }
    }
}

fn draw_values(f: &mut Frame, area: Rect, app: &mut InspectorApp) {
    let title = if app.focused_panel == FocusedPanel::Values {
        "Values (focused)"
    } else {
        "Values"
    };

    if app.values.is_empty() {
        let block = Block::default().borders(Borders::ALL).title(title);
        let paragraph = Paragraph::new("No values found").block(block);
        f.render_widget(paragraph, area);
    } else {
        let items: Vec<ListItem> = app
            .values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                let content = vec![Line::from(vec![
                    Span::styled(format!("{:4}: ", i), Style::default().fg(Color::DarkGray)),
                    Span::raw(value),
                ])];
                ListItem::new(content)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol(">> ");

        f.render_stateful_widget(list, area, &mut app.value_list_state);

        if app.focused_panel == FocusedPanel::Values {
            let scrollbar = Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));
            let mut scrollbar_state = ScrollbarState::default()
                .content_length(app.values.len())
                .position(app.value_list_state.selected().unwrap_or(0));
            f.render_stateful_widget(
                scrollbar,
                area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }),
                &mut scrollbar_state,
            );
        }
    }
}

fn draw_empty_panel(f: &mut Frame, area: Rect, app: &mut InspectorApp) {
    let block = Block::default().borders(Borders::ALL).title("Select a table");
    let content = "Press Enter to view table entries\n\nControls:\n- Up/Down or j/k: Navigate items\n- Left/Right or h/l: Switch panels\n- Enter: Select table\n- Tab: Cycle panels\n- Esc: Back/Exit\n- q: Quit";
    let paragraph = Paragraph::new(content).block(block);
    f.render_widget(paragraph, area);
}

fn load_table_key_values(db: &DbEnv, table: Tables) -> Result<(Vec<String>, Vec<String>)> {
    let tx = db.tx()?;
    let mut keys = Vec::new();
    let mut values = Vec::new();

    match table {
        Tables::Headers => load_key_values::<katana_db::tables::Headers>(&tx, &mut keys, &mut values)?,
        Tables::BlockHashes => load_key_values::<katana_db::tables::BlockHashes>(&tx, &mut keys, &mut values)?,
        Tables::BlockNumbers => load_key_values::<katana_db::tables::BlockNumbers>(&tx, &mut keys, &mut values)?,
        Tables::BlockBodyIndices => {
            load_key_values::<katana_db::tables::BlockBodyIndices>(&tx, &mut keys, &mut values)?
        }
        Tables::BlockStatusses => {
            load_key_values::<katana_db::tables::BlockStatusses>(&tx, &mut keys, &mut values)?
        }
        Tables::TxNumbers => load_key_values::<katana_db::tables::TxNumbers>(&tx, &mut keys, &mut values)?,
        Tables::TxBlocks => load_key_values::<katana_db::tables::TxBlocks>(&tx, &mut keys, &mut values)?,
        Tables::TxHashes => load_key_values::<katana_db::tables::TxHashes>(&tx, &mut keys, &mut values)?,
        Tables::TxTraces => load_key_values::<katana_db::tables::TxTraces>(&tx, &mut keys, &mut values)?,
        Tables::Transactions => load_key_values::<katana_db::tables::Transactions>(&tx, &mut keys, &mut values)?,
        Tables::Receipts => load_key_values::<katana_db::tables::Receipts>(&tx, &mut keys, &mut values)?,
        Tables::CompiledClassHashes => {
            load_key_values::<katana_db::tables::CompiledClassHashes>(&tx, &mut keys, &mut values)?
        }
        Tables::Classes => load_key_values::<katana_db::tables::Classes>(&tx, &mut keys, &mut values)?,
        Tables::ContractInfo => load_key_values::<katana_db::tables::ContractInfo>(&tx, &mut keys, &mut values)?,
        Tables::ContractStorage => {
            load_dupsort_key_values::<katana_db::tables::ContractStorage>(&tx, &mut keys, &mut values)?
        }
        Tables::ClassDeclarationBlock => {
            load_key_values::<katana_db::tables::ClassDeclarationBlock>(&tx, &mut keys, &mut values)?
        }
        Tables::ClassDeclarations => {
            load_dupsort_key_values::<katana_db::tables::ClassDeclarations>(&tx, &mut keys, &mut values)?
        }
        Tables::ContractInfoChangeSet => {
            load_key_values::<katana_db::tables::ContractInfoChangeSet>(&tx, &mut keys, &mut values)?
        }
        Tables::NonceChangeHistory => {
            load_dupsort_key_values::<katana_db::tables::NonceChangeHistory>(&tx, &mut keys, &mut values)?
        }
        Tables::ClassChangeHistory => {
            load_dupsort_key_values::<katana_db::tables::ClassChangeHistory>(&tx, &mut keys, &mut values)?
        }
        Tables::StorageChangeHistory => {
            load_dupsort_key_values::<katana_db::tables::StorageChangeHistory>(&tx, &mut keys, &mut values)?
        }
        Tables::StorageChangeSet => {
            load_key_values::<katana_db::tables::StorageChangeSet>(&tx, &mut keys, &mut values)?
        }
        Tables::StageCheckpoints => {
            load_key_values::<katana_db::tables::StageCheckpoints>(&tx, &mut keys, &mut values)?
        }
        Tables::ClassesTrie => load_key_values::<katana_db::tables::ClassesTrie>(&tx, &mut keys, &mut values)?,
        Tables::ContractsTrie => {
            load_key_values::<katana_db::tables::ContractsTrie>(&tx, &mut keys, &mut values)?
        }
        Tables::StoragesTrie => load_key_values::<katana_db::tables::StoragesTrie>(&tx, &mut keys, &mut values)?,
        Tables::ClassesTrieHistory => {
            load_dupsort_key_values::<katana_db::tables::ClassesTrieHistory>(&tx, &mut keys, &mut values)?
        }
        Tables::ContractsTrieHistory => {
            load_dupsort_key_values::<katana_db::tables::ContractsTrieHistory>(&tx, &mut keys, &mut values)?
        }
        Tables::StoragesTrieHistory => {
            load_dupsort_key_values::<katana_db::tables::StoragesTrieHistory>(&tx, &mut keys, &mut values)?
        }
        Tables::ClassesTrieChangeSet => {
            load_key_values::<katana_db::tables::ClassesTrieChangeSet>(&tx, &mut keys, &mut values)?
        }
        Tables::ContractsTrieChangeSet => {
            load_key_values::<katana_db::tables::ContractsTrieChangeSet>(&tx, &mut keys, &mut values)?
        }
        Tables::StoragesTrieChangeSet => {
            load_key_values::<katana_db::tables::StoragesTrieChangeSet>(&tx, &mut keys, &mut values)?
        }
    }

    tx.commit()?;
    Ok((keys, values))
}

fn load_key_values<T>(tx: &impl DbTx, keys: &mut Vec<String>, values: &mut Vec<String>) -> Result<()>
where
    T: katana_db::tables::Table,
    T::Key: std::fmt::Debug,
    T::Value: std::fmt::Debug,
{
    let mut cursor = tx.cursor::<T>()?;
    let walker = cursor.walk(None)?;

    let mut count = 0;
    for result in walker {
        if count >= 1000 {
            // Limit to 1000 entries to avoid memory issues
            keys.push("... (truncated, showing first 1000 entries)".to_string());
            values.push("... (truncated, showing first 1000 entries)".to_string());
            break;
        }

        match result {
            Ok((key, value)) => {
                keys.push(format!("{:?}", key));
                values.push(format!("{:?}", value));
                count += 1;
            }
            Err(e) => {
                keys.push(format!("Error reading entry: {}", e));
                values.push("".to_string());
                break;
            }
        }
    }

    if count == 0 {
        keys.push("No entries found".to_string());
        values.push("".to_string());
    }

    Ok(())
}

fn load_dupsort_key_values<T>(tx: &impl DbTx, keys: &mut Vec<String>, values: &mut Vec<String>) -> Result<()>
where
    T: katana_db::tables::DupSort,
    T::Key: std::fmt::Debug,
    T::Value: std::fmt::Debug,
    T::SubKey: std::fmt::Debug,
{
    let mut cursor = tx.cursor_dup::<T>()?;
    let mut walker = cursor.walk_dup(None, None)?;

    if let Some(walker) = walker {
        let mut count = 0;
        for result in walker {
            if count >= 1000 {
                // Limit to 1000 entries to avoid memory issues
                keys.push("... (truncated, showing first 1000 entries)".to_string());
                values.push("... (truncated, showing first 1000 entries)".to_string());
                break;
            }

            match result {
                Ok((key, value)) => {
                    keys.push(format!("{:?}", key));
                    values.push(format!("{:?}", value));
                    count += 1;
                }
                Err(e) => {
                    keys.push(format!("Error reading entry: {}", e));
                    values.push("".to_string());
                    break;
                }
            }
        }

        if count == 0 {
            keys.push("No entries found".to_string());
            values.push("".to_string());
        }
    } else {
        keys.push("No entries found".to_string());
        values.push("".to_string());
    }

    Ok(())
}
