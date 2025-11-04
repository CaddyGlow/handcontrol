mod app {
    use handcontrol_client_lib::{
        config::ClientConfig,
        storage::{ServerRegistry, ServerRegistryEntry},
        CommandList, CommandParameter, CommandParameterType, CommandSummary, DiscoveredServer,
    };
    use std::collections::{BTreeSet, HashMap};
    use uuid::Uuid;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FocusPane {
        Servers,
        Commands,
    }

    #[derive(Debug, Clone)]
    pub enum Overlay {
        Help,
        Search(SearchState),
        TagFilter(TagFilterState),
        ParameterForm(ParameterFormState),
        CommandExecution(CommandExecutionState),
        Enrollment(EnrollmentState),
        Prompt(PromptState),
    }

    #[derive(Debug, Clone, Default)]
    pub struct StatusLine {
        pub message: String,
    }

    #[derive(Debug, Clone)]
    pub struct App {
        pub config: ClientConfig,
        pub registry: ServerRegistry,
        pub discovered: Vec<DiscoveredServer>,
        pub servers: Vec<ServerItem>,
        pub selected_server: Option<usize>,
        pub focus: FocusPane,
        pub overlay: Option<Overlay>,
        pub status: StatusLine,
        pub command_state: CommandListState,
        pub discovery_in_progress: bool,
        pub enrollment_in_progress: bool,
        pub pending_command_server: Option<Uuid>,
        pub pending_escape: Option<EscapeSequenceState>,
    }

    impl App {
        pub fn new(config: ClientConfig, registry: ServerRegistry) -> Self {
            let mut app = Self {
                config,
                registry,
                discovered: Vec::new(),
                servers: Vec::new(),
                selected_server: None,
                focus: FocusPane::Servers,
                overlay: None,
                status: StatusLine::default(),
                command_state: CommandListState::default(),
                discovery_in_progress: false,
                enrollment_in_progress: false,
                pending_command_server: None,
                pending_escape: None,
            };
            app.rebuild_servers();
            app
        }

        pub fn rebuild_servers(&mut self) {
            let mut combined: Vec<ServerItem> = Vec::new();
            let mut seen: BTreeSet<Uuid> = BTreeSet::new();
            let mut discovered_by_id: HashMap<Uuid, DiscoveredServer> = HashMap::new();
            let mut discovered_orphans: Vec<DiscoveredServer> = Vec::new();

            for server in &self.discovered {
                if let Some(id) = server.server_id {
                    discovered_by_id.insert(id, server.clone());
                } else {
                    discovered_orphans.push(server.clone());
                }
            }

            let registry_entries: Vec<ServerRegistryEntry> =
                self.registry.iter().cloned().collect();

            for entry in registry_entries {
                let mut discovered = discovered_by_id.remove(&entry.id);

                if discovered.is_none() {
                    if let Some(hostname) = &entry.hostname {
                        if let Some(pos) = discovered_orphans
                            .iter()
                            .position(|d| &d.instance_name == hostname)
                        {
                            discovered = Some(discovered_orphans.remove(pos));
                        }
                    }
                }

                seen.insert(entry.id);

                combined.push(ServerItem::from_registry(entry, discovered));
            }

            for server in discovered_orphans.into_iter() {
                combined.push(ServerItem::from_discovered(server));
            }

            let mut discovered_without_id: Vec<ServerItem> = discovered_by_id
                .into_iter()
                .filter_map(|(id, server)| {
                    if seen.contains(&id) {
                        None
                    } else {
                        Some(ServerItem::from_discovered(server))
                    }
                })
                .collect();

            combined.append(&mut discovered_without_id);

            combined.sort_by(|a, b| {
                a.label
                    .to_ascii_lowercase()
                    .cmp(&b.label.to_ascii_lowercase())
            });

            self.servers = combined;
            if self.servers.is_empty() {
                self.selected_server = None;
            } else if let Some(selected) = self.selected_server {
                if selected >= self.servers.len() {
                    self.selected_server = Some(0);
                }
            } else {
                self.selected_server = Some(0);
            }
        }

        pub fn set_discovered(&mut self, servers: Vec<DiscoveredServer>) {
            self.discovered = servers;
            self.rebuild_servers();
        }

        pub fn clear_commands(&mut self) {
            self.command_state = CommandListState::default();
            self.pending_command_server = None;
        }

        pub fn set_commands(&mut self, server_id: Uuid, command_list: CommandList) {
            self.command_state.set_command_list(server_id, command_list);
            self.pending_command_server = None;
        }

        pub fn begin_command_load(&mut self, server_id: Uuid) {
            self.command_state.loading = true;
            self.command_state.error = None;
            self.pending_command_server = Some(server_id);
        }

        pub fn set_status_message<S: Into<String>>(&mut self, message: S) {
            self.status.message = message.into();
        }

        pub fn selected_server(&self) -> Option<&ServerItem> {
            self.selected_server.and_then(|idx| self.servers.get(idx))
        }

        pub fn selected_server_mut(&mut self) -> Option<&mut ServerItem> {
            if let Some(idx) = self.selected_server {
                self.servers.get_mut(idx)
            } else {
                None
            }
        }

        pub fn find_server_by_id(&self, id: &Uuid) -> Option<&ServerItem> {
            self.servers
                .iter()
                .find(|server| server.id.as_ref() == Some(id))
        }

        pub fn find_server_index(&self, id: &Uuid) -> Option<usize> {
            self.servers
                .iter()
                .position(|server| server.id.as_ref() == Some(id))
        }
    }

    #[derive(Debug, Clone)]
    pub struct ServerItem {
        pub id: Option<Uuid>,
        pub label: String,
        pub subtitle: Option<String>,
        pub status: ServerStatus,
        pub addresses: Vec<String>,
        pub port: Option<u16>,
        pub fingerprint: Option<String>,
        pub registry_entry: Option<ServerRegistryEntry>,
        pub discovered: Option<DiscoveredServer>,
    }

    impl ServerItem {
        fn from_registry(entry: ServerRegistryEntry, discovered: Option<DiscoveredServer>) -> Self {
            let mut addresses = discovered
                .as_ref()
                .map(|d| d.addresses.clone())
                .unwrap_or_else(Vec::new);
            if addresses.is_empty() {
                if let Some(ip) = &entry.ip {
                    addresses.push(ip.clone());
                }
            }

            let label = entry
                .hostname
                .clone()
                .or_else(|| discovered.as_ref().map(|d| d.instance_name.clone()))
                .unwrap_or_else(|| entry.id.to_string());

            let subtitle = if !addresses.is_empty() {
                Some(addresses[0].clone())
            } else {
                entry.ip.clone()
            };

            Self {
                id: Some(entry.id),
                label,
                subtitle,
                status: ServerStatus::Enrolled,
                addresses,
                port: entry.port,
                fingerprint: entry.cert_fingerprint.clone(),
                registry_entry: Some(entry),
                discovered,
            }
        }

        fn from_discovered(discovered: DiscoveredServer) -> Self {
            let label = discovered.instance_name.clone();
            let subtitle = discovered.addresses.get(0).cloned();

            Self {
                id: discovered.server_id,
                label,
                subtitle,
                status: ServerStatus::Available,
                addresses: discovered.addresses.clone(),
                port: Some(discovered.port),
                fingerprint: discovered.cert_fingerprint.clone(),
                registry_entry: None,
                discovered: Some(discovered),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ServerStatus {
        Enrolled,
        Available,
        Unknown,
    }

    #[derive(Debug, Clone, Default)]
    pub struct CommandListState {
        pub commands: Vec<CommandSummary>,
        pub filtered_indices: Vec<usize>,
        pub selected: Option<usize>,
        pub search_query: String,
        pub tag_filter: Option<String>,
        pub config_version: Option<u64>,
        pub loading: bool,
        pub error: Option<String>,
        pub server_id: Option<Uuid>,
    }

    impl CommandListState {
        pub fn set_command_list(&mut self, server_id: Uuid, list: CommandList) {
            self.commands = list.commands;
            self.config_version = Some(list.config_version);
            self.filtered_indices = (0..self.commands.len()).collect();
            self.selected = if self.commands.is_empty() {
                None
            } else {
                Some(0)
            };
            self.loading = false;
            self.error = None;
            self.server_id = Some(server_id);
            self.reapply_filters();
        }

        pub fn reapply_filters(&mut self) {
            let query = self.search_query.to_ascii_lowercase();
            let tag = self.tag_filter.clone();
            self.filtered_indices = self
                .commands
                .iter()
                .enumerate()
                .filter(|(_, cmd)| {
                    let matches_query = if query.is_empty() {
                        true
                    } else {
                        cmd.name.to_ascii_lowercase().contains(&query)
                            || cmd
                                .description
                                .as_ref()
                                .map(|d| d.to_ascii_lowercase().contains(&query))
                                .unwrap_or(false)
                            || cmd
                                .tags
                                .iter()
                                .any(|tag| tag.to_ascii_lowercase().contains(&query))
                    };

                    let matches_tag = if let Some(tag_filter) = &tag {
                        cmd.tags.iter().any(|t| t == tag_filter)
                    } else {
                        true
                    };

                    matches_query && matches_tag
                })
                .map(|(idx, _)| idx)
                .collect();

            if let Some(selected) = self.selected {
                if !self.filtered_indices.contains(&selected) {
                    self.selected = self.filtered_indices.first().copied();
                }
            } else {
                self.selected = self.filtered_indices.first().copied();
            }
        }

        pub fn selected_command(&self) -> Option<&CommandSummary> {
            self.selected
                .and_then(|idx| self.commands.get(idx))
                .or_else(|| {
                    self.filtered_indices
                        .first()
                        .and_then(|idx| self.commands.get(*idx))
                })
        }

        pub fn available_tags(&self) -> Vec<String> {
            let mut tags: BTreeSet<String> = BTreeSet::new();
            for cmd in &self.commands {
                for tag in &cmd.tags {
                    tags.insert(tag.clone());
                }
            }
            tags.into_iter().collect()
        }

        pub fn filtered_iter(&self) -> impl Iterator<Item = (usize, &CommandSummary)> {
            self.filtered_indices
                .iter()
                .filter_map(|idx| self.commands.get(*idx).map(|cmd| (*idx, cmd)))
        }

        pub fn select_next(&mut self) {
            if self.filtered_indices.is_empty() {
                self.selected = None;
                return;
            }
            if self.selected.is_none() {
                self.selected = Some(self.filtered_indices[0]);
                tracing::debug!(
                    ?self.selected,
                    "initialized command selection (next)"
                );
                return;
            }

            let current = self.selected.unwrap();
            let position = self
                .filtered_indices
                .iter()
                .position(|idx| *idx == current)
                .unwrap_or(0);
            let next = (position + 1) % self.filtered_indices.len();
            self.selected = Some(self.filtered_indices[next]);
            tracing::debug!(?self.selected, "advanced command selection");
        }

        pub fn select_previous(&mut self) {
            if self.filtered_indices.is_empty() {
                self.selected = None;
                return;
            }
            if self.selected.is_none() {
                self.selected = self.filtered_indices.last().copied();
                tracing::debug!(
                    ?self.selected,
                    "initialized command selection (previous)"
                );
                return;
            }

            let current = self.selected.unwrap();
            let position = self
                .filtered_indices
                .iter()
                .position(|idx| *idx == current)
                .unwrap_or(0);
            let prev = if position == 0 {
                self.filtered_indices.len() - 1
            } else {
                position - 1
            };
            self.selected = Some(self.filtered_indices[prev]);
            tracing::debug!(?self.selected, "moved command selection backwards");
        }

        pub fn set_search_query(&mut self, query: String) {
            self.search_query = query;
            self.reapply_filters();
        }

        pub fn clear_search(&mut self) {
            self.search_query.clear();
            self.reapply_filters();
        }

        pub fn set_tag_filter(&mut self, tag: Option<String>) {
            self.tag_filter = tag;
            self.reapply_filters();
        }

        pub fn filtered_len(&self) -> usize {
            self.filtered_indices.len()
        }
    }

    #[derive(Debug, Clone)]
    pub struct ParameterFormState {
        pub command: CommandSummary,
        pub fields: Vec<ParameterFieldState>,
        pub selected: usize,
        pub editing: bool,
        pub error: Option<String>,
        pub server_id: Uuid,
        pub server_label: String,
        pub text_cursor: Option<usize>,
    }

    impl ParameterFormState {
        pub fn new(command: CommandSummary, server_id: Uuid, server_label: String) -> Self {
            let fields = command
                .parameters
                .iter()
                .map(ParameterFieldState::from_parameter)
                .collect();

            Self {
                command,
                fields,
                selected: 0,
                editing: false,
                error: None,
                server_id,
                server_label,
                text_cursor: None,
            }
        }

        pub fn parameter_values(&self) -> HashMap<String, String> {
            let mut values = HashMap::new();
            for field in &self.fields {
                values.insert(field.definition.name.clone(), field.value_as_string());
            }
            values
        }
    }

    #[derive(Debug, Clone)]
    pub struct ParameterFieldState {
        pub definition: CommandParameter,
        pub value: ParameterValue,
        pub dirty: bool,
    }

    impl ParameterFieldState {
        fn from_parameter(param: &CommandParameter) -> Self {
            let value = match param.param_type {
                CommandParameterType::Slider => {
                    let default = param
                        .default_value
                        .as_ref()
                        .and_then(|v| v.parse::<i32>().ok())
                        .or(param.min)
                        .unwrap_or(0);
                    ParameterValue::Slider {
                        value: default,
                        min: param.min,
                        max: param.max,
                        step: 1,
                    }
                }
                CommandParameterType::Text => ParameterValue::Text(
                    param.default_value.clone().unwrap_or_else(|| String::new()),
                ),
                CommandParameterType::Toggle => {
                    let default = param
                        .default_value
                        .as_deref()
                        .map(|v| v.eq_ignore_ascii_case("true"))
                        .unwrap_or(false);
                    ParameterValue::Toggle { value: default }
                }
                CommandParameterType::Dropdown => {
                    let selected = param
                        .default_value
                        .as_ref()
                        .and_then(|default| {
                            param.options.iter().position(|option| option == default)
                        })
                        .unwrap_or(0);
                    ParameterValue::Dropdown {
                        options: param.options.clone(),
                        selected,
                    }
                }
            };

            Self {
                definition: param.clone(),
                value,
                dirty: false,
            }
        }

        pub fn value_as_string(&self) -> String {
            match &self.value {
                ParameterValue::Slider { value, .. } => value.to_string(),
                ParameterValue::Text(text) => text.clone(),
                ParameterValue::Toggle { value } => {
                    if *value {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                ParameterValue::Dropdown { options, selected } => options
                    .get(*selected)
                    .cloned()
                    .unwrap_or_else(|| "".to_string()),
            }
        }
    }

    #[derive(Debug, Clone)]
    pub enum ParameterValue {
        Slider {
            value: i32,
            min: Option<i32>,
            max: Option<i32>,
            step: i32,
        },
        Text(String),
        Toggle {
            value: bool,
        },
        Dropdown {
            options: Vec<String>,
            selected: usize,
        },
    }

    #[derive(Debug, Clone)]
    pub struct CommandExecutionState {
        pub command_name: String,
        pub server_label: String,
        pub server_id: Uuid,
        pub command_id: String,
        pub output: Vec<CommandOutputLine>,
        pub scroll_offset: usize,
        pub auto_scroll: bool,
        pub running: bool,
        pub exit_code: Option<i32>,
        pub error: Option<String>,
        pub paused: bool,
    }

    impl CommandExecutionState {
        pub fn new(
            command_name: String,
            command_id: String,
            server_label: String,
            server_id: Uuid,
            auto_scroll: bool,
        ) -> Self {
            Self {
                command_name,
                command_id,
                server_label,
                server_id,
                output: Vec::new(),
                scroll_offset: 0,
                auto_scroll,
                running: true,
                exit_code: None,
                error: None,
                paused: false,
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct CommandOutputLine {
        pub channel: OutputChannel,
        pub content: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum OutputChannel {
        Stdout,
        Stderr,
        Status,
    }

    #[derive(Debug, Clone)]
    pub struct SearchState {
        pub query: String,
        pub cursor: usize,
    }

    #[derive(Debug, Clone)]
    pub struct TagFilterState {
        pub tags: Vec<String>,
        pub selected: usize,
    }

    #[derive(Debug, Clone)]
    pub struct PromptState {
        pub title: String,
        pub message: String,
        pub confirm_label: String,
        pub cancel_label: String,
        pub action: PromptAction,
    }

    #[derive(Debug, Clone)]
    pub struct EnrollmentState {
        pub server: ServerItem,
        pub step: EnrollmentStep,
    }

    #[derive(Debug, Clone)]
    pub enum PromptAction {
        None,
        RemoveServer { server_id: Uuid },
    }

    #[derive(Debug, Clone)]
    pub enum EnrollmentStep {
        MethodSelect,
        QrInput(QrInputState),
        Approval(ApprovalState),
        InProgress { message: String },
        Completed { message: String },
        Error { message: String },
    }

    #[derive(Debug, Clone)]
    pub struct QrInputState {
        pub payload: String,
        pub cursor: usize,
    }

    #[derive(Debug, Clone)]
    pub struct ApprovalState {
        pub verification_code: Option<String>,
        pub status_message: String,
        pub started: bool,
    }

    #[derive(Debug, Clone)]
    pub struct EscapeSequenceState {
        pub collected: Vec<char>,
    }
}

use anyhow::{anyhow, Context, Result};
use app::{App, FocusPane, OutputChannel, ParameterValue, ServerStatus};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event as CrosstermEvent, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use handcontrol_client_lib::{
    config, config::DiscoveryConfig, discover_servers, enroll_via_approval, execute_command,
    list_commands, validate_parameters, ApprovalEnrollmentInput, CommandList, CommandParameterType,
    CommandStreamEvent, CommandSummary, DiscoveredServer, ServerRegistry, ServerRegistryEntry,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::{collections::HashMap, fs, io, time::Duration};
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

type EventSender = mpsc::UnboundedSender<UiEvent>;
type EventReceiver = mpsc::UnboundedReceiver<UiEvent>;

#[derive(Debug)]
enum UiEvent {
    Input(CrosstermEvent),
    Tick,
    Task(TaskEvent),
}

#[derive(Debug)]
enum TaskEvent {
    DiscoveryFinished(Result<Vec<DiscoveredServer>, anyhow::Error>),
    CommandListLoaded {
        server_id: Uuid,
        result: Result<CommandList, anyhow::Error>,
    },
    CommandExecution(CommandExecutionUpdate),
    Enrollment(EnrollmentTaskEvent),
}

#[derive(Debug)]
enum CommandExecutionUpdate {
    Started {
        server_id: Uuid,
        command_id: String,
    },
    Stream {
        server_id: Uuid,
        command_id: String,
        event: CommandStreamEvent,
    },
    Finished {
        server_id: Uuid,
        command_id: String,
        result: Result<i32, anyhow::Error>,
    },
}

#[derive(Debug)]
enum EnrollmentTaskEvent {
    Started { server_id: Option<Uuid> },
    VerificationCode { code: String },
    Progress { message: String },
    Completed { message: String },
    Failed { error: anyhow::Error },
}

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        execute!(
            stdout,
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
            )
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if let Err(err) = disable_raw_mode() {
            tracing::warn!("Failed to disable raw mode: {err}");
        }
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            PopKeyboardEnhancementFlags,
            DisableFocusChange,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen,
            Show
        );
    }
}

fn setup_terminal() -> Result<(TerminalGuard, Terminal<CrosstermBackend<io::Stdout>>)> {
    let guard = TerminalGuard::new()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok((guard, terminal))
}

#[tokio::main]
async fn main() -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .try_init();

    let config = config::load().context("Failed to load client configuration")?;
    let registry = ServerRegistry::load().context("Failed to load server registry")?;
    let mut app = App::new(config, registry);

    let (guard, mut terminal) = setup_terminal()?;
    let _guard = guard;

    let (event_tx, mut event_rx): (EventSender, EventReceiver) = mpsc::unbounded_channel();

    spawn_input_thread(event_tx.clone());
    spawn_tick_task(event_tx.clone(), Duration::from_millis(100));

    if app.config.discovery.auto_discover {
        trigger_discovery(&mut app, &event_tx);
    } else {
        app.set_status_message("Ready");
    }

    let mut should_quit = false;
    while !should_quit {
        draw(&mut terminal, &app)?;

        if let Some(event) = event_rx.recv().await {
            match event {
                UiEvent::Tick => {
                    // reserved for animations
                }
                UiEvent::Input(event) => {
                    should_quit = handle_input_event(event, &mut app, &event_tx)?;
                }
                UiEvent::Task(task_event) => {
                    handle_task_event(task_event, &mut app)?;
                }
            }
        } else {
            break;
        }
    }

    Ok(())
}

fn spawn_input_thread(event_tx: EventSender) {
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            tracing::debug!(?event, "crossterm event read");
            if event_tx.send(UiEvent::Input(event)).is_err() {
                break;
            }
        }
        tracing::debug!("input thread exiting");
    });
}

fn spawn_tick_task(event_tx: EventSender, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if event_tx.send(UiEvent::Tick).is_err() {
                break;
            }
        }
    });
}

fn trigger_discovery(app: &mut App, event_tx: &EventSender) {
    if app.discovery_in_progress {
        return;
    }
    app.discovery_in_progress = true;
    app.set_status_message("Discovering servers...");
    let task_tx = event_tx.clone();
    spawn_discovery(task_tx, app.config.discovery.clone());
}

fn trigger_enrollment(app: &mut App, event_tx: &EventSender) -> Result<()> {
    if app.enrollment_in_progress {
        app.set_status_message("Enrollment already in progress");
        return Ok(());
    }

    let server = match app.selected_server().cloned() {
        Some(server) => server,
        None => {
            app.set_status_message("Select a server to enroll");
            return Ok(());
        }
    };

    if server.registry_entry.is_some() {
        app.set_status_message("Server is already enrolled");
        return Ok(());
    }

    if server.addresses.is_empty() {
        app.set_status_message("Selected server does not have any reachable addresses");
        return Ok(());
    }

    let addresses = server.addresses.clone();
    let port = server.port;
    let server_id_hint = server.id;
    let label = server.label.clone();

    let overlay_state = app::EnrollmentState {
        server: server.clone(),
        step: app::EnrollmentStep::Approval(app::ApprovalState {
            verification_code: None,
            status_message: "Requesting enrollment...".to_string(),
            started: false,
        }),
    };
    app.overlay = Some(app::Overlay::Enrollment(overlay_state));

    let device_name = app
        .config
        .device
        .as_ref()
        .and_then(|device| device.name.clone());
    let device_model = app
        .config
        .device
        .as_ref()
        .and_then(|device| device.model.clone());

    let timeout = Duration::from_secs(60);
    let poll_interval = Duration::from_secs(2);

    app.enrollment_in_progress = true;
    app.set_status_message(format!("Requesting enrollment for {}", label));

    let tx = event_tx.clone();
    tokio::spawn(async move {
        let start_event = EnrollmentTaskEvent::Started {
            server_id: server_id_hint,
        };
        let _ = tx.send(UiEvent::Task(TaskEvent::Enrollment(start_event)));

        let wait_event = EnrollmentTaskEvent::Progress {
            message: format!("Waiting for approval from {}", label),
        };
        let _ = tx.send(UiEvent::Task(TaskEvent::Enrollment(wait_event)));

        let input = ApprovalEnrollmentInput {
            addresses,
            port,
            server_id_hint,
            device_name,
            device_model,
            timeout,
            poll_interval,
        };

        let result = enroll_via_approval(input, |code| {
            let event = EnrollmentTaskEvent::VerificationCode {
                code: code.to_string(),
            };
            let _ = tx.send(UiEvent::Task(TaskEvent::Enrollment(event)));
        })
        .await;

        match result {
            Ok(outcome) => {
                let message = format!(
                    "Enrollment complete for {} (server {})",
                    label, outcome.server_id
                );
                let event = EnrollmentTaskEvent::Completed { message };
                let _ = tx.send(UiEvent::Task(TaskEvent::Enrollment(event)));
            }
            Err(error) => {
                let event = EnrollmentTaskEvent::Failed { error };
                let _ = tx.send(UiEvent::Task(TaskEvent::Enrollment(event)));
            }
        }
    });

    Ok(())
}

fn spawn_discovery(event_tx: EventSender, discovery_config: DiscoveryConfig) {
    tokio::spawn(async move {
        let outcome = tokio::task::spawn_blocking(move || discover_servers(&discovery_config))
            .await
            .map_err(|err| anyhow!(err))
            .and_then(|res| res);

        let _ = event_tx.send(UiEvent::Task(TaskEvent::DiscoveryFinished(outcome)));
    });
}

fn spawn_load_commands(event_tx: EventSender, entry: ServerRegistryEntry) {
    tokio::spawn(async move {
        let server_id = entry.id;
        let result = list_commands(&entry).await;
        let _ = event_tx.send(UiEvent::Task(TaskEvent::CommandListLoaded {
            server_id,
            result,
        }));
    });
}

fn spawn_execute_command(
    event_tx: EventSender,
    entry: ServerRegistryEntry,
    command_id: String,
    parameters: HashMap<String, String>,
) {
    tokio::spawn(async move {
        let server_id = entry.id;
        let start_event = CommandExecutionUpdate::Started {
            server_id,
            command_id: command_id.clone(),
        };
        let _ = event_tx.send(UiEvent::Task(TaskEvent::CommandExecution(start_event)));

        let command_id_for_stream = command_id.clone();
        let sender = event_tx.clone();
        let result = execute_command(&entry, &command_id, parameters, move |event| {
            let update = CommandExecutionUpdate::Stream {
                server_id,
                command_id: command_id_for_stream.clone(),
                event,
            };
            let _ = sender.send(UiEvent::Task(TaskEvent::CommandExecution(update)));
        })
        .await;

        let finish = CommandExecutionUpdate::Finished {
            server_id,
            command_id,
            result,
        };
        let _ = event_tx.send(UiEvent::Task(TaskEvent::CommandExecution(finish)));
    });
}

fn handle_input_event(
    event: CrosstermEvent,
    app: &mut App,
    event_tx: &EventSender,
) -> Result<bool> {
    match event {
        CrosstermEvent::Key(key) => {
            let mut should_quit = false;
            let expanded_keys = expand_key_event(key, app);
            tracing::debug!(keys = ?expanded_keys, "expanded keys");
            if expanded_keys.is_empty() {
                return Ok(false);
            }
            for key_event in expanded_keys {
                if handle_key_event(key_event, app, event_tx)? {
                    should_quit = true;
                    break;
                }
            }
            Ok(should_quit)
        }
        CrosstermEvent::Resize(_, _) => Ok(false),
        _ => Ok(false),
    }
}

fn expand_key_event(key: KeyEvent, app: &mut App) -> Vec<KeyEvent> {
    tracing::debug!(?key, "expand_key_event input");

    let is_plain_press =
        matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) && key.modifiers.is_empty();

    if is_plain_press && matches!(key.code, KeyCode::Esc) {
        tracing::debug!("start escape sequence");
        app.pending_escape = Some(app::EscapeSequenceState {
            collected: Vec::new(),
        });
        return Vec::new();
    }

    if let Some(state) = app.pending_escape.as_mut() {
        if is_plain_press {
            match key.code {
                KeyCode::Char(c) if state.collected.is_empty() && (c == '[' || c == 'O') => {
                    tracing::debug!("escape sequence received prefix {}", c);
                    state.collected.push(c);
                    return Vec::new();
                }
                KeyCode::Char(c) if !state.collected.is_empty() => {
                    state.collected.push(c);
                    if matches!(c, 'A' | 'B' | 'C' | 'D') {
                        let code = match c {
                            'A' => KeyCode::Up,
                            'B' => KeyCode::Down,
                            'C' => KeyCode::Right,
                            'D' => KeyCode::Left,
                            _ => unreachable!(),
                        };
                        tracing::debug!(sequence = ?state.collected, ?code, "escape sequence decoded to arrow");
                        app.pending_escape = None;
                        return vec![KeyEvent::new(code, KeyModifiers::NONE)];
                    } else {
                        tracing::debug!(sequence = ?state.collected, "escape sequence accumulating");
                        return Vec::new();
                    }
                }
                _ => {}
            }
        }

        tracing::debug!("escape sequence fallback, emitting raw events");
        app.pending_escape = None;
        return vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), key];
    }

    vec![key]
}

fn handle_key_event(key: KeyEvent, app: &mut App, event_tx: &EventSender) -> Result<bool> {
    tracing::debug!(?key, "received key event");
    if matches!(key.kind, KeyEventKind::Release) {
        tracing::debug!(?key, "ignoring key release");
        return Ok(false);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        tracing::debug!("Ctrl+C pressed; exiting");
        return Ok(true);
    }

    if let Some(overlay) = app.overlay.take() {
        tracing::debug!("delegating key to overlay");
        let result = handle_overlay_value(key, app, overlay, event_tx)?;
        match result {
            OverlayHandlerResult::Continue(next) => app.overlay = Some(next),
            OverlayHandlerResult::Close => {}
            OverlayHandlerResult::Quit => return Ok(true),
        }
        return Ok(false);
    }

    let normalized = if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        }
    } else {
        key.code
    };
    tracing::debug!(
        ?normalized,
        modifiers = ?key.modifiers,
        "normalized key for handling"
    );

    match normalized {
        KeyCode::Char('q') => Ok(true),
        KeyCode::Char('d') => {
            trigger_discovery(app, event_tx);
            Ok(false)
        }
        KeyCode::Char('e') => {
            if let Err(err) = trigger_enrollment(app, event_tx) {
                app.set_status_message(format!("Unable to start enrollment: {err}"));
            }
            Ok(false)
        }
        KeyCode::Char('r') => {
            if let Err(err) = open_remove_prompt(app) {
                app.set_status_message(format!("Unable to prepare removal prompt: {err}"));
            }
            Ok(false)
        }
        KeyCode::Tab | KeyCode::BackTab => {
            toggle_focus(app);
            Ok(false)
        }
        KeyCode::Right => {
            if !app.command_state.commands.is_empty() {
                app.focus = FocusPane::Commands;
            }
            Ok(false)
        }
        KeyCode::Left => {
            app.focus = FocusPane::Servers;
            Ok(false)
        }
        KeyCode::Enter => {
            match app.focus {
                FocusPane::Servers => {
                    if let Some(server) = app.selected_server().cloned() {
                        if let Some(entry) = server.registry_entry.clone() {
                            if app.command_state.loading {
                                app.set_status_message("Already fetching commands...");
                                return Ok(false);
                            }
                            if let Some(server_id) = server.id {
                                app.begin_command_load(server_id);
                                app.set_status_message(format!(
                                    "Fetching commands for {}",
                                    server.label
                                ));
                                let tx = event_tx.clone();
                                spawn_load_commands(tx, entry);
                            } else {
                                app.set_status_message(
                                    "Server has unknown ID; unable to fetch commands",
                                );
                            }
                        } else {
                            app.set_status_message(
                                "Server not enrolled; enroll before listing commands",
                            );
                        }
                    }
                }
                FocusPane::Commands => {
                    if let Err(err) = trigger_command_activation(app, event_tx) {
                        app.set_status_message(format!("Failed to start command: {err}"));
                    }
                }
            }
            Ok(false)
        }
        KeyCode::Up => {
            match app.focus {
                FocusPane::Servers => move_selection_up(app),
                FocusPane::Commands => move_command_selection_up(app),
            }
            Ok(false)
        }
        KeyCode::Down => {
            match app.focus {
                FocusPane::Servers => move_selection_down(app),
                FocusPane::Commands => move_command_selection_down(app),
            }
            Ok(false)
        }
        KeyCode::Char('k') | KeyCode::Char('K') => {
            match app.focus {
                FocusPane::Servers => move_selection_up(app),
                FocusPane::Commands => move_command_selection_up(app),
            }
            Ok(false)
        }
        KeyCode::Char('j') | KeyCode::Char('J') => {
            match app.focus {
                FocusPane::Servers => move_selection_down(app),
                FocusPane::Commands => move_command_selection_down(app),
            }
            Ok(false)
        }
        KeyCode::Char('/') => {
            if app.command_state.commands.is_empty() {
                app.set_status_message("Load commands before searching");
            } else {
                let query = app.command_state.search_query.clone();
                let cursor = query.chars().count();
                app.overlay = Some(app::Overlay::Search(app::SearchState { query, cursor }));
            }
            Ok(false)
        }
        KeyCode::Char('t') => {
            if let Err(err) = open_tag_filter_overlay(app) {
                app.set_status_message(format!("Unable to open tag filter: {err}"));
            }
            Ok(false)
        }
        KeyCode::Char('x') => {
            if let Err(err) = trigger_command_activation(app, event_tx) {
                app.set_status_message(format!("Failed to start command: {err}"));
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn handle_task_event(event: TaskEvent, app: &mut App) -> Result<()> {
    match event {
        TaskEvent::DiscoveryFinished(result) => {
            app.discovery_in_progress = false;
            match result {
                Ok(servers) => {
                    let count = servers.len();
                    app.set_discovered(servers);
                    app.set_status_message(format!("Discovery complete ({count} servers)"));
                }
                Err(err) => {
                    app.set_status_message(format!("Discovery failed: {}", err));
                }
            }
        }
        TaskEvent::CommandListLoaded { server_id, result } => {
            if let Some(pending) = app.pending_command_server {
                if pending != server_id {
                    tracing::debug!(
                        ?server_id,
                        ?pending,
                        "Ignoring command list response for stale request"
                    );
                    return Ok(());
                }
            }
            match result {
                Ok(list) => {
                    app.set_commands(server_id, list);
                    if let Some(idx) = app.find_server_index(&server_id) {
                        app.selected_server = Some(idx);
                    }
                    if app.focus == FocusPane::Servers {
                        app.focus = FocusPane::Commands;
                    }
                    let label = app
                        .find_server_by_id(&server_id)
                        .map(|srv| srv.label.clone())
                        .unwrap_or_else(|| server_id.to_string());
                    app.set_status_message(format!("Loaded commands for {}", label));
                }
                Err(err) => {
                    app.command_state.loading = false;
                    app.command_state.error = Some(err.to_string());
                    app.set_status_message(format!("Failed to load commands: {}", err));
                    app.pending_command_server = None;
                }
            }
        }
        TaskEvent::CommandExecution(update) => match update {
            CommandExecutionUpdate::Started {
                server_id,
                command_id,
            } => {
                if let Some(app::Overlay::CommandExecution(state)) = app.overlay.as_mut() {
                    if state.server_id == server_id && state.command_id == command_id {
                        state.running = true;
                        state.exit_code = None;
                        state.error = None;
                        state.output.clear();
                        state.scroll_offset = 0;
                    }
                }
                app.set_status_message("Command execution started");
            }
            CommandExecutionUpdate::Stream {
                server_id,
                command_id,
                event,
            } => {
                if let Some(app::Overlay::CommandExecution(state)) = app.overlay.as_mut() {
                    if state.server_id == server_id && state.command_id == command_id {
                        let (channel, content) = match event {
                            CommandStreamEvent::Stdout(line) => (app::OutputChannel::Stdout, line),
                            CommandStreamEvent::Stderr(line) => (app::OutputChannel::Stderr, line),
                        };
                        state
                            .output
                            .push(app::CommandOutputLine { channel, content });
                        if state.auto_scroll {
                            state.scroll_offset = 0;
                        }
                    }
                }
            }
            CommandExecutionUpdate::Finished {
                server_id,
                command_id,
                result,
            } => {
                if let Some(app::Overlay::CommandExecution(state)) = app.overlay.as_mut() {
                    if state.server_id == server_id && state.command_id == command_id {
                        state.running = false;
                        match result {
                            Ok(code) => {
                                state.exit_code = Some(code);
                                state.output.push(app::CommandOutputLine {
                                    channel: app::OutputChannel::Status,
                                    content: format!("Command completed with exit code {code}"),
                                });
                                app.set_status_message(format!(
                                    "Command completed with exit code {code}"
                                ));
                            }
                            Err(err) => {
                                state.error = Some(err.to_string());
                                state.output.push(app::CommandOutputLine {
                                    channel: app::OutputChannel::Status,
                                    content: format!("Command failed: {}", err),
                                });
                                app.set_status_message(format!("Command failed: {}", err));
                            }
                        }
                    }
                }
            }
        },
        TaskEvent::Enrollment(event) => match event {
            EnrollmentTaskEvent::Started { .. } => {
                if let Some(app::Overlay::Enrollment(state)) = app.overlay.as_mut() {
                    match &mut state.step {
                        app::EnrollmentStep::Approval(approval) => {
                            approval.status_message =
                                "Enrollment request sent; waiting for verification code"
                                    .to_string();
                            approval.started = true;
                        }
                        _ => {
                            state.step = app::EnrollmentStep::Approval(app::ApprovalState {
                                verification_code: None,
                                status_message:
                                    "Enrollment request sent; waiting for verification code"
                                        .to_string(),
                                started: true,
                            });
                        }
                    }
                }
                app.set_status_message("Enrollment request sent; waiting for verification code");
            }
            EnrollmentTaskEvent::VerificationCode { code } => {
                if let Some(app::Overlay::Enrollment(state)) = app.overlay.as_mut() {
                    match &mut state.step {
                        app::EnrollmentStep::Approval(approval) => {
                            approval.verification_code = Some(code.clone());
                            approval.status_message =
                                "Approve this request on the server to continue".to_string();
                        }
                        _ => {
                            state.step = app::EnrollmentStep::Approval(app::ApprovalState {
                                verification_code: Some(code.clone()),
                                status_message: "Approve this request on the server to continue"
                                    .to_string(),
                                started: true,
                            });
                        }
                    }
                }
                app.set_status_message(format!(
                    "Enter verification code {} on the server to approve enrollment",
                    code
                ));
            }
            EnrollmentTaskEvent::Progress { message } => {
                if let Some(app::Overlay::Enrollment(state)) = app.overlay.as_mut() {
                    match &mut state.step {
                        app::EnrollmentStep::Approval(approval) => {
                            approval.status_message = message.clone();
                        }
                        _ => {
                            state.step = app::EnrollmentStep::InProgress {
                                message: message.clone(),
                            };
                        }
                    }
                }
                app.set_status_message(message);
            }
            EnrollmentTaskEvent::Completed { message } => {
                app.enrollment_in_progress = false;
                if let Some(app::Overlay::Enrollment(state)) = app.overlay.as_mut() {
                    state.step = app::EnrollmentStep::Completed {
                        message: message.clone(),
                    };
                }
                match ServerRegistry::load() {
                    Ok(registry) => {
                        app.registry = registry;
                        app.rebuild_servers();
                        app.set_status_message(message);
                    }
                    Err(err) => {
                        tracing::error!(
                            "Enrollment succeeded but reloading registry failed: {}",
                            err
                        );
                        app.set_status_message(format!(
                            "{} (registry reload failed: {})",
                            message, err
                        ));
                    }
                }
            }
            EnrollmentTaskEvent::Failed { error } => {
                app.enrollment_in_progress = false;
                let error_text = error.to_string();
                if let Some(app::Overlay::Enrollment(state)) = app.overlay.as_mut() {
                    state.step = app::EnrollmentStep::Error {
                        message: error_text.clone(),
                    };
                }
                app.set_status_message(format!("Enrollment failed: {}", error_text));
            }
        },
    }
    Ok(())
}

enum OverlayHandlerResult {
    Continue(app::Overlay),
    Close,
    Quit,
}

fn handle_overlay_value(
    key: KeyEvent,
    app: &mut App,
    overlay: app::Overlay,
    event_tx: &EventSender,
) -> Result<OverlayHandlerResult> {
    match overlay {
        app::Overlay::Search(mut state) => match handle_search_overlay_key(key, app, &mut state)? {
            SearchOverlayAction::Continue => {
                Ok(OverlayHandlerResult::Continue(app::Overlay::Search(state)))
            }
            SearchOverlayAction::Close => Ok(OverlayHandlerResult::Close),
        },
        app::Overlay::TagFilter(state) => handle_tag_overlay_event(key, app, state),
        app::Overlay::ParameterForm(state) => {
            handle_parameter_overlay_event(key, app, state, event_tx)
        }
        app::Overlay::CommandExecution(mut state) => {
            match handle_execution_overlay_key(key, app, &mut state)? {
                ExecutionOverlayAction::Close => Ok(OverlayHandlerResult::Close),
                ExecutionOverlayAction::Continue => Ok(OverlayHandlerResult::Continue(
                    app::Overlay::CommandExecution(state),
                )),
            }
        }
        app::Overlay::Enrollment(state) => handle_enrollment_overlay_event(key, app, state),
        app::Overlay::Prompt(state) => handle_prompt_overlay_event(key, app, state),
        app::Overlay::Help => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                Ok(OverlayHandlerResult::Close)
            } else {
                Ok(OverlayHandlerResult::Continue(app::Overlay::Help))
            }
        }
    }
}

enum SearchOverlayAction {
    Continue,
    Close,
}

fn handle_search_overlay_key(
    key: KeyEvent,
    app: &mut App,
    state: &mut app::SearchState,
) -> Result<SearchOverlayAction> {
    let len = state.query.chars().count();
    let normalized = if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        }
    } else {
        key.code
    };

    match normalized {
        KeyCode::Esc => Ok(SearchOverlayAction::Close),
        KeyCode::Enter => {
            app.command_state.set_search_query(state.query.clone());
            if let Some(first) = app.command_state.filtered_indices.first().copied() {
                app.command_state.selected = Some(first);
            }
            Ok(SearchOverlayAction::Close)
        }
        KeyCode::Backspace => {
            if state.cursor > 0 {
                let remove_index = state.cursor - 1;
                if remove_char_at(&mut state.query, remove_index) {
                    state.cursor = remove_index;
                }
            }
            Ok(SearchOverlayAction::Continue)
        }
        KeyCode::Delete => {
            if state.cursor < len {
                remove_char_at(&mut state.query, state.cursor);
            }
            Ok(SearchOverlayAction::Continue)
        }
        KeyCode::Left => {
            if state.cursor > 0 {
                state.cursor -= 1;
            }
            Ok(SearchOverlayAction::Continue)
        }
        KeyCode::Right => {
            if state.cursor < len {
                state.cursor += 1;
            }
            Ok(SearchOverlayAction::Continue)
        }
        KeyCode::Home => {
            state.cursor = 0;
            Ok(SearchOverlayAction::Continue)
        }
        KeyCode::End => {
            state.cursor = len;
            Ok(SearchOverlayAction::Continue)
        }
        KeyCode::Char(c) => {
            if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                return Ok(SearchOverlayAction::Continue);
            }
            insert_char_at(&mut state.query, state.cursor, c);
            state.cursor += 1;
            Ok(SearchOverlayAction::Continue)
        }
        KeyCode::Tab => Ok(SearchOverlayAction::Continue),
        _ => Ok(SearchOverlayAction::Continue),
    }
}

enum ExecutionOverlayAction {
    Continue,
    Close,
}

fn handle_execution_overlay_key(
    key: KeyEvent,
    app: &mut App,
    state: &mut app::CommandExecutionState,
) -> Result<ExecutionOverlayAction> {
    let normalized = if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        }
    } else {
        key.code
    };

    match normalized {
        KeyCode::Esc => {
            if !state.running {
                Ok(ExecutionOverlayAction::Close)
            } else {
                app.set_status_message("Command still running; cannot close yet");
                Ok(ExecutionOverlayAction::Continue)
            }
        }
        _ => Ok(ExecutionOverlayAction::Continue),
    }
}

fn handle_enrollment_overlay_event(
    key: KeyEvent,
    _app: &mut App,
    state: app::EnrollmentState,
) -> Result<OverlayHandlerResult> {
    let normalized = if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        }
    } else {
        key.code
    };

    match normalized {
        KeyCode::Esc => Ok(OverlayHandlerResult::Close),
        KeyCode::Enter => {
            if matches!(
                state.step,
                app::EnrollmentStep::Completed { .. } | app::EnrollmentStep::Error { .. }
            ) {
                Ok(OverlayHandlerResult::Close)
            } else {
                Ok(OverlayHandlerResult::Continue(app::Overlay::Enrollment(
                    state,
                )))
            }
        }
        _ => Ok(OverlayHandlerResult::Continue(app::Overlay::Enrollment(
            state,
        ))),
    }
}

fn handle_parameter_overlay_event(
    key: KeyEvent,
    app: &mut App,
    mut state: app::ParameterFormState,
    event_tx: &EventSender,
) -> Result<OverlayHandlerResult> {
    let ctrl_submit = key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl_submit {
        state.editing = false;
        state.text_cursor = None;

        let provided = state.parameter_values();
        let sanitized = match validate_parameters(&state.command, &provided) {
            Ok(values) => values,
            Err(err) => {
                state.error = Some(err.to_string());
                return Ok(OverlayHandlerResult::Continue(app::Overlay::ParameterForm(
                    state,
                )));
            }
        };

        let entry = match app
            .find_server_by_id(&state.server_id)
            .and_then(|srv| srv.registry_entry.clone())
        {
            Some(entry) => entry,
            None => {
                state.error = Some("Server enrollment missing".to_string());
                return Ok(OverlayHandlerResult::Continue(app::Overlay::ParameterForm(
                    state,
                )));
            }
        };

        let overlay = begin_command_execution(
            app,
            event_tx,
            entry,
            state.command.clone(),
            state.server_label.clone(),
            sanitized,
        );
        return Ok(OverlayHandlerResult::Continue(overlay));
    }

    if state.editing {
        if let Some(field) = state.fields.get_mut(state.selected) {
            if let ParameterValue::Text(text) = &mut field.value {
                let cursor = state.text_cursor.unwrap_or_else(|| text.chars().count());
                let mut cursor = cursor;
                state.error = None;
                let normalized = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    match key.code {
                        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
                        other => other,
                    }
                } else {
                    key.code
                };

                match normalized {
                    KeyCode::Esc => {
                        state.editing = false;
                        state.text_cursor = None;
                    }
                    KeyCode::Enter => {
                        state.editing = false;
                        state.text_cursor = None;
                    }
                    KeyCode::Backspace => {
                        if cursor > 0 {
                            let remove_index = cursor - 1;
                            if remove_char_at(text, remove_index) {
                                cursor = remove_index;
                            }
                        }
                        state.text_cursor = Some(cursor);
                    }
                    KeyCode::Delete => {
                        if remove_char_at(text, cursor) {
                            state.text_cursor = Some(cursor);
                        }
                    }
                    KeyCode::Left => {
                        if cursor > 0 {
                            cursor -= 1;
                        }
                        state.text_cursor = Some(cursor);
                    }
                    KeyCode::Right => {
                        if cursor < text.chars().count() {
                            cursor += 1;
                        }
                        state.text_cursor = Some(cursor);
                    }
                    KeyCode::Home => {
                        state.text_cursor = Some(0);
                    }
                    KeyCode::End => {
                        state.text_cursor = Some(text.chars().count());
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        insert_char_at(text, cursor, c);
                        state.text_cursor = Some(cursor + 1);
                    }
                    _ => {}
                }
            } else {
                state.editing = false;
                state.text_cursor = None;
            }
        }
        return Ok(OverlayHandlerResult::Continue(app::Overlay::ParameterForm(
            state,
        )));
    }

    let field_count = state.fields.len();
    let normalized = if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        }
    } else {
        key.code
    };

    match normalized {
        KeyCode::Esc => return Ok(OverlayHandlerResult::Close),
        KeyCode::Up | KeyCode::BackTab => {
            if field_count > 0 {
                state.selected = if state.selected == 0 {
                    field_count - 1
                } else {
                    state.selected - 1
                };
                state.error = None;
                state.editing = false;
                state.text_cursor = None;
            }
        }
        KeyCode::Down | KeyCode::Tab => {
            if field_count > 0 {
                state.selected = (state.selected + 1) % field_count;
                state.error = None;
                state.editing = false;
                state.text_cursor = None;
            }
        }
        KeyCode::Left => {
            if let Some(field) = state.fields.get_mut(state.selected) {
                match &mut field.value {
                    ParameterValue::Slider {
                        value, min, step, ..
                    } => {
                        let new = value.saturating_sub(*step);
                        if let Some(min) = min {
                            *value = new.max(*min);
                        } else {
                            *value = new;
                        }
                    }
                    ParameterValue::Dropdown { selected, .. } => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                    }
                    ParameterValue::Toggle { value } => {
                        *value = !*value;
                    }
                    ParameterValue::Text(_) => {}
                }
                state.error = None;
            }
        }
        KeyCode::Right => {
            if let Some(field) = state.fields.get_mut(state.selected) {
                match &mut field.value {
                    ParameterValue::Slider {
                        value, max, step, ..
                    } => {
                        let mut new = value.saturating_add(*step);
                        if let Some(max) = max {
                            if new > *max {
                                new = *max;
                            }
                        }
                        *value = new;
                    }
                    ParameterValue::Dropdown { selected, options } => {
                        if *selected + 1 < options.len() {
                            *selected += 1;
                        }
                    }
                    ParameterValue::Toggle { value } => {
                        *value = !*value;
                    }
                    ParameterValue::Text(_) => {}
                }
                state.error = None;
            }
        }
        KeyCode::Char(' ') => {
            if let Some(field) = state.fields.get_mut(state.selected) {
                if let ParameterValue::Toggle { value } = &mut field.value {
                    *value = !*value;
                    state.error = None;
                }
            }
        }
        KeyCode::Enter => {
            if let Some(field) = state.fields.get_mut(state.selected) {
                if let ParameterValue::Text(text) = &mut field.value {
                    state.editing = true;
                    state.text_cursor = Some(text.chars().count());
                    state.error = None;
                }
            }
        }
        _ => {}
    }

    Ok(OverlayHandlerResult::Continue(app::Overlay::ParameterForm(
        state,
    )))
}

fn begin_command_execution(
    app: &mut App,
    event_tx: &EventSender,
    entry: ServerRegistryEntry,
    command: CommandSummary,
    server_label: String,
    parameters: HashMap<String, String>,
) -> app::Overlay {
    let auto_scroll = app.config.tui.auto_scroll;
    let server_id = entry.id;
    let command_id = command.id.clone();
    let command_name = command.name.clone();
    spawn_execute_command(event_tx.clone(), entry, command_id.clone(), parameters);
    app::Overlay::CommandExecution(app::CommandExecutionState::new(
        command_name,
        command_id,
        server_label,
        server_id,
        auto_scroll,
    ))
}

fn trigger_command_activation(app: &mut App, event_tx: &EventSender) -> Result<()> {
    if app.command_state.filtered_len() == 0 {
        app.set_status_message("No commands available");
        return Ok(());
    }

    let command = match app.command_state.selected_command() {
        Some(cmd) => cmd.clone(),
        None => {
            app.set_status_message("Select a command first");
            return Ok(());
        }
    };

    let server_id = match app.command_state.server_id {
        Some(id) => id,
        None => {
            app.set_status_message("Unknown target server for command");
            return Ok(());
        }
    };

    let server_entry = match app
        .find_server_by_id(&server_id)
        .and_then(|srv| srv.registry_entry.clone())
    {
        Some(entry) => entry,
        None => {
            app.set_status_message("Server not enrolled");
            return Ok(());
        }
    };

    let server_label = app
        .find_server_by_id(&server_id)
        .map(|srv| srv.label.clone())
        .unwrap_or_else(|| server_id.to_string());

    let needs_confirmation = command.requires_confirmation || app.config.tui.confirm_commands;

    if command.parameters.is_empty() && !needs_confirmation {
        let sanitized = validate_parameters(&command, &HashMap::new())?;
        let overlay = begin_command_execution(
            app,
            event_tx,
            server_entry,
            command,
            server_label,
            sanitized,
        );
        app.overlay = Some(overlay);
    } else {
        let form = app::ParameterFormState::new(command, server_id, server_label);
        app.overlay = Some(app::Overlay::ParameterForm(form));
    }

    Ok(())
}

fn open_remove_prompt(app: &mut App) -> Result<()> {
    let server = match app.selected_server() {
        Some(server) => server.clone(),
        None => {
            app.set_status_message("Select an enrolled server to remove");
            return Ok(());
        }
    };

    let server_id = match server.id {
        Some(id) => id,
        None => {
            app.set_status_message("Cannot remove a discovered server without enrollment");
            return Ok(());
        }
    };

    if server.registry_entry.is_none() {
        app.set_status_message("Server is not enrolled");
        return Ok(());
    }

    let prompt = app::PromptState {
        title: "Remove Enrollment".to_string(),
        message: format!(
            "Remove stored enrollment and certificates for {}?",
            server.label
        ),
        confirm_label: "Remove".to_string(),
        cancel_label: "Cancel".to_string(),
        action: app::PromptAction::RemoveServer { server_id },
    };

    app.overlay = Some(app::Overlay::Prompt(prompt));
    app.set_status_message(format!(
        "Confirm removal for {} (Enter removes, Esc cancels)",
        server.label
    ));
    Ok(())
}

fn open_tag_filter_overlay(app: &mut App) -> Result<()> {
    if app.command_state.commands.is_empty() {
        app.set_status_message("Load commands before filtering");
        return Ok(());
    }

    let mut tags = app.command_state.available_tags();
    if tags.is_empty() {
        app.set_status_message("Commands do not define tags");
        return Ok(());
    }

    tags.insert(0, "All".to_string());
    let current = app
        .command_state
        .tag_filter
        .as_ref()
        .and_then(|tag| tags.iter().position(|t| t == tag));

    let state = app::TagFilterState {
        tags,
        selected: current.unwrap_or(0),
    };

    app.overlay = Some(app::Overlay::TagFilter(state));
    Ok(())
}

fn handle_prompt_overlay_event(
    key: KeyEvent,
    app: &mut App,
    state: app::PromptState,
) -> Result<OverlayHandlerResult> {
    let normalized = if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        }
    } else {
        key.code
    };

    match normalized {
        KeyCode::Esc | KeyCode::Char('n') => Ok(OverlayHandlerResult::Close),
        KeyCode::Char('y') | KeyCode::Enter => {
            execute_prompt_action(app, &state.action)?;
            Ok(OverlayHandlerResult::Close)
        }
        _ => Ok(OverlayHandlerResult::Continue(app::Overlay::Prompt(state))),
    }
}

fn execute_prompt_action(app: &mut App, action: &app::PromptAction) -> Result<()> {
    match action {
        app::PromptAction::None => Ok(()),
        app::PromptAction::RemoveServer { server_id } => remove_server(app, *server_id),
    }
}

fn remove_server(app: &mut App, server_id: Uuid) -> Result<()> {
    let mut registry = ServerRegistry::load()?;
    registry.remove(&server_id);
    registry.save()?;

    if let Ok(mut dir) = config::config_dir() {
        dir.push("client-certs");
        dir.push(server_id.to_string());
        if dir.exists() {
            if let Err(err) = fs::remove_dir_all(&dir) {
                tracing::warn!(
                    "Failed to remove certificate directory {}: {}",
                    dir.display(),
                    err
                );
            }
        }
    }

    app.registry = registry;
    app.clear_commands();
    app.rebuild_servers();
    app.set_status_message("Enrollment removed");
    Ok(())
}

fn handle_tag_overlay_event(
    key: KeyEvent,
    app: &mut App,
    mut state: app::TagFilterState,
) -> Result<OverlayHandlerResult> {
    let len = state.tags.len();
    if len == 0 {
        return Ok(OverlayHandlerResult::Close);
    }

    let normalized = if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        }
    } else {
        key.code
    };

    match normalized {
        KeyCode::Esc => Ok(OverlayHandlerResult::Close),
        KeyCode::Up | KeyCode::BackTab => {
            state.selected = if state.selected == 0 {
                len - 1
            } else {
                state.selected - 1
            };
            Ok(OverlayHandlerResult::Continue(app::Overlay::TagFilter(
                state,
            )))
        }
        KeyCode::Down | KeyCode::Tab => {
            state.selected = (state.selected + 1) % len;
            Ok(OverlayHandlerResult::Continue(app::Overlay::TagFilter(
                state,
            )))
        }
        KeyCode::Enter => {
            if state.selected == 0 {
                app.command_state.set_tag_filter(None);
            } else if let Some(tag) = state.tags.get(state.selected) {
                app.command_state.set_tag_filter(Some(tag.clone()));
            }
            Ok(OverlayHandlerResult::Close)
        }
        _ => Ok(OverlayHandlerResult::Continue(app::Overlay::TagFilter(
            state,
        ))),
    }
}

fn move_selection_up(app: &mut App) {
    let len = app.servers.len();
    if len == 0 {
        return;
    }

    match app.selected_server {
        Some(current) if current > 0 => app.selected_server = Some(current - 1),
        Some(_) => {}
        None => app.selected_server = Some(len.saturating_sub(1)),
    }
    tracing::debug!(
        selected = ?app.selected_server,
        "moved server selection up"
    );
}

fn move_selection_down(app: &mut App) {
    let len = app.servers.len();
    if len == 0 {
        return;
    }

    match app.selected_server {
        Some(current) if current + 1 < len => app.selected_server = Some(current + 1),
        Some(_) => {}
        None => app.selected_server = Some(0),
    }
    tracing::debug!(
        selected = ?app.selected_server,
        "moved server selection down"
    );
}

fn insert_char_at(target: &mut String, index: usize, ch: char) {
    let len = target.chars().count();
    if index >= len {
        target.push(ch);
        return;
    }

    let mut buffer = String::with_capacity(target.len() + ch.len_utf8());
    for (i, existing) in target.chars().enumerate() {
        if i == index {
            buffer.push(ch);
        }
        buffer.push(existing);
    }
    *target = buffer;
}

fn remove_char_at(target: &mut String, index: usize) -> bool {
    let len = target.chars().count();
    if index >= len {
        return false;
    }

    let mut buffer = String::with_capacity(target.len());
    for (i, existing) in target.chars().enumerate() {
        if i != index {
            buffer.push(existing);
        }
    }
    *target = buffer;
    true
}

fn format_text_with_cursor(value: &str, editing: bool, cursor: Option<usize>) -> String {
    if !editing {
        return value.to_string();
    }

    let mut result = String::new();
    let len = value.chars().count();
    let cursor = cursor.unwrap_or(len).min(len);
    for (idx, ch) in value.chars().enumerate() {
        if idx == cursor {
            result.push('|');
        }
        result.push(ch);
    }
    if cursor == len {
        result.push('|');
    }
    result
}

fn parameter_value_display(
    field: &app::ParameterFieldState,
    is_selected: bool,
    editing: bool,
    cursor: Option<usize>,
) -> String {
    match &field.value {
        ParameterValue::Slider {
            value, min, max, ..
        } => {
            let mut text = value.to_string();
            if let Some(min) = min {
                text.push_str(&format!(" (min {min}"));
                if let Some(max) = max {
                    text.push_str(&format!(", max {max})"));
                } else {
                    text.push(')');
                }
            } else if let Some(max) = max {
                text.push_str(&format!(" (max {max})"));
            }
            text
        }
        ParameterValue::Text(text) => {
            let show_cursor = is_selected && editing;
            format_text_with_cursor(text, show_cursor, cursor)
        }
        ParameterValue::Toggle { value } => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        ParameterValue::Dropdown { options, selected } => options
            .get(*selected)
            .cloned()
            .unwrap_or_else(|| "".to_string()),
    }
}

fn move_command_selection_up(app: &mut App) {
    if app.command_state.filtered_len() == 0 {
        return;
    }
    app.command_state.select_previous();
}

fn move_command_selection_down(app: &mut App) {
    if app.command_state.filtered_len() == 0 {
        return;
    }
    app.command_state.select_next();
}

fn toggle_focus(app: &mut App) {
    match app.focus {
        FocusPane::Servers => {
            if !app.command_state.commands.is_empty() {
                app.focus = FocusPane::Commands;
            }
        }
        FocusPane::Commands => {
            app.focus = FocusPane::Servers;
        }
    }
}

fn draw(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &App) -> Result<()> {
    terminal.draw(|frame| {
        let area = frame.area();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

        render_header(frame, layout[0], app);
        render_main(frame, layout[1], app);
        render_status(frame, layout[2], app);
        render_overlay(frame, layout[1], app);
    })?;
    Ok(())
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focus_text = match app.focus {
        FocusPane::Servers => "Servers",
        FocusPane::Commands => "Commands",
    };
    let title = format!("HandControl TUI v{}", APP_VERSION);
    let line = Line::from(vec![
        Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            "[d] Discover  [e] Enroll  [r] Remove  [/] Search  [t] Tags  [Tab] Switch",
            Style::default().fg(Color::Gray),
        ),
        Span::raw("   "),
        Span::styled(
            format!("Focus: {focus_text}"),
            Style::default().fg(Color::Gray),
        ),
    ]);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn render_main(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);

    render_servers(frame, columns[0], app);
    render_commands(frame, columns[1], app);
}

fn render_servers(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let highlight_style = if matches!(app.focus, FocusPane::Servers) {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    };

    let items: Vec<ListItem> = if app.servers.is_empty() {
        vec![ListItem::new(Line::from(
            "No servers found. Press 'd' to discover.",
        ))]
    } else {
        app.servers
            .iter()
            .map(|server| {
                let mut lines = Vec::new();
                let status_style = match server.status {
                    ServerStatus::Enrolled => Style::default().fg(Color::Green),
                    ServerStatus::Available => Style::default().fg(Color::Blue),
                    ServerStatus::Unknown => Style::default().fg(Color::Yellow),
                };
                lines.push(Line::from(Span::styled(
                    server.label.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                if let Some(subtitle) = &server.subtitle {
                    lines.push(Line::from(Span::styled(
                        subtitle.clone(),
                        Style::default().fg(Color::Gray),
                    )));
                }
                let status_text = match server.status {
                    ServerStatus::Enrolled => "Enrolled",
                    ServerStatus::Available => "Available",
                    ServerStatus::Unknown => "Unknown",
                };
                lines.push(Line::from(Span::styled(status_text, status_style)));
                ListItem::new(lines)
            })
            .collect()
    };

    let title = format!(
        "Servers ({}) [d] Discover [e] Enroll [r] Remove",
        app.servers.len()
    );
    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(highlight_style)
        .highlight_symbol("> ");
    let mut state = ListState::default();
    state.select(app.selected_server);
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_commands(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let highlight_style = if matches!(app.focus, FocusPane::Commands) {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    };

    let mut title = String::from("Commands");
    if let Some(server_id) = app.command_state.server_id {
        if let Some(server) = app.find_server_by_id(&server_id) {
            title.push_str(&format!(" ({})", server.label));
        }
    }
    if !app.command_state.search_query.is_empty() {
        title.push_str(&format!(" /{}", app.command_state.search_query));
    }
    if let Some(tag) = &app.command_state.tag_filter {
        title.push_str(&format!(" tag:{}", tag));
    }
    title.push_str(" [/] Search [t] Tags [x] Execute");

    if app.command_state.loading {
        let paragraph = Paragraph::new("Loading commands...")
            .block(Block::default().title(title).borders(Borders::ALL));
        frame.render_widget(paragraph, area);
        return;
    }

    if app.command_state.filtered_len() == 0 {
        let message = if app.command_state.commands.is_empty() {
            "No commands loaded. Select an enrolled server and press Enter."
        } else if !app.command_state.search_query.is_empty()
            || app.command_state.tag_filter.is_some()
        {
            "No commands match the active filters."
        } else {
            "No commands available on this server."
        };
        let paragraph =
            Paragraph::new(message).block(Block::default().title(title).borders(Borders::ALL));
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = app
        .command_state
        .filtered_iter()
        .map(|(_, command)| {
            let mut lines = Vec::new();
            let mut name_spans = vec![Span::styled(
                command.name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )];
            if command.requires_confirmation {
                name_spans.push(Span::styled(
                    " !confirm",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            lines.push(Line::from(name_spans));
            if let Some(desc) = &command.description {
                lines.push(Line::from(Span::raw(desc.clone())));
            }
            if !command.tags.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("#{}", command.tags.join(" #")),
                    Style::default().fg(Color::Gray),
                )));
            }
            ListItem::new(lines)
        })
        .collect();

    let mut state = ListState::default();
    let selected_pos = app.command_state.selected.as_ref().and_then(|selected| {
        app.command_state
            .filtered_indices
            .iter()
            .position(|idx| idx == selected)
    });
    state.select(selected_pos);

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(highlight_style)
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let status_message = if app.discovery_in_progress {
        "Discovering servers..."
    } else if app.status.message.is_empty() {
        "Ready"
    } else {
        app.status.message.as_str()
    };

    let mut spans = Vec::new();
    spans.push(Span::styled(
        "Status: ",
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(status_message.to_string()));
    spans.push(Span::raw("   "));
    spans.push(Span::styled(
        format!(
            "Servers: {}  Commands: {}",
            app.servers.len(),
            app.command_state.filtered_len()
        ),
        Style::default().fg(Color::Gray),
    ));
    if app.command_state.loading {
        spans.push(Span::raw("   Loading..."));
    }
    if !app.command_state.search_query.is_empty() {
        spans.push(Span::raw(format!(
            "   Search: {}",
            app.command_state.search_query
        )));
    }
    if let Some(tag) = &app.command_state.tag_filter {
        spans.push(Span::raw(format!("   Tag: {}", tag)));
    }

    let paragraph = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::TOP));
    frame.render_widget(paragraph, area);
}

fn render_overlay(frame: &mut Frame<'_>, content_area: Rect, app: &App) {
    if let Some(overlay) = &app.overlay {
        match overlay {
            app::Overlay::TagFilter(state) => {
                let area = centered_rect(content_area, 40, 40);
                frame.render_widget(Clear, area);
                let block = Block::default().title("Filter Tags").borders(Borders::ALL);
                let inner = Rect {
                    x: area.x.saturating_add(1),
                    y: area.y.saturating_add(1),
                    width: area.width.saturating_sub(2),
                    height: area.height.saturating_sub(2),
                };
                frame.render_widget(block, area);

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(1)])
                    .split(inner);

                let items: Vec<ListItem> = state
                    .tags
                    .iter()
                    .enumerate()
                    .map(|(idx, tag)| {
                        let label = if idx == 0 { "All" } else { tag.as_str() };
                        ListItem::new(Line::from(Span::raw(label.to_string())))
                    })
                    .collect();

                let mut list_state = ListState::default();
                list_state.select(Some(state.selected.min(state.tags.len().saturating_sub(1))));
                let list = List::new(items)
                    .highlight_style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("> ");
                frame.render_stateful_widget(list, chunks[0], &mut list_state);

                let instructions = Paragraph::new("Enter apply · Esc cancel")
                    .style(Style::default().fg(Color::Gray));
                frame.render_widget(instructions, chunks[1]);
            }
            app::Overlay::Prompt(state) => {
                let area = centered_rect(content_area, 50, 35);
                frame.render_widget(Clear, area);
                let block = Block::default()
                    .title(state.title.as_str())
                    .borders(Borders::ALL);
                frame.render_widget(block, area);

                let inner = Rect {
                    x: area.x.saturating_add(1),
                    y: area.y.saturating_add(1),
                    width: area.width.saturating_sub(2),
                    height: area.height.saturating_sub(2),
                };

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(1)])
                    .split(inner);

                let message = Paragraph::new(state.message.as_str());
                frame.render_widget(message, chunks[0]);

                let instructions = format!(
                    "[Enter/y] {} · [Esc/n] {}",
                    state.confirm_label, state.cancel_label
                );
                frame.render_widget(
                    Paragraph::new(instructions).style(Style::default().fg(Color::Gray)),
                    chunks[1],
                );
            }
            app::Overlay::Enrollment(state) => {
                let area = centered_rect(content_area, 60, 50);
                frame.render_widget(Clear, area);
                let title = format!("Enroll {}", state.server.label);
                let block = Block::default().title(title).borders(Borders::ALL);
                frame.render_widget(block, area);

                let inner = Rect {
                    x: area.x.saturating_add(1),
                    y: area.y.saturating_add(1),
                    width: area.width.saturating_sub(2),
                    height: area.height.saturating_sub(2),
                };

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(3), Constraint::Length(1)])
                    .split(inner);

                let mut lines: Vec<Line> = Vec::new();
                lines.push(Line::from(vec![Span::styled(
                    state.server.label.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )]));
                lines.push(Line::from(""));

                let instructions = match &state.step {
                    app::EnrollmentStep::Approval(approval) => {
                        lines.push(Line::from(Span::styled(
                            approval.status_message.clone(),
                            Style::default().fg(Color::Gray),
                        )));
                        if let Some(code) = &approval.verification_code {
                            lines.push(Line::from(""));
                            lines.push(Line::from(vec![Span::styled(
                                code.clone(),
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                            )]));
                            lines.push(Line::from(Span::styled(
                                "Enter this code on the server to approve enrollment",
                                Style::default().fg(Color::Gray),
                            )));
                            "Esc hides overlay after noting the code"
                        } else {
                            "Waiting for verification code · Esc hides overlay"
                        }
                    }
                    app::EnrollmentStep::InProgress { message } => {
                        lines.push(Line::from(Span::raw(message.clone())));
                        "Esc hides overlay"
                    }
                    app::EnrollmentStep::Completed { message } => {
                        lines.push(Line::from(Span::styled(
                            message.clone(),
                            Style::default().fg(Color::Green),
                        )));
                        "Press Enter or Esc to close"
                    }
                    app::EnrollmentStep::Error { message } => {
                        lines.push(Line::from(Span::styled(
                            message.clone(),
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        )));
                        "Press Enter or Esc to close"
                    }
                    other => {
                        lines.push(Line::from(Span::styled(
                            format!("{other:?}"),
                            Style::default().fg(Color::Gray),
                        )));
                        "Esc hides overlay"
                    }
                };

                let body = Paragraph::new(lines).alignment(Alignment::Center);
                frame.render_widget(body, chunks[0]);
                frame.render_widget(
                    Paragraph::new(instructions).style(Style::default().fg(Color::Gray)),
                    chunks[1],
                );
            }
            app::Overlay::ParameterForm(state) => {
                let area = centered_rect(content_area, 70, 75);
                frame.render_widget(Clear, area);
                let title = format!("Execute {} on {}", state.command.name, state.server_label);
                let block = Block::default().title(title).borders(Borders::ALL);
                let inner = Rect {
                    x: area.x.saturating_add(1),
                    y: area.y.saturating_add(1),
                    width: area.width.saturating_sub(2),
                    height: area.height.saturating_sub(2),
                };
                frame.render_widget(block, area);

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(3),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                let items: Vec<ListItem> = if state.fields.is_empty() {
                    vec![ListItem::new(Line::from(
                        "No parameters. Press Ctrl+Enter to run.",
                    ))]
                } else {
                    state
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(idx, field)| {
                            let type_label = match field.definition.param_type {
                                CommandParameterType::Slider => "slider",
                                CommandParameterType::Text => "text",
                                CommandParameterType::Toggle => "toggle",
                                CommandParameterType::Dropdown => "dropdown",
                            };
                            let value_display = parameter_value_display(
                                field,
                                idx == state.selected,
                                state.editing && idx == state.selected,
                                if state.editing && idx == state.selected {
                                    state.text_cursor
                                } else {
                                    None
                                },
                            );
                            let mut lines = Vec::new();
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("{}", field.definition.name),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                                Span::raw("  "),
                                Span::styled(
                                    format!("[{type_label}]"),
                                    Style::default().fg(Color::Gray),
                                ),
                            ]));
                            lines.push(Line::from(Span::raw(value_display)));
                            if let Some(desc) = &field.definition.description {
                                lines.push(Line::from(Span::styled(
                                    desc.clone(),
                                    Style::default().fg(Color::Gray),
                                )));
                            }
                            ListItem::new(lines)
                        })
                        .collect()
                };

                let mut list_state = ListState::default();
                if !state.fields.is_empty() {
                    list_state.select(Some(state.selected));
                }

                let highlight_style = if state.editing {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD | Modifier::ITALIC)
                } else {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                };

                let list = List::new(items)
                    .highlight_style(highlight_style)
                    .highlight_symbol("> ");
                frame.render_stateful_widget(list, chunks[0], &mut list_state);

                if let Some(err) = &state.error {
                    let error = Paragraph::new(err.clone())
                        .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
                    frame.render_widget(error, chunks[1]);
                }

                let instructions = if state.fields.is_empty() {
                    "Ctrl+Enter run · Esc cancel"
                } else {
                    "Ctrl+Enter run · Esc cancel · Enter edit text · Space toggle · ←/→ adjust"
                };
                frame.render_widget(
                    Paragraph::new(instructions).style(Style::default().fg(Color::Gray)),
                    chunks[2],
                );
            }
            app::Overlay::CommandExecution(state) => {
                let area = centered_rect(content_area, 80, 80);
                frame.render_widget(Clear, area);
                let status_title = if state.running {
                    "Running"
                } else {
                    "Completed"
                };
                let mut title = format!(
                    "{}: {} on {}",
                    status_title, state.command_name, state.server_label
                );
                if let Some(code) = state.exit_code {
                    title.push_str(&format!(" · exit {code}"));
                }
                let block = Block::default().title(title).borders(Borders::ALL);
                let inner = Rect {
                    x: area.x.saturating_add(1),
                    y: area.y.saturating_add(1),
                    width: area.width.saturating_sub(2),
                    height: area.height.saturating_sub(2),
                };
                frame.render_widget(block, area);

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(3),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                let visible_lines = chunks[0].height as usize;
                let total_lines = state.output.len();
                let offset = state.scroll_offset.min(total_lines);
                let end = total_lines.saturating_sub(offset);
                let start = end.saturating_sub(visible_lines);
                let slice = &state.output[start..end];

                let items: Vec<ListItem> = slice
                    .iter()
                    .map(|line| {
                        let (label, color) = match line.channel {
                            OutputChannel::Stdout => ("stdout", Color::Green),
                            OutputChannel::Stderr => ("stderr", Color::Red),
                            OutputChannel::Status => ("status", Color::Gray),
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                format!("{label}: "),
                                Style::default().fg(color).add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(line.content.clone()),
                        ]))
                    })
                    .collect();

                let list = List::new(items);
                frame.render_widget(list, chunks[0]);

                if let Some(error) = &state.error {
                    frame.render_widget(
                        Paragraph::new(error.clone())
                            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        chunks[1],
                    );
                }

                let instructions = if state.running {
                    "Command running… Esc closes once finished"
                } else {
                    "Esc to close"
                };
                frame.render_widget(
                    Paragraph::new(instructions).style(Style::default().fg(Color::Gray)),
                    chunks[2],
                );
            }
            app::Overlay::Search(state) => {
                let area = centered_rect(content_area, 60, 30);
                let inner = Rect {
                    x: area.x.saturating_add(1),
                    y: area.y.saturating_add(1),
                    width: area.width.saturating_sub(2),
                    height: area.height.saturating_sub(2),
                };
                let prefix = "Query: ";
                let instructions = "Enter to apply · Esc to cancel";
                let paragraph = Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled(prefix, Style::default().fg(Color::Gray)),
                        Span::raw(state.query.clone()),
                    ]),
                    Line::from(Span::styled(instructions, Style::default().fg(Color::Gray))),
                ])
                .block(
                    Block::default()
                        .title("Search Commands")
                        .borders(Borders::ALL),
                );
                frame.render_widget(Clear, area);
                frame.render_widget(paragraph, area);

                let cursor_offset = prefix.chars().count() as u16 + state.cursor as u16;
                frame.set_cursor_position((
                    inner.x + cursor_offset.min(inner.width.saturating_sub(1)),
                    inner.y,
                ));
            }
            app::Overlay::Help => {
                let message = Paragraph::new("Help is coming soon.")
                    .block(Block::default().title("Help").borders(Borders::ALL));
                let area = centered_rect(content_area, 60, 40);
                frame.render_widget(Clear, area);
                frame.render_widget(message, area);
            }
        }
    }
}

fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}
