use anyhow::{Result, anyhow};
use clap::{Arg, Command};
use hull::config::{Config, get_config_path, get_db_path, get_root_path, get_socket_path};
use hull::database::Database;
use hull::interfaces::Interface;
use hull::protocol::{
    Command as HullCommand, InterfaceCommand, Request, RouterCommand, RouterLinkCommand,
    RouterRouteCommand, SwitchCommand, SwitchPortCommand, error_response,
};
use hull::{interfaces, routers, switches};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, trace};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::writer::MakeWriterExt;

/// Daemon log output format.
#[derive(Debug, Clone, Copy)]
pub enum LogFormat {
    /// Human-readable log lines.
    Text,
    /// JSON log lines.
    Json,
}

fn main() {
    let matches = Command::new("hulld")
        .about("Hull daemon")
        .arg(
            Arg::new("log-format")
                .long("log-format")
                .value_name("FORMAT")
                .help("Set hulld log format: text or json")
                .value_parser(["text", "json"])
                .default_value("text"),
        )
        .arg(
            Arg::new("log-level")
                .long("log-level")
                .value_name("LEVEL")
                .help("Set hulld log level: error, warn, info, debug, or trace")
                .value_parser(["error", "warn", "info", "debug", "trace"])
                .default_value("info"),
        )
        .arg(
            Arg::new("log-file")
                .long("log-file")
                .value_name("FILE")
                .help("Write hulld logs to a file while also logging to stderr")
                .value_parser(clap::value_parser!(PathBuf))
                .default_value("/var/logs/hull/hulld.log"),
        )
        .get_matches();

    let log_format = match matches
        .get_one::<String>("log-format")
        .map_or("text", std::string::String::as_str)
    {
        "json" => LogFormat::Json,
        _ => LogFormat::Text,
    };

    let log_level = match matches
        .get_one::<String>("log-level")
        .map_or("info", std::string::String::as_str)
    {
        "error" => LevelFilter::ERROR,
        "warn" => LevelFilter::WARN,
        "debug" => LevelFilter::DEBUG,
        "trace" => LevelFilter::TRACE,
        _ => LevelFilter::INFO,
    };

    let log_file = matches
        .get_one::<PathBuf>("log-file")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("/var/logs/hull/hulld.log"));

    if let Err(e) = run(log_format, log_level, log_file) {
        use std::io::Write;

        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "hulld failed: {e:#}");
        std::process::exit(1);
    }
}

/// Run the hulld daemon.
///
/// # Errors
/// Returns an error if the process is not root, the socket cannot be created, or the
/// daemon cannot accept or handle requests.
pub fn run(log_format: LogFormat, log_level: LevelFilter, log_file: PathBuf) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_async(log_format, log_level, log_file))
}

async fn run_async(log_format: LogFormat, log_level: LevelFilter, log_file: PathBuf) -> Result<()> {
    let output = std::process::Command::new("id").arg("-u").output()?;
    let uid = String::from_utf8(output.stdout)?;
    if uid.trim() != "0" {
        return Err(anyhow!("hulld must be run as root"));
    }

    let root_path = get_root_path();
    fs::create_dir_all(&root_path)?;

    init_logging(log_format, log_level, &log_file)?;
    debug!(
        ?log_format,
        log_level = %log_level,
        log_file = %log_file.display(),
        "initialized logging"
    );

    reconcile_startup_state(&root_path).await?;

    let socket_path = get_socket_path(&root_path);
    if socket_path.exists() {
        trace!(socket_path = %socket_path.display(), "removing stale socket");
        let _ = fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o666))?;
    log_daemon_start(&socket_path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                trace!("accepted daemon connection");
                if let Err(e) = handle_connection(stream).await {
                    error!(event = "done", operation = "daemon", message = %format!("hulld request error: {e:#}"));
                }
            }
            Err(e) => {
                error!(event = "done", operation = "daemon", message = %format!("hulld socket error: {e}"));
            }
        }
    }
}

fn init_logging(log_format: LogFormat, log_level: LevelFilter, log_file: &Path) -> Result<()> {
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)?;
    let writer = std::io::stderr.and(Mutex::new(file));

    match log_format {
        LogFormat::Text => {
            let subscriber = tracing_subscriber::fmt()
                .with_timer(tracing_subscriber::fmt::time::SystemTime)
                .with_target(false)
                .with_max_level(log_level)
                .compact()
                .with_writer(writer)
                .finish();
            tracing::subscriber::set_global_default(subscriber)?;
        }
        LogFormat::Json => {
            let subscriber = tracing_subscriber::fmt()
                .with_timer(tracing_subscriber::fmt::time::SystemTime)
                .with_target(false)
                .with_max_level(log_level)
                .json()
                .flatten_event(true)
                .with_current_span(false)
                .with_span_list(false)
                .with_writer(writer)
                .finish();
            tracing::subscriber::set_global_default(subscriber)?;
        }
    }

    Ok(())
}

async fn reconcile_startup_state(root_path: &Path) -> Result<()> {
    let db_path = get_db_path(root_path);
    if !db_path.exists() {
        info!(db_path = %db_path.display(), "skipping startup reconciliation");
        return Ok(());
    }

    let config_path = get_config_path(root_path, None);
    let config = Config::load(&config_path)?;
    info!(
        db_path = %db_path.display(),
        bridge = %config.bridge_name,
        "reconciling startup infrastructure"
    );

    let storage = Arc::new(Database::new(db_path));
    let ovs = hull::ovs::BridgeClient::connect().await?;
    let ovs = Arc::new(ovs);
    ensure_infrastructure(&config, &ovs).await?;

    let interface_ops = interfaces::InterfaceOps::new(storage);
    interface_ops.sync().await?;

    info!(
        bridge = %config.bridge_name,
        "completed startup reconciliation"
    );

    Ok(())
}

fn log_daemon_start(socket_path: &std::path::Path) {
    info!(
        event = "start",
        operation = %socket_path.display(),
        message = "daemon listening"
    );
}

fn log_request_start(op: &str) {
    info!(
        event = "start",
        operation = %op,
        message = %format!("starting {op}")
    );
}

fn log_request_ok(op: &str) {
    info!(
        event = "done",
        operation = %op,
        message = %format!("completed {op}")
    );
}

fn log_request_err(op: &str, message: &str) {
    error!(event = "done", operation = %op, message = %message);
}

async fn handle_connection(mut stream: UnixStream) -> Result<()> {
    let mut buffer = Vec::new();
    let mut temp_buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut temp_buf).await?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(temp_buf.get(..n).unwrap_or(&[]));
        trace!(
            bytes_read = n,
            buffered_bytes = buffer.len(),
            "read request chunk"
        );
        if serde_json::from_slice::<Request>(&buffer).is_ok() {
            break;
        }
    }

    let request: Request = serde_json::from_slice(&buffer)?;
    let op = operation_name(&request.command);
    debug!(operation = op, "parsed request");
    log_request_start(op);
    let response = match handle_request(request).await {
        Ok(value) => {
            log_request_ok(op);
            value
        }
        Err(e) => {
            log_request_err(op, &format!("{e:#}"));
            error_response(format!("{e:#}"))
        }
    };

    let response_bytes = serde_json::to_vec_pretty(&response)?;
    stream.write_all(&response_bytes).await?;
    stream.shutdown().await?;
    Ok(())
}

fn operation_name(command: &HullCommand) -> &'static str {
    trace!("resolving operation name");
    match command {
        HullCommand::Init => "init",
        HullCommand::Deinit => "deinit",
        HullCommand::Interface(InterfaceCommand::Ls) => "interface.ls",
        HullCommand::Interface(InterfaceCommand::Create { .. }) => "interface.create",
        HullCommand::Interface(InterfaceCommand::Rm { .. }) => "interface.rm",
        HullCommand::Switch(SwitchCommand::Ls) => "switch.ls",
        HullCommand::Switch(SwitchCommand::Create { .. }) => "switch.create",
        HullCommand::Switch(SwitchCommand::Rm { .. }) => "switch.rm",
        HullCommand::Switch(SwitchCommand::Port(SwitchPortCommand::Ls)) => "switch.port.ls",
        HullCommand::Switch(SwitchCommand::Port(SwitchPortCommand::Create { .. })) => {
            "switch.port.create"
        }
        HullCommand::Switch(SwitchCommand::Port(SwitchPortCommand::Rm { .. })) => "switch.port.rm",
        HullCommand::Router(RouterCommand::Ls) => "router.ls",
        HullCommand::Router(RouterCommand::Create { .. }) => "router.create",
        HullCommand::Router(RouterCommand::Rm { .. }) => "router.rm",
        HullCommand::Router(RouterCommand::Attach { .. }) => "router.attach",
        HullCommand::Router(RouterCommand::Detach { .. }) => "router.detach",
        HullCommand::Router(RouterCommand::Link(_)) => "router.link",
        HullCommand::Router(RouterCommand::Route(_)) => "router.route",
        HullCommand::Sync => "sync",
    }
}

async fn handle_request(request: Request) -> Result<serde_json::Value> {
    let Request {
        config,
        bridge_name,
        command,
    } = request;
    let root_path = get_root_path();
    let config_path = get_config_path(&root_path, config);
    let db_path = get_db_path(&root_path);
    let is_hull_initialized = root_path.exists() && db_path.exists();
    debug!(
        root_path = %root_path.display(),
        config_path = %config_path.display(),
        db_path = %db_path.display(),
        is_hull_initialized,
        "dispatching request"
    );

    let ovs = hull::ovs::BridgeClient::connect().await?;

    if matches!(&command, HullCommand::Init) {
        return handle_init(root_path, config_path, db_path, bridge_name, &ovs).await;
    }

    if matches!(&command, HullCommand::Deinit) {
        return handle_deinit(root_path, config_path, db_path, &ovs).await;
    }

    if !is_hull_initialized {
        return Err(anyhow!(
            "Hull is not initialized. Please run `hull init` first."
        ));
    }

    let config = Config::load(&config_path)?;
    trace!(bridge = %config.bridge_name, "loaded config");
    let storage = Arc::new(Database::new(db_path));
    let config_arc = Arc::new(config.clone());
    let ovs_arc = Arc::new(ovs);
    let interface_ops = interfaces::InterfaceOps::new(storage.clone());
    let switch_ops = switches::SwitchOps::new(storage.clone(), config_arc.clone(), ovs_arc.clone());
    let router_ops = routers::RouterOps::new(storage.clone(), config_arc.clone(), ovs_arc.clone());

    ensure_infrastructure(&config, &ovs_arc).await?;

    handle_initialized_command(
        command,
        config,
        storage,
        interface_ops,
        switch_ops,
        router_ops,
    )
    .await
}

async fn handle_init(
    root_path: PathBuf,
    config_path: PathBuf,
    db_path: PathBuf,
    bridge_name: Option<String>,
    ovs: &hull::ovs::BridgeClient,
) -> Result<serde_json::Value> {
    if db_path.exists() {
        return Err(anyhow!(
            "Hull is already initialized at '{}'. Please run `hull deinit` first.",
            root_path.display()
        ));
    }

    debug!(
        root_path = %root_path.display(),
        config_path = %config_path.display(),
        db_path = %db_path.display(),
        bridge_name = ?bridge_name,
        "initializing hull"
    );
    fs::create_dir_all(&root_path)?;

    let config = Config {
        bridge_name: bridge_name.unwrap_or_else(|| Config::default().bridge_name),
    };
    trace!(bridge = %config.bridge_name, "saving config");
    config.save(&config_path)?;

    let storage = Database::new(db_path);
    trace!("initializing database");
    storage.init()?;
    debug!(bridge = %config.bridge_name, "creating infrastructure");
    create_infrastructure(&config, ovs).await?;

    Ok(serde_json::json!({
        "status": "success",
        "message": "Hull initialized",
    }))
}

async fn handle_deinit(
    root_path: PathBuf,
    config_path: PathBuf,
    db_path: PathBuf,
    ovs: &hull::ovs::BridgeClient,
) -> Result<serde_json::Value> {
    if !db_path.exists() {
        return Err(anyhow!(
            "Hull is not initialized. No directory found at '{}'.",
            root_path.display()
        ));
    }

    debug!(
        root_path = %root_path.display(),
        config_path = %config_path.display(),
        db_path = %db_path.display(),
        "deinitializing hull"
    );
    let storage = Database::new(db_path.clone());
    let config = Config::load(&config_path)?;
    trace!(bridge = %config.bridge_name, "destroying infrastructure");
    destroy_infrastructure(&config, &storage, ovs).await?;

    drop(storage);
    trace!("removing persisted state");
    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_file(&config_path);

    Ok(serde_json::json!({
        "status": "success",
        "message": "Hull deinitialized",
    }))
}

async fn handle_initialized_command(
    command: HullCommand,
    config: Config,
    storage: Arc<Database>,
    interface_ops: interfaces::InterfaceOps,
    switch_ops: switches::SwitchOps,
    router_ops: routers::RouterOps,
) -> Result<serde_json::Value> {
    match command {
        HullCommand::Interface(subcommand) => {
            handle_interface_command(subcommand, &interface_ops).await
        }
        HullCommand::Switch(subcommand) => {
            handle_switch_command(subcommand, &config, &storage, &switch_ops, &router_ops).await
        }
        HullCommand::Router(subcommand) => {
            handle_router_command(subcommand, &config, &switch_ops, &router_ops).await
        }
        HullCommand::Sync => {
            interface_ops.sync().await?;
            reconcile_all_flows(&config, &switch_ops, &router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "All flows synced from database state",
            }))
        }
        HullCommand::Init | HullCommand::Deinit => unreachable!(),
    }
}

async fn handle_interface_command(
    subcommand: InterfaceCommand,
    interface_ops: &interfaces::InterfaceOps,
) -> Result<serde_json::Value> {
    match subcommand {
        InterfaceCommand::Ls => Ok(interface_list_response(interface_ops.list()?)),
        InterfaceCommand::Create { name, mac } => {
            let interface = interface_ops.create(&name, mac.as_deref()).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Created interface",
                "name": name,
                "mac": interface.mac,
            }))
        }
        InterfaceCommand::Rm { name } => {
            interface_ops.remove(&name).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Removed interface",
                "name": name,
            }))
        }
    }
}

async fn handle_switch_command(
    subcommand: SwitchCommand,
    config: &Config,
    storage: &Database,
    switch_ops: &switches::SwitchOps,
    router_ops: &routers::RouterOps,
) -> Result<serde_json::Value> {
    match subcommand {
        SwitchCommand::Ls => switch_list_response(switch_ops.list()?, storage),
        SwitchCommand::Create { name, ip, mask } => {
            switch_ops.create(&name, &ip, mask).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Created switch",
                "name": name,
            }))
        }
        SwitchCommand::Rm { name } => {
            switch_ops.remove(&name).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Removed switch",
                "name": name,
            }))
        }
        SwitchCommand::Port(subcommand) => {
            handle_switch_port_command(subcommand, config, switch_ops, router_ops).await
        }
    }
}

async fn handle_switch_port_command(
    subcommand: SwitchPortCommand,
    config: &Config,
    switch_ops: &switches::SwitchOps,
    router_ops: &routers::RouterOps,
) -> Result<serde_json::Value> {
    match subcommand {
        SwitchPortCommand::Ls => Ok(switch_port_list_response(switch_ops.list_switch_ports()?)),
        SwitchPortCommand::Create {
            name,
            switch,
            interface,
        } => {
            switch_ops
                .create_switch_port(&name, &switch, &interface)
                .await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Created switch port",
                "name": name,
                "switch": switch,
                "interface": interface,
            }))
        }
        SwitchPortCommand::Rm { switch, name } => {
            switch_ops.remove_switch_port(&switch, &name).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Removed switch port",
                "name": name,
                "switch": switch,
            }))
        }
    }
}

async fn handle_router_command(
    subcommand: RouterCommand,
    config: &Config,
    switch_ops: &switches::SwitchOps,
    router_ops: &routers::RouterOps,
) -> Result<serde_json::Value> {
    match subcommand {
        RouterCommand::Ls => router_list_response(router_ops.list()?, router_ops),
        RouterCommand::Create { name } => {
            router_ops.create(&name).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Created router",
                "name": name,
            }))
        }
        RouterCommand::Rm { name } => {
            router_ops.remove(&name).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Removed router",
                "name": name,
            }))
        }
        RouterCommand::Attach { router, switch } => {
            router_ops.attach(&router, &switch).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Attached switch to router",
                "router": router,
                "switch": switch,
            }))
        }
        RouterCommand::Detach { router, switch } => {
            router_ops.detach(&router, &switch).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Detached switch from router",
                "router": router,
                "switch": switch,
            }))
        }
        RouterCommand::Link(subcommand) => {
            handle_router_link_command(subcommand, config, switch_ops, router_ops).await
        }
        RouterCommand::Route(subcommand) => {
            handle_router_route_command(subcommand, config, switch_ops, router_ops).await
        }
    }
}

async fn handle_router_link_command(
    subcommand: RouterLinkCommand,
    config: &Config,
    switch_ops: &switches::SwitchOps,
    router_ops: &routers::RouterOps,
) -> Result<serde_json::Value> {
    match subcommand {
        RouterLinkCommand::Set {
            router,
            port,
            ip,
            mac,
        } => {
            router_ops.set_link(&router, &port, &ip, &mac).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Set router link",
                "router": router,
                "link": port,
                "ip": ip,
                "mac": mac,
            }))
        }
        RouterLinkCommand::Unset { router } => {
            router_ops.unset_link(&router).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Unset router link",
                "router": router,
            }))
        }
    }
}

async fn handle_router_route_command(
    subcommand: RouterRouteCommand,
    config: &Config,
    switch_ops: &switches::SwitchOps,
    router_ops: &routers::RouterOps,
) -> Result<serde_json::Value> {
    match subcommand {
        RouterRouteCommand::Add {
            router,
            source,
            destination,
            next_hop,
            next_hop_mac,
            metric,
        } => {
            router_ops
                .add_route(
                    &router,
                    &source,
                    &destination,
                    next_hop.as_deref(),
                    next_hop_mac.as_deref(),
                    metric,
                )
                .await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Added route",
                "router": router,
                "source": source,
                "destination": destination,
                "next_hop": next_hop,
                "next_hop_mac": next_hop_mac,
                "metric": metric,
            }))
        }
        RouterRouteCommand::Rm {
            router,
            source,
            destination,
        } => {
            router_ops.rm_route(&router, &source, &destination).await?;
            reconcile_all_flows(config, switch_ops, router_ops).await?;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Removed route",
                "router": router,
                "source": source,
                "destination": destination,
            }))
        }
        RouterRouteCommand::Ls { router } => {
            let routes = router_ops.list_routes(&router)?;
            Ok(router_list_routes_response(&routes))
        }
    }
}

fn interface_list_response(interfaces: Vec<hull::database::Interface>) -> serde_json::Value {
    trace!(count = interfaces.len(), "building interface list response");
    serde_json::Value::Array(
        interfaces
            .into_iter()
            .map(|interface| serde_json::json!({ "name": interface.name, "mac": interface.mac }))
            .collect(),
    )
}

fn switch_list_response(
    switches: Vec<hull::database::Switch>,
    storage: &Database,
) -> Result<serde_json::Value> {
    trace!(count = switches.len(), "building switch list response");
    let mut output_list = Vec::new();
    for switch in switches {
        let ports = storage.get_switch_ports_for_switch(&switch.name)?;
        output_list.push(serde_json::json!({
            "name": switch.name,
            "ip": switch.ip,
            "mask": switch.mask,
            "ports": ports.into_iter().map(|port| serde_json::json!({
                "name": port.name,
                "interface_name": port.interface_name,
                "ip": port.ip,
                "mac": port.mac,
            })).collect::<Vec<_>>(),
        }));
    }

    Ok(serde_json::Value::Array(output_list))
}

fn switch_port_list_response(ports: Vec<hull::database::SwitchPort>) -> serde_json::Value {
    trace!(count = ports.len(), "building switch port list response");
    serde_json::Value::Array(
        ports
            .into_iter()
            .map(|port| {
                serde_json::json!({
                    "name": port.name,
                    "switch_name": port.switch_name,
                    "interface_name": port.interface_name,
                    "ip": port.ip,
                    "mac": port.mac,
                })
            })
            .collect(),
    )
}

fn router_list_response(
    router_list: Vec<hull::database::Router>,
    router_ops: &routers::RouterOps,
) -> Result<serde_json::Value> {
    trace!(count = router_list.len(), "building router list response");
    let mut output_list = Vec::new();
    for router in router_list {
        let switches = router_ops.list_attached_switches(&router.name)?;
        let routes = router_ops.list_routes(&router.name)?;
        output_list.push(serde_json::json!({
            "name": router.name,
            "switches": switches,
            "routes": router_list_routes(&routes),
            "link": {
                "name": router.link_name,
                "ip": router.link_ip,
                "mac": router.link_mac,
            }
        }));
    }

    Ok(serde_json::Value::Array(output_list))
}

fn router_list_routes_response(routes: &[hull::database::RouterRoute]) -> serde_json::Value {
    trace!(count = routes.len(), "building router routes response");
    serde_json::Value::Array(router_list_routes(routes))
}

fn router_list_routes(routes: &[hull::database::RouterRoute]) -> Vec<serde_json::Value> {
    trace!(count = routes.len(), "serializing router routes");
    routes
        .iter()
        .map(|route| {
            serde_json::json!({
                "source": route.source,
                "destination": route.destination,
                "next_hop": route.next_hop,
                "next_hop_mac": route.next_hop_mac,
                "metric": route.metric,
            })
        })
        .collect()
}

async fn create_infrastructure(config: &Config, ovs: &hull::ovs::BridgeClient) -> Result<()> {
    debug!(bridge = %config.bridge_name, "creating infrastructure");
    ovs.add_bridge(&config.bridge_name).await
}

async fn destroy_infrastructure(
    config: &Config,
    storage: &Database,
    ovs: &hull::ovs::BridgeClient,
) -> Result<()> {
    debug!(bridge = %config.bridge_name, "destroying infrastructure");
    ovs.del_bridge(&config.bridge_name).await?;

    let Ok(interfaces) = storage.list_interfaces() else {
        return Ok(());
    };

    for interface in interfaces {
        let _ = Interface::delete(&interface.name).await;
    }

    Ok(())
}

async fn ensure_infrastructure(config: &Config, ovs: &hull::ovs::BridgeClient) -> Result<()> {
    trace!(bridge = %config.bridge_name, "ensuring infrastructure");
    let bridges = ovs.list_bridges().await?;
    if bridges.iter().any(|l| l == &config.bridge_name) {
        trace!(bridge = %config.bridge_name, "bridge already exists");
        return Ok(());
    }

    debug!(bridge = %config.bridge_name, "bridge missing; creating infrastructure");
    create_infrastructure(config, ovs).await?;

    Ok(())
}

async fn reconcile_all_flows(
    config: &Config,
    switch_ops: &switches::SwitchOps,
    router_ops: &routers::RouterOps,
) -> Result<()> {
    debug!(bridge = %config.bridge_name, "reconciling flows");
    let mut of = hull::of::OF::connect(&config.bridge_name).await?;
    of.remove(None).await?;
    switch_ops.sync().await?;
    router_ops.sync().await?;
    Ok(())
}
