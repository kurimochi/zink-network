use alloy::{
    primitives::{Address, U256, utils::format_ether},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    transports::ws::WsConnect,
};
use clap::Parser;
use common::{
    chain::ZinKNet,
    config::CommonConfig,
    p2p::{Behaviour, SwarmExt},
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use dotenv::dotenv;
use libp2p::futures::StreamExt;
use log::LevelFilter;
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Tabs},
};
use std::{collections::HashMap, error::Error, io, time::Duration};
use tokio::select;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerWidget};

mod chain;
mod p2p;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    common: CommonConfig,
}

// Data structure for a task that is ready for execution
#[derive(Clone)]
pub struct ReadyTask {
    pub task_id: U256,
    pub requestor: Address,
    pub reward: U256,
    pub declarations: U256,
}

// Application state
struct App {
    running: bool,
    active_tab: usize,
    user_addr: Address,
    local_peer_id: libp2p::PeerId,
    listeners: Vec<String>,
    block_number: Option<u64>,
    connected_peers: usize,
    pending_tasks: HashMap<U256, (Address, U256)>, // taskId -> (requestor, reward)
    pending_elfs: HashMap<U256, (Vec<u8>, Address)>, // taskId -> (elf_bytes, signer)
    ready_tasks: HashMap<U256, ReadyTask>,
    table_state: TableState,
}

impl App {
    fn new(user_addr: Address, local_peer_id: libp2p::PeerId) -> Self {
        Self {
            running: true,
            active_tab: 0,
            user_addr,
            local_peer_id,
            listeners: Vec::new(),
            block_number: None,
            connected_peers: 0,
            pending_tasks: HashMap::new(),
            pending_elfs: HashMap::new(),
            ready_tasks: HashMap::new(),
            table_state: TableState::default(),
        }
    }

    pub fn next_task(&mut self) {
        if self.ready_tasks.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i >= self.ready_tasks.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub fn previous_task(&mut self) {
        if self.ready_tasks.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.ready_tasks.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // --- Logger Setup ---
    tui_logger::init_logger(LevelFilter::Info).unwrap();
    tracing_subscriber::registry()
        .with(tui_logger::TuiTracingSubscriberLayer)
        .init();

    dotenv().ok();
    let cli = Cli::parse();

    // --- Blockchain & P2P Setup ---
    let signer: PrivateKeySigner = cli.common.private_key.parse()?;
    let user_addr = signer.address();
    let ws = WsConnect::new(&cli.common.rpc_url);
    let provider = ProviderBuilder::new().connect_ws(ws).await?;
    let zinknet = ZinKNet::new(provider.clone(), cli.common.contract);
    let mut chain_stream = zinknet.setup_ethlistener_polling().await?;
    let mut swarm = Behaviour::new_swarm()?;
    let _ = swarm.subscribe("test")?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
    let local_peer_id = *swarm.local_peer_id();

    // --- TUI Setup ---
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // --- App Creation and Main Loop ---
    let mut app = App::new(user_addr, local_peer_id);

    let res = run_app(
        &mut terminal,
        &mut app,
        &mut chain_stream,
        &mut swarm,
        &provider,
        &zinknet,
    )
    .await;

    // --- TUI Cleanup ---
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Error: {:?}", err);
    }

    Ok(())
}

async fn run_app<B, P, T>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    chain_stream: &mut T,
    swarm: &mut libp2p::Swarm<Behaviour>,
    provider: &P,
    zinknet: &ZinKNet<P>,
) -> io::Result<()>
where
    B: Backend,
    P: Provider + Send + Sync + 'static,
    T: StreamExt<Item = alloy::rpc::types::Log> + Unpin,
{
    let mut block_update_interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        terminal.draw(|f| ui(f, app))?;

        // Handle key inputs (non-blocking)
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => app.running = false,
                    KeyCode::Right => app.active_tab = (app.active_tab + 1) % 2,
                    KeyCode::Left => app.active_tab = (app.active_tab + 2 - 1) % 2,
                    KeyCode::Down => app.next_task(),
                    KeyCode::Up => app.previous_task(),
                    _ => {}
                }
            }
        }

        // Handle async events
        select! {
            biased;

            _ = block_update_interval.tick() => {
                if let Ok(bn) = provider.get_block_number().await {
                    app.block_number = Some(bn);
                }
            }
            Some(log) = chain_stream.next() => {
                if let Err(e) = chain::handle_blockchain_event(
                    log,
                    zinknet,
                    &mut app.pending_tasks,
                    &mut app.pending_elfs,
                    &mut app.ready_tasks,
                ).await {
                    log::error!("Failed to handle blockchain event: {}", e);
                }
            }
            event = swarm.select_next_some() => {
                if let libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } = &event {
                    app.listeners.push(address.to_string());
                }
                app.connected_peers = swarm.behaviour().gossipsub.all_peers().count();
                if let Err(e) = p2p::handle_swarm_event(
                    event,
                    swarm,
                    zinknet,
                    &mut app.pending_tasks,
                    &mut app.pending_elfs,
                    &mut app.ready_tasks,
                ).await {
                    log::error!("Failed to handle swarm event: {}", e);
                }
            }
            // Default branch to prevent select! from blocking forever
            _ = tokio::time::sleep(Duration::from_millis(1)) => {}
        }

        if !app.running {
            return Ok(());
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3), // Header
                Constraint::Length(3), // Tabs
                Constraint::Min(0),    // Main content
                Constraint::Length(8), // Log panel
                Constraint::Length(3), // Footer
            ]
            .as_ref(),
        )
        .split(area);

    // Header
    let header_title = format!(
        " ZinKNet Node [Block: {} | Peers: {}] ",
        app.block_number
            .map_or_else(|| "...".to_string(), |b| b.to_string()),
        app.connected_peers
    );
    let header =
        Paragraph::new(header_title).block(Block::default().borders(Borders::ALL).title("Header"));
    f.render_widget(header, chunks[0]);

    // Tabs
    let titles: Vec<_> = ["Tasks", "My Info"]
        .iter()
        .cloned()
        .map(Line::from)
        .collect();
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Tabs"))
        .select(app.active_tab)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[1]);

    // Main Content Area
    let main_chunk = chunks[2];
    match app.active_tab {
        0 => {
            let header_cells = ["Task ID", "Requestor", "Reward (ETH)", "Declarations"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().fg(Color::Red)));
            let header = Row::new(header_cells).height(1).bottom_margin(1);

            let rows = app.ready_tasks.values().map(|task| {
                let reward_eth = format_ether(task.reward);
                let cells = vec![
                    Cell::from(task.task_id.to_string()),
                    Cell::from(task.requestor.to_string()),
                    Cell::from(reward_eth),
                    Cell::from(task.declarations.to_string()),
                ];
                Row::new(cells).height(1)
            });

            let widths = [
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ];
            let table = Table::new(rows, widths)
                .header(header)
                .block(Block::default().borders(Borders::ALL).title("Ready Tasks"))
                .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            f.render_stateful_widget(table, main_chunk, &mut app.table_state);
        }
        1 => {
            let mut info_lines = vec![
                Line::from(vec![
                    Span::raw("Blockchain Address: "),
                    Span::styled(app.user_addr.to_string(), Style::default().fg(Color::Cyan)),
                ]),
                Line::from(vec![
                    Span::raw("Peer ID: "),
                    Span::styled(
                        app.local_peer_id.to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                ]),
            ];
            for listener in &app.listeners {
                info_lines.push(Line::from(vec![
                    Span::raw("Listening on: "),
                    Span::styled(listener, Style::default().fg(Color::Cyan)),
                ]));
            }
            let info_paragraph = Paragraph::new(info_lines)
                .block(Block::default().borders(Borders::ALL).title("My Info"));
            f.render_widget(info_paragraph, main_chunk);
        }
        _ => unreachable!(),
    };

    // Log Panel
    let logger_widget = TuiLoggerWidget::default()
        .block(Block::default().title("Logs").borders(Borders::ALL))
        .output_separator('|')
        .output_timestamp(Some("%H:%M:%S".to_string()))
        .output_level(Some(TuiLoggerLevelOutput::Abbreviated))
        .output_target(false)
        .output_file(false)
        .output_line(false)
        .style_error(Style::default().fg(Color::Red))
        .style_warn(Style::default().fg(Color::Yellow))
        .style_info(Style::default().fg(Color::Cyan));
    f.render_widget(logger_widget, chunks[3]);

    // Footer
    let footer_content = Paragraph::new("[←/→: Switch Tab] [↑/↓: Scroll] [q: Quit]")
        .style(Style::default().fg(Color::LightCyan));
    let footer = footer_content.block(Block::default().borders(Borders::ALL).title("Footer"));
    f.render_widget(footer, chunks[4]);
}
