use anyhow::{Result, anyhow};
use aya::include_bytes_aligned;
use clap::{Arg, Command};
use hull::cidr::Ipv4Network;
use hull::database::Database;
use hull::ebpf::BridgePlane;
use hull::protocol::{
    Command as HullCommand, Request, RouterCommand, RouterRouteCommand, SwitchCommand,
    SwitchPortCommand, error_response,
};
use hull::switches::SwitchRouterOps;
use hull::{get_db_path, get_root_path, get_socket_path};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{error, info};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::writer::MakeWriterExt;

#[derive(Debug, Clone, Copy)]
pub enum LogFormat {
    Text,
    Json,
}

struct State {
    storage: Arc<Database>,
    switch_router_ops: SwitchRouterOps,
}

impl State {
    fn new(root_path: &Path) -> Result<Self> {
        let db_path = get_db_path(root_path);
        if !db_path.exists() {
            return Err(anyhow!(
                "Hull is not initialized. Please run 'hull init' first."
            ));
        }
        let storage = Arc::new(Database::new(db_path));
        let plane = Arc::new(load_plane()?);
        let switch_router_ops = SwitchRouterOps::new(storage.clone(), plane);
        Ok(Self {
            storage,
            switch_router_ops,
        })
    }

    fn sync_startup(&self) -> Result<()> {
        self.switch_router_ops.sync()
    }
}

fn main() {
    let matches = Command::new("hulld")
        .arg(
            Arg::new("log-format")
                .long("log-format")
                .value_parser(["text", "json"])
                .default_value("text"),
        )
        .arg(
            Arg::new("log-level")
                .long("log-level")
                .value_parser(["error", "warn", "info", "debug", "trace"])
                .default_value("info"),
        )
        .arg(
            Arg::new("log-file")
                .long("log-file")
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
        eprintln!("hulld failed: {e:#}");
        std::process::exit(1);
    }
}

pub fn run(log_format: LogFormat, log_level: LevelFilter, log_file: PathBuf) -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("hulld requires root (CAP_BPF + CAP_NET_ADMIN)");
        std::process::exit(1);
    }

    let root_path = get_root_path();
    fs::create_dir_all(&root_path)?;
    init_logging(log_format, log_level, &log_file)?;

    let state = State::new(&root_path).ok();
    if let Some(ref s) = state {
        s.sync_startup()?;
    }

    let socket_path = get_socket_path(&root_path);
    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o666))?;
    info!(socket = %socket_path.display(), "daemon listening");

    let shared = Arc::new(Mutex::new(state));
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(e) = handle_connection(stream, shared.clone()) {
                    error!("request error: {e:#}");
                }
            }
            Err(e) => error!("listener error: {e}"),
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
    let writer = std::io::stderr.and(std::sync::Mutex::new(file));

    match log_format {
        LogFormat::Text => {
            let subscriber = tracing_subscriber::fmt()
                .with_target(false)
                .with_max_level(log_level)
                .compact()
                .with_writer(writer)
                .finish();
            tracing::subscriber::set_global_default(subscriber)?;
        }
        LogFormat::Json => {
            let subscriber = tracing_subscriber::fmt()
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

fn load_plane() -> Result<BridgePlane> {
    let data = include_bytes_aligned!(env!("HULL_EBPF_OBJECT"));
    BridgePlane::load(data)
}

fn handle_connection(mut stream: UnixStream, state: Arc<Mutex<Option<State>>>) -> Result<()> {
    let mut buffer = Vec::new();
    let mut temp_buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut temp_buf)?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&temp_buf[..n]);
        if serde_json::from_slice::<Request>(&buffer).is_ok() {
            break;
        }
    }

    let request: Request = serde_json::from_slice(&buffer)?;
    info!(command = ?request.command, "received command");
    let response = match handle_request(request, state) {
        Ok(value) => value,
        Err(e) => error_response(format!("{e:#}")),
    };

    let response_bytes = serde_json::to_vec_pretty(&response)?;
    stream.write_all(&response_bytes)?;
    stream.shutdown(Shutdown::Both)?;
    Ok(())
}

fn handle_request(request: Request, state: Arc<Mutex<Option<State>>>) -> Result<serde_json::Value> {
    let root_path = get_root_path();
    let db_path = get_db_path(&root_path);

    match request.command {
        HullCommand::Init => {
            let response = handle_init(root_path.clone(), db_path)?;
            let mut guard = state.lock().map_err(|_| anyhow!("state lock poisoned"))?;
            *guard = Some(State::new(&root_path)?);
            Ok(response)
        }
        HullCommand::Deinit => {
            let response = handle_deinit(db_path)?;
            let mut guard = state.lock().map_err(|_| anyhow!("state lock poisoned"))?;
            *guard = None;
            Ok(response)
        }
        command => {
            let mut guard = state.lock().map_err(|_| anyhow!("state lock poisoned"))?;
            if guard.is_none() {
                if db_path.exists() {
                    *guard = Some(State::new(&root_path)?);
                } else {
                    return Err(anyhow!(
                        "Hull is not initialized. Please run 'hull init' first."
                    ));
                }
            }
            let state_ref = guard
                .as_ref()
                .ok_or_else(|| anyhow!("Hull is not initialized. Please run 'hull init' first."))?;
            handle_initialized(command, state_ref)
        }
    }
}

fn handle_init(root_path: PathBuf, db_path: PathBuf) -> Result<serde_json::Value> {
    if root_path.exists() && db_path.exists() {
        return Err(anyhow!("Hull already initialized"));
    }

    fs::create_dir_all(&root_path)?;
    let db = Database::new(db_path);
    db.reset_schema()?;
    db.init()?;

    Ok(serde_json::json!({"status":"success","message":"Hull initialized"}))
}

fn handle_deinit(db_path: PathBuf) -> Result<serde_json::Value> {
    let db = Database::new(db_path.clone());
    let _ = db.reset_schema();
    let _ = fs::remove_file(db_path);
    Ok(serde_json::json!({"status":"success","message":"Hull deinitialized"}))
}

fn handle_initialized(command: HullCommand, state: &State) -> Result<serde_json::Value> {
    match command {
        HullCommand::Switch(cmd) => handle_switch(cmd, state),
        HullCommand::Router(cmd) => handle_router(cmd, state),
        HullCommand::Sync => {
            state.switch_router_ops.sync()?;
            Ok(serde_json::json!({"status":"success","message":"All state synced from database"}))
        }
        HullCommand::Init | HullCommand::Deinit => unreachable!(),
    }
}

fn handle_switch(cmd: SwitchCommand, state: &State) -> Result<serde_json::Value> {
    match cmd {
        SwitchCommand::Ls => {
            let networks = state.switch_router_ops.list_switches()?;
            let mut switches = Vec::new();
            for n in networks {
                let ports = state
                    .switch_router_ops
                    .list_switch_ports(&n.name)?
                    .into_iter()
                    .map(|ep| {
                        serde_json::json!({
                            "uuid": ep.uuid,
                            "name": ep.name,
                            "tap_name": ep.tap_name,
                            "ip": ep.ip,
                            "mac": ep.mac,
                        })
                    })
                    .collect::<Vec<_>>();
                if let Some(subnet) = state
                    .switch_router_ops
                    .list_subnets(&n.name)?
                    .into_iter()
                    .next()
                {
                    let (ip, mask) = split_cidr(&subnet.cidr)?;
                    switches.push(
                        serde_json::json!({"uuid":n.uuid,"name":n.name,"ip":ip,"mask":mask,"ports":ports}),
                    );
                }
            }
            Ok(serde_json::json!(switches))
        }
        SwitchCommand::Create { name, ip, mask } => {
            let cidr = format!("{ip}/{mask}");
            state.switch_router_ops.create_switch(&name)?;
            state
                .switch_router_ops
                .add_subnet(&name, "default", &cidr, None, None)?;
            state.switch_router_ops.sync()?;
            Ok(serde_json::json!({"status":"success","message":"Created switch","name":name}))
        }
        SwitchCommand::Rm { name } => {
            let existing_ports = state.switch_router_ops.list_switch_ports(&name)?;
            for port in existing_ports {
                state
                    .switch_router_ops
                    .remove_switch_port(&name, &port.name)?;
            }
            state.switch_router_ops.remove_switch(&name)?;
            state.switch_router_ops.sync()?;
            Ok(serde_json::json!({"status":"success","message":"Removed switch","name":name}))
        }
        SwitchCommand::Port(port_cmd) => match port_cmd {
            SwitchPortCommand::Ls => {
                let networks = state.switch_router_ops.list_switches()?;
                let mut ports = Vec::new();
                for n in networks {
                    for ep in state.switch_router_ops.list_switch_ports(&n.name)? {
                        ports.push(serde_json::json!({
                            "uuid": ep.uuid,
                            "name": ep.name,
                            "switch_name": n.name,
                            "tap_name": ep.tap_name,
                            "ip": ep.ip,
                            "mac": ep.mac,
                        }));
                    }
                }
                Ok(serde_json::json!(ports))
            }
            SwitchPortCommand::Create {
                switch,
                name,
                ip,
                mac,
            } => {
                state.switch_router_ops.add_switch_port(
                    &switch,
                    "default",
                    &name,
                    &name,
                    mac.as_deref(),
                    ip.as_deref(),
                )?;
                state.switch_router_ops.sync()?;
                Ok(
                    serde_json::json!({"status":"success","message":"Created switch port","name":name,"switch":switch}),
                )
            }
            SwitchPortCommand::Rm { switch, name } => {
                state.switch_router_ops.remove_switch_port(&switch, &name)?;
                state.switch_router_ops.sync()?;
                Ok(
                    serde_json::json!({"status":"success","message":"Removed switch port","name":name,"switch":switch}),
                )
            }
        },
    }
}

fn handle_router(cmd: RouterCommand, state: &State) -> Result<serde_json::Value> {
    match cmd {
        RouterCommand::Ls => {
            let routers = state
                .storage
                .list_routers()?
                .into_iter()
                .map(|r| serde_json::json!({"name": r.name, "uuid": r.uuid}))
                .collect::<Vec<_>>();
            Ok(serde_json::json!(routers))
        }
        RouterCommand::Show { name } => {
            let router = state.storage.get_router(&name)?;
            let attachments = state.storage.list_router_ports_for_router(&name)?;
            let routes = list_routes_with_learned(state, &name)?;
            let attachments = attachments
                .into_iter()
                .map(|a| {
                    serde_json::json!({
                        "uuid": a.uuid,
                        "switch": a.switch_name
                    })
                })
                .collect::<Vec<_>>();
            let routes = routes
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "uuid": r.uuid,
                        "source": r.source,
                        "destination": r.destination,
                        "next_hop": r.next_hop,
                        "next_hop_mac": r.next_hop_mac,
                        "metric": r.metric,
                        "learned": r.router_name == "__learned__"
                    })
                })
                .collect::<Vec<_>>();
            Ok(serde_json::json!({
                "name": router.name,
                "uuid": router.uuid,
                "attachments": attachments,
                "routes": routes
            }))
        }
        RouterCommand::Create { name } => {
            let router = state.storage.create_router(&name)?;
            Ok(serde_json::json!({"status":"success","message":"Created router","router":router}))
        }
        RouterCommand::Rm { name } => {
            state.storage.remove_router(&name)?;
            Ok(serde_json::json!({"status":"success","message":"Removed router","name":name}))
        }
        RouterCommand::Attach { router, switch } => {
            validate_router_attach_no_cidr_conflict(state, &router, &switch)?;
            state.storage.create_router_port(&router, &switch)?;
            state.switch_router_ops.sync()?;
            Ok(
                serde_json::json!({"status":"success","message":"Attached router to switch","router":router,"switch":switch}),
            )
        }
        RouterCommand::Detach { router, switch } => {
            let attached = state
                .storage
                .list_router_ports_for_router(&router)?
                .into_iter()
                .any(|p| p.switch_name == switch);
            if !attached {
                return Err(anyhow!(
                    "Router '{router}' is not attached to switch '{switch}'"
                ));
            }
            state.storage.remove_router_port(&router, &switch)?;
            state.switch_router_ops.sync()?;
            Ok(
                serde_json::json!({"status":"success","message":"Detached router from switch","router":router,"switch":switch}),
            )
        }
        RouterCommand::Route(route_cmd) => match route_cmd {
            RouterRouteCommand::Add {
                router,
                source,
                destination,
                next_hop,
                next_hop_mac,
                metric,
            } => {
                let route = state.storage.create_router_route(
                    &router,
                    &source,
                    &destination,
                    next_hop.as_deref(),
                    next_hop_mac.as_deref(),
                    metric,
                )?;
                state.switch_router_ops.sync()?;
                Ok(serde_json::json!({"status":"success","message":"Added route","route":route}))
            }
            RouterRouteCommand::Rm {
                router,
                source,
                destination,
            } => {
                state
                    .storage
                    .remove_router_route(&router, &source, &destination)?;
                state.switch_router_ops.sync()?;
                Ok(serde_json::json!({"status":"success","message":"Removed route"}))
            }
            RouterRouteCommand::Ls { router } => {
                Ok(serde_json::json!(list_routes_with_learned(state, &router)?))
            }
        },
    }
}

fn list_routes_with_learned(
    state: &State,
    router: &str,
) -> Result<Vec<hull::database::RouterRoute>> {
    let mut routes = state.storage.list_router_routes(router)?;
    let attachments = state.storage.list_router_ports_for_router(router)?;
    for attachment in attachments {
        for subnet in state
            .switch_router_ops
            .list_subnets(&attachment.switch_name)?
        {
            routes.push(hull::database::RouterRoute {
                uuid: format!("learned:{}:{}", attachment.switch_name, subnet.cidr),
                router_name: "__learned__".to_string(),
                source: subnet.cidr.clone(),
                destination: subnet.cidr,
                next_hop: None,
                next_hop_mac: None,
                metric: 0,
            });
        }
    }
    Ok(routes)
}

fn validate_router_attach_no_cidr_conflict(
    state: &State,
    router_name: &str,
    candidate_switch: &str,
) -> Result<()> {
    let _ = state.storage.get_router(router_name)?;
    let _ = state.switch_router_ops.list_subnets(candidate_switch)?;

    let existing_attachments = state.storage.list_router_ports_for_router(router_name)?;
    let candidate_subnets = state.switch_router_ops.list_subnets(candidate_switch)?;

    for attachment in existing_attachments {
        if attachment.switch_name == candidate_switch {
            return Err(anyhow!(
                "Router '{router_name}' is already attached to switch '{candidate_switch}'"
            ));
        }

        let existing_subnets = state
            .switch_router_ops
            .list_subnets(&attachment.switch_name)?;
        for existing in &existing_subnets {
            let existing_net = parse_ipnetwork(&existing.cidr)?;
            for candidate in &candidate_subnets {
                let candidate_net = parse_ipnetwork(&candidate.cidr)?;
                if cidr_overlap(&existing_net, &candidate_net) {
                    return Err(anyhow!(
                        "CIDR conflict: switch '{}' subnet '{}' overlaps switch '{}' subnet '{}' on router '{}'",
                        candidate_switch,
                        candidate.cidr,
                        attachment.switch_name,
                        existing.cidr,
                        router_name
                    ));
                }
            }
        }
    }

    Ok(())
}

fn parse_ipnetwork(cidr: &str) -> Result<Ipv4Network> {
    cidr.parse::<Ipv4Network>()
        .map_err(|e| anyhow!("invalid cidr '{cidr}': {e}"))
}

fn cidr_overlap(a: &Ipv4Network, b: &Ipv4Network) -> bool {
    a.overlaps(*b)
}

fn split_cidr(cidr: &str) -> Result<(String, u8)> {
    let mut parts = cidr.split('/');
    let ip = parts
        .next()
        .ok_or_else(|| anyhow!("invalid cidr: missing ip"))?
        .to_string();
    let mask = parts
        .next()
        .ok_or_else(|| anyhow!("invalid cidr: missing mask"))?
        .parse::<u8>()
        .map_err(|e| anyhow!("invalid cidr mask: {e}"))?;
    Ok((ip, mask))
}
