use alloy::{
    primitives::{Address, U256, utils::format_ether},
    providers::{Provider, ProviderBuilder},
    signers::{Signer, local::PrivateKeySigner},
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
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Tabs},
};
use sp1_sdk::{SP1ProofWithPublicValues, SP1Stdin};
use std::{
    collections::HashMap,
    error::Error,
    io::{self, Stdout},
    time::Duration,
};
use tokio::{select, sync::mpsc};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerWidget, TuiTracingSubscriberLayer};

mod chain;
mod p2p;
mod zkvm;

type TaskResult = Result<(), String>;
type CalculationResult = Result<SP1ProofWithPublicValues, String>;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    common: CommonConfig,
}

// Data structure for a competition that is ready for execution
#[derive(Clone)]
pub struct ReadyCompetition {
    pub competition_id: U256,
    pub issuer: Address,
    pub reward: U256,
    pub competitors: U256,
    pub elf: Vec<u8>,
    pub stdin: SP1Stdin,
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
    chain_only_competitions: HashMap<U256, (Address, U256)>, // competitionId -> (issuer, reward)
    elf_only_competitions: HashMap<U256, (Vec<u8>, SP1Stdin, Address)>, // competitionId -> (elf_bytes, stdin, signer)
    ready_competitions: HashMap<U256, ReadyCompetition>,
    table_state: TableState,
    active_competition_id: Option<U256>,
    processing_competition: Option<ReadyCompetition>,
    // UI state for async tasks
    is_joining: bool,
    is_leaving: bool,
    is_calculating: bool,
    show_popup: bool,
    popup_title: String,
    popup_content: String,
    join_result_receiver: mpsc::Receiver<TaskResult>,
    leave_result_receiver: mpsc::Receiver<TaskResult>,
    calculation_result_receiver: mpsc::Receiver<CalculationResult>,
    joining_competition_id: Option<U256>,
}

impl App {
    async fn new<P: Provider + Clone, S>(
        user_addr: Address,
        zinknet: &ZinKNet<P, S>,
        local_peer_id: libp2p::PeerId,
        join_result_receiver: mpsc::Receiver<TaskResult>,
        leave_result_receiver: mpsc::Receiver<TaskResult>,
        calculation_result_receiver: mpsc::Receiver<CalculationResult>,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            running: true,
            active_tab: 0,
            user_addr,
            local_peer_id,
            listeners: Vec::new(),
            block_number: None,
            connected_peers: 0,
            chain_only_competitions: HashMap::new(),
            elf_only_competitions: HashMap::new(),
            ready_competitions: HashMap::new(),
            table_state: TableState::default(),
            active_competition_id: {
                let active_competition_id =
                    zinknet.contract.activeCompetition(user_addr).call().await?;
                if active_competition_id != U256::ZERO {
                    Some(active_competition_id)
                } else {
                    None
                }
            },
            processing_competition: None,
            is_joining: false,
            is_leaving: false,
            is_calculating: false,
            show_popup: false,
            popup_title: String::new(),
            popup_content: String::new(),
            join_result_receiver,
            leave_result_receiver,
            calculation_result_receiver,
            joining_competition_id: None,
        })
    }

    pub fn next_competition(&mut self) {
        if self.ready_competitions.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i >= self.ready_competitions.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub fn previous_competition(&mut self) {
        if self.ready_competitions.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.ready_competitions.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }
}

struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut tui = Tui::new()?;
    let result = run(&mut tui.terminal).await;
    result
}

async fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), Box<dyn Error>> {
    dotenv().ok();

    // --- Logger Setup ---
    tui_logger::init_logger(log::LevelFilter::Info)?;
    if std::env::var("RUST_LOG").is_ok() {
        tui_logger::set_env_filter_from_env(None);
    }
    let env_filter = tracing_subscriber::filter::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::filter::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(TuiTracingSubscriberLayer)
        .init();

    let cli = Cli::parse();

    // --- Blockchain & P2P Setup ---
    let signer: PrivateKeySigner = cli.common.private_key.parse()?;
    let user_addr = signer.address();
    let ws = WsConnect::new(&cli.common.rpc_url);
    let provider = ProviderBuilder::new().connect_ws(ws).await?;
    let zinknet = ZinKNet::new(provider.clone(), cli.common.contract, signer);
    let mut chain_stream = zinknet.setup_ethlistener_polling().await?;
    let mut swarm = Behaviour::new_swarm()?;
    let _ = swarm.subscribe("test")?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
    let local_peer_id = *swarm.local_peer_id();

    // --- App Creation and Main Loop ---
    let (join_result_sender, join_result_receiver) = mpsc::channel(1);
    let (leave_result_sender, leave_result_receiver) = mpsc::channel(1);
    let (calculation_result_sender, calculation_result_receiver) = mpsc::channel(1);
    let mut app = App::new(
        user_addr,
        &zinknet,
        local_peer_id,
        join_result_receiver,
        leave_result_receiver,
        calculation_result_receiver,
    )
    .await?;

    let res = run_app(
        terminal,
        &mut app,
        &mut chain_stream,
        &mut swarm,
        &provider,
        &zinknet,
        join_result_sender,
        leave_result_sender,
        calculation_result_sender,
    )
    .await;

    if let Err(err) = res {
        println!("Error: {:?}", err);
    }

    Ok(())
}

async fn run_app<B, P, S, T>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    chain_stream: &mut T,
    swarm: &mut libp2p::Swarm<Behaviour>,
    provider: &P,
    zinknet: &ZinKNet<P, S>,
    join_result_sender: mpsc::Sender<TaskResult>,
    leave_result_sender: mpsc::Sender<TaskResult>,
    calculation_result_sender: mpsc::Sender<CalculationResult>,
) -> io::Result<()>
where
    B: Backend,
    P: Provider + Send + Sync + 'static + Clone,
    S: Signer + Send + Sync + 'static + Clone,
    T: StreamExt<Item = alloy::rpc::types::Log> + Unpin,
{
    let mut block_update_interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        terminal.draw(|f| ui(f, app))?;

        // Handle key inputs (non-blocking)
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if app.is_joining || app.is_leaving || app.is_calculating {
                    match key.code {
                        KeyCode::Char('c') => {
                            if app.show_popup {
                                app.show_popup = false;
                            }
                        }
                        KeyCode::Char('v') => {
                            if !app.show_popup {
                                app.show_popup = true;
                                // Potentially restore popup content based on is_joining vs is_leaving
                            }
                        }
                        _ => {}
                    }
                } else if app.show_popup {
                    // If a result popup is shown, any key dismisses it
                    app.show_popup = false;
                } else {
                    match key.code {
                        KeyCode::Char('q') => app.running = false,
                        KeyCode::Right => app.active_tab = (app.active_tab + 1) % 3,
                        KeyCode::Left => app.active_tab = (app.active_tab + 3 - 1) % 3,
                        KeyCode::Down => {
                            if app.active_tab == 0 {
                                app.next_competition()
                            }
                        }
                        KeyCode::Up => {
                            if app.active_tab == 0 {
                                app.previous_competition()
                            }
                        }
                        KeyCode::Char('j') => {
                            if app.active_tab == 0
                                && app.active_competition_id.is_none()
                                && app.table_state.selected().is_some()
                            {
                                if let Some(selected_index) = app.table_state.selected() {
                                    if let Some(competition) =
                                        app.ready_competitions.values().nth(selected_index)
                                    {
                                        app.is_joining = true;
                                        app.show_popup = true;
                                        app.joining_competition_id =
                                            Some(competition.competition_id);
                                        app.popup_title = "Joining Competition".to_string();
                                        app.popup_content = format!(
                                            "Attempting to join competition {}...",
                                            competition.competition_id
                                        );

                                        let sender = join_result_sender.clone();
                                        let zinknet_clone = zinknet.clone();
                                        let competition_id = competition.competition_id;

                                        tokio::spawn(async move {
                                            let result = zinknet_clone
                                                .join_competition(competition_id)
                                                .await;
                                            let _ = sender
                                                .send(result.map(|_| ()).map_err(|e| e.to_string()))
                                                .await;
                                        });
                                    }
                                }
                            }
                        }
                        KeyCode::Char('l') => {
                            if app.active_tab == 1
                                && app.processing_competition.is_some()
                                && !app.is_calculating
                            {
                                app.is_leaving = true;
                                app.show_popup = true;
                                app.popup_title = "Leaving Competition".to_string();
                                app.popup_content =
                                    "Attempting to leave competition...".to_string();

                                let sender = leave_result_sender.clone();
                                let zinknet_clone = zinknet.clone();

                                tokio::spawn(async move {
                                    let result = zinknet_clone.leave_competition().await;
                                    let _ = sender
                                        .send(result.map(|_| ()).map_err(|e| e.to_string()))
                                        .await;
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Handle async events
        select! {
            biased;

            // Handle competition join results
            Some(result) = app.join_result_receiver.recv() => {
                app.is_joining = false;
                match result {
                    Ok(_) => {
                        // Move competition to processing
                        if let Some(joined_id) = app.joining_competition_id {
                            if let Some(competition_to_move) = app.ready_competitions.remove(&joined_id) {
                                app.processing_competition = Some(competition_to_move.clone());

                                // Start calculation
                                app.is_calculating = true;
                                app.popup_title = "Calculating".to_string();
                                app.popup_content = format!("Generating proof for competition {}...", joined_id);
                                app.show_popup = true;

                                let calculation_sender = calculation_result_sender.clone();
                                tokio::spawn(async move {
                                    let result = zkvm::sp1_proof(&competition_to_move.elf, &competition_to_move.stdin)
                                        .map_err(|e| e.to_string());
                                    let _ = calculation_sender.send(result).await;
                                });
                            }
                        }
                        app.table_state.select(None); // Deselect table row
                    }
                    Err(e) => {
                        app.popup_title = "Error".to_string();
                        app.popup_content = format!("Failed to join competition: {}

Press any key to close.", e);
                        app.show_popup = true;
                    }
                }
                app.joining_competition_id = None;
            }

            // Handle calculation results
            Some(result) = app.calculation_result_receiver.recv() => {
                app.is_calculating = false;
                match result {
                    Ok(_proof) => {
                        // TODO: Handle the proof (e.g., submit to blockchain)
                        app.popup_title = "Calculation Complete".to_string();
                        app.popup_content = "Successfully generated proof.

(Proof submission not yet implemented)

Press any key to close.".to_string();
                        log::info!("Successfully generated proof.");
                    }
                    Err(e) => {
                        app.popup_title = "Error".to_string();
                        app.popup_content = format!("Failed to generate proof: {}

Press any key to close.", e);
                        log::error!("Failed to generate proof: {}", e);
                    }
                }
                app.show_popup = true;
            }

            // Handle competition leave results
            Some(result) = app.leave_result_receiver.recv() => {
                app.is_leaving = false;
                match result {
                    Ok(_) => {
                        app.popup_title = "Success".to_string();
                        app.popup_content = "Successfully left the competition.

Press any key to close.".to_string();
                        if let Some(leaving_competition) = app.processing_competition.take() {
                            app.ready_competitions.insert(leaving_competition.competition_id, leaving_competition);
                        }
                    }
                    Err(e) => {
                        app.popup_title = "Error".to_string();
                        app.popup_content = format!("Failed to leave competition: {}

Press any key to close.", e);
                    }
                }
                 app.show_popup = true;
            }

            _ = block_update_interval.tick() => {
                if let Ok(bn) = provider.get_block_number().await {
                    app.block_number = Some(bn);
                }
            }
            Some(log) = chain_stream.next() => {
                if let Err(e) = chain::handle_blockchain_event(
                    log,
                    zinknet,
                    app.user_addr,
                    &mut app.chain_only_competitions,
                    &mut app.elf_only_competitions,
                    &mut app.ready_competitions,
                    &mut app.active_competition_id,
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
                    &mut app.chain_only_competitions,
                    &mut app.elf_only_competitions,
                    &mut app.ready_competitions,
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
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Length(3), // Tabs
            Constraint::Min(0),    // Main content
            Constraint::Length(8), // Log panel
            Constraint::Length(1), // Footer
        ])
        .split(area);

    // Header
    let header_title = format!(
        "ZinKNet Node | Block: {} | Peers: {}",
        app.block_number
            .map_or_else(|| "...".to_string(), |b| b.to_string()),
        app.connected_peers
    );
    let header = Paragraph::new(header_title).alignment(Alignment::Center);
    f.render_widget(header, chunks[0]);

    // Tabs
    let titles: Vec<_> = ["Competitions", "Processing", "My Info"]
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
            let header_cells = ["Competition ID", "Issuer", "Reward (ETH)", "Competitors"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().fg(Color::Red)));
            let header = Row::new(header_cells).height(1).bottom_margin(1);

            let rows: Vec<Row> = app
                .ready_competitions
                .values()
                .map(|competition| {
                    let reward_eth = format_ether(competition.reward);
                    let cells = vec![
                        Cell::from(competition.competition_id.to_string()),
                        Cell::from(competition.issuer.to_string()),
                        Cell::from(reward_eth),
                        Cell::from(competition.competitors.to_string()),
                    ];
                    Row::new(cells).height(1)
                })
                .collect();

            let widths = [
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ];
            let table = Table::new(rows, widths)
                .header(header)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Ready Competitions"),
                )
                .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            f.render_stateful_widget(table, main_chunk, &mut app.table_state);
        }
        1 => {
            let mut lines = Vec::new();
            if let Some(processing_competition) = &app.processing_competition {
                lines.push(Line::from("Processing Competition: "));
                lines.push(Line::from(vec![
                    Span::raw("  Competition ID: "),
                    Span::styled(
                        processing_competition.competition_id.to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  Issuer: "),
                    Span::styled(
                        processing_competition.issuer.to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  Reward: "),
                    Span::styled(
                        format!("{} ETH", format_ether(processing_competition.reward)),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
                if app.is_calculating {
                    lines.push(Line::from("  Status: Generating proof..."));
                } else {
                    lines.push(Line::from("  Status: Waiting for next step."));
                }
            } else if let Some(active_competition_id) = app.active_competition_id {
                lines.push(Line::from("Active Competition: "));
                lines.push(Line::from(vec![
                    Span::raw("  Competition ID: "),
                    Span::styled(
                        active_competition_id.to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
                lines.push(Line::from(
                    "(Another node may be processing this competition)",
                ));
            } else {
                lines.push(Line::from("No competition is currently being processed."));
            }

            let processing_paragraph = Paragraph::new(lines)
                .block(Block::default().borders(Borders::ALL).title("Processing"));
            f.render_widget(processing_paragraph, main_chunk);
        }
        2 => {
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
    let footer_text = if app.is_joining || app.is_leaving || app.is_calculating {
        if app.show_popup {
            "[c: Close Popup]".to_string()
        } else {
            "[v: View Progress] [q: Quit]".to_string()
        }
    } else if app.show_popup {
        "[Any key: Close Popup]".to_string()
    } else {
        let mut base = "[←/→: Switch Tab] [q: Quit]".to_string();
        if app.active_tab == 0 {
            base = "[←/→: Switch Tab] [↑/↓: Scroll] [q: Quit]".to_string();
            if app.active_competition_id.is_none() && app.table_state.selected().is_some() {
                base.push_str(" [j: Join Competition]");
            }
        } else if app.active_tab == 1 && app.processing_competition.is_some() && !app.is_calculating
        {
            base.push_str(" [l: Leave Competition]");
        }
        base
    };
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::LightCyan))
        .alignment(Alignment::Center);
    f.render_widget(footer, chunks[4]);

    // Popup
    if app.show_popup {
        render_popup(f, app.popup_title.as_str(), app.popup_content.as_str());
    }
}

fn render_popup(f: &mut Frame, title: &str, content: &str) {
    let area = f.area();
    let popup_area = centered_rect(60, 25, area);

    let popup_block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::DarkGray));

    let popup_text = Paragraph::new(content)
        .wrap(ratatui::widgets::Wrap { trim: true })
        .alignment(Alignment::Center)
        .block(popup_block);

    f.render_widget(Clear, popup_area);
    f.render_widget(popup_text, popup_area);
}

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
