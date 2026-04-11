use crate::config::{Config, get_config_path, get_db_path, get_root_path, get_socket_path};
use crate::database::Database;
use crate::interfaces::Interface;
use crate::protocol::{
    Command, InterfaceCommand, Request, RouterCommand, RouterLinkCommand, RouterRouteCommand,
    SwitchCommand, SwitchPortCommand, error_response,
};
use crate::{interfaces, routers, switches};
use anyhow::{Result, anyhow};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub enum LogFormat {
    Text,
    Json,
}

struct Logger {
    format: LogFormat,
    file: Mutex<fs::File>,
}

impl Logger {
    fn new(format: LogFormat, log_file: &Path) -> Result<Self> {
        if let Some(parent) = log_file.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)?;

        Ok(Self {
            format,
            file: Mutex::new(file),
        })
    }

    fn daemon_start(&self, socket_path: &std::path::Path) {
        self.emit(
            "info",
            "start",
            Some(socket_path.display().to_string()),
            Some("daemon listening".to_string()),
        );
    }

    fn request_start(&self, op: &str) {
        self.emit(
            "info",
            "start",
            Some(op.to_string()),
            Some(format!("starting {}", op)),
        );
    }

    fn request_ok(&self, op: &str) {
        self.emit(
            "info",
            "done",
            Some(op.to_string()),
            Some(format!("completed {}", op)),
        );
    }

    fn request_err(&self, op: &str, message: &str) {
        self.emit(
            "error",
            "done",
            Some(op.to_string()),
            Some(message.to_string()),
        );
    }

    fn emit(&self, level: &str, event: &str, operation: Option<String>, message: Option<String>) {
        let line = match self.format {
            LogFormat::Text => {
                let mut line = format!("level={} event={}", level, event);
                if let Some(operation) = operation {
                    line.push_str(&format!(" operation={}", format_text_value(&operation)));
                }
                if let Some(message) = message {
                    line.push_str(&format!(" message={}", format_text_value(&message)));
                }
                line
            }
            LogFormat::Json => {
                let mut payload = serde_json::json!({
                    "level": level,
                    "event": event,
                });
                if let Some(operation) = operation {
                    payload["operation"] = serde_json::Value::String(operation);
                }
                if let Some(message) = message {
                    payload["message"] = serde_json::Value::String(message);
                }
                payload.to_string()
            }
        };

        eprintln!("{}", line);

        let mut file = self.file.lock().expect("logger mutex poisoned");
        let _ = writeln!(file, "{}", line);
        let _ = file.flush();
    }
}

fn format_text_value(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

pub fn run(log_format: LogFormat, log_file: PathBuf) -> Result<()> {
    if unsafe { libc::getuid() } != 0 {
        return Err(anyhow!("hulld must be run as root"));
    }

    let root_path = get_root_path()?;
    fs::create_dir_all(&root_path)?;

    let socket_path = get_socket_path(&root_path);
    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o666))?;
    let logger = Logger::new(log_format, &log_file)?;
    logger.daemon_start(&socket_path);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream, &logger) {
                    eprintln!("hulld request error: {:#}", e);
                }
            }
            Err(e) => {
                eprintln!("hulld socket error: {}", e);
            }
        }
    }

    Ok(())
}

fn handle_connection(mut stream: UnixStream, logger: &Logger) -> Result<()> {
    let request: Request = serde_json::from_reader(&stream)?;
    let op = operation_name(&request.command);
    logger.request_start(op);
    let response = match handle_request(request) {
        Ok(value) => {
            logger.request_ok(op);
            value
        }
        Err(e) => {
            logger.request_err(op, &format!("{:#}", e));
            error_response(format!("{:#}", e))
        }
    };

    serde_json::to_writer_pretty(&mut stream, &response)?;
    stream.shutdown(Shutdown::Write)?;
    Ok(())
}

fn operation_name(command: &Command) -> &'static str {
    match command {
        Command::Init => "init",
        Command::Deinit => "deinit",
        Command::Interface(InterfaceCommand::Ls) => "interface.ls",
        Command::Interface(InterfaceCommand::Create { .. }) => "interface.create",
        Command::Interface(InterfaceCommand::Rm { .. }) => "interface.rm",
        Command::Switch(SwitchCommand::Ls) => "switch.ls",
        Command::Switch(SwitchCommand::Create { .. }) => "switch.create",
        Command::Switch(SwitchCommand::Rm { .. }) => "switch.rm",
        Command::Switch(SwitchCommand::Port(SwitchPortCommand::Ls)) => "switch.port.ls",
        Command::Switch(SwitchCommand::Port(SwitchPortCommand::Create { .. })) => {
            "switch.port.create"
        }
        Command::Switch(SwitchCommand::Port(SwitchPortCommand::Rm { .. })) => "switch.port.rm",
        Command::Router(RouterCommand::Ls) => "router.ls",
        Command::Router(RouterCommand::Create { .. }) => "router.create",
        Command::Router(RouterCommand::Rm { .. }) => "router.rm",
        Command::Router(RouterCommand::Attach { .. }) => "router.attach",
        Command::Router(RouterCommand::Detach { .. }) => "router.detach",
        Command::Router(RouterCommand::Link(_)) => "router.link",
        Command::Router(RouterCommand::Route(_)) => "router.route",
        Command::Sync => "sync",
    }
}

fn handle_request(request: Request) -> Result<serde_json::Value> {
    let Request { config, command } = request;
    let root_path = get_root_path()?;
    let config_path = get_config_path(&root_path, config);
    let db_path = get_db_path(&root_path);
    let is_hull_initialized = root_path.exists() && db_path.exists();

    match command {
        Command::Init => {
            if db_path.exists() {
                return Err(anyhow!(
                    "Hull is already initialized at '{}'. Please run `hull deinit` first.",
                    root_path.display()
                ));
            }

            fs::create_dir_all(&root_path)?;

            let config = Config::default();
            config.save(&config_path)?;

            let storage = Database::new(db_path.clone());
            storage.init()?;
            create_infrastructure(&config)?;

            Ok(serde_json::json!({
                "status": "success",
                "message": "Hull initialized",
            }))
        }
        Command::Deinit => {
            if !db_path.exists() {
                return Err(anyhow!(
                    "Hull is not initialized. No directory found at '{}'.",
                    root_path.display()
                ));
            }

            let storage = Database::new(db_path.clone());
            let config = Config::load(&config_path)?;
            destroy_infrastructure(&config, &storage)?;

            let _ = fs::remove_file(&db_path);
            let _ = fs::remove_file(&config_path);

            Ok(serde_json::json!({
                "status": "success",
                "message": "Hull deinitialized and state removed",
            }))
        }
        other => {
            if !is_hull_initialized {
                return Err(anyhow!("Hull is not initialized. Please run `hull init`."));
            }

            let storage = Arc::new(Database::new(db_path));
            let config = Arc::new(Config::load(&config_path)?);
            let switch_ops = Arc::new(switches::SwitchOps::new(storage.clone(), config.clone()));
            let router_ops = Arc::new(routers::RouterOps::new(storage.clone(), config.clone()));
            let interface_ops = Arc::new(interfaces::InterfaceOps::new(storage.clone()));

            ensure_infrastructure(&config)?;

            match other {
                Command::Interface(subcommand) => match subcommand {
                    InterfaceCommand::Ls => {
                        let interfaces = interface_ops.list()?;
                        let output_list: Vec<_> = interfaces
                            .iter()
                            .map(|i| serde_json::json!({ "name": i.name }))
                            .collect();
                        Ok(serde_json::Value::Array(output_list))
                    }
                    InterfaceCommand::Create { name } => {
                        interface_ops.create(&name)?;
                        Ok(serde_json::json!({
                            "status": "success",
                            "message": "Created interface",
                            "name": name,
                        }))
                    }
                    InterfaceCommand::Rm { name } => {
                        interface_ops.remove(&name)?;
                        Ok(serde_json::json!({
                            "status": "success",
                            "message": "Removed interface",
                            "name": name,
                        }))
                    }
                },
                Command::Switch(subcommand) => match subcommand {
                    SwitchCommand::Ls => {
                        let switches = switch_ops.list()?;
                        let mut output_list = Vec::new();
                        for s in switches {
                            let ports = storage.get_switch_ports_for_switch(&s.name)?;
                            let ports_data: Vec<_> = ports
                                .iter()
                                .map(|p| {
                                    serde_json::json!({
                                        "name": p.name,
                                        "ip": p.ip,
                                        "mac": p.mac,
                                        "interface": p.interface_name
                                    })
                                })
                                .collect();
                            output_list.push(serde_json::json!({
                                "name": s.name,
                                "ip": s.ip,
                                "mask": s.mask,
                                "ports": ports_data,
                            }));
                        }
                        Ok(serde_json::Value::Array(output_list))
                    }
                    SwitchCommand::Create { name, ip, mask } => {
                        switch_ops.create(&name, &ip, mask)?;
                        Ok(serde_json::json!({
                            "status": "success",
                            "message": "Created switch",
                            "name": name,
                            "ip": ip,
                            "mask": mask,
                        }))
                    }
                    SwitchCommand::Rm { name } => {
                        switch_ops.remove(&name)?;
                        Ok(serde_json::json!({
                            "status": "success",
                            "message": "Removed switch",
                            "name": name,
                        }))
                    }
                    SwitchCommand::Port(subcommand) => match subcommand {
                        SwitchPortCommand::Ls => {
                            let ports = switch_ops.list_switch_ports()?;
                            let output_list: Vec<_> = ports
                                .iter()
                                .map(|p| {
                                    serde_json::json!({
                                        "name": p.name,
                                        "switch": p.switch_name,
                                        "interface": p.interface_name,
                                        "ip": p.ip,
                                        "mac": p.mac
                                    })
                                })
                                .collect();
                            Ok(serde_json::Value::Array(output_list))
                        }
                        SwitchPortCommand::Create {
                            switch,
                            name,
                            interface,
                        } => {
                            switch_ops.create_switch_port(&name, &switch, &interface)?;
                            Ok(serde_json::json!({
                                "status": "success",
                                "message": "Created port",
                                "name": name,
                                "switch": switch,
                                "interface": interface,
                            }))
                        }
                        SwitchPortCommand::Rm { switch, name } => {
                            switch_ops.remove_switch_port(&switch, &name)?;
                            Ok(serde_json::json!({
                                "status": "success",
                                "message": "Removed port",
                                "name": name,
                            }))
                        }
                    },
                },
                Command::Router(subcommand) => match subcommand {
                    RouterCommand::Ls => {
                        let routers = router_ops.list()?;
                        let mut output_list = Vec::new();
                        for r in &routers {
                            let switches = router_ops.list_attached_switches(&r.name)?;
                            let routes = router_ops.list_routes(&r.name)?;
                            let routes_list: Vec<_> = routes
                                .iter()
                                .map(|rt| {
                                    serde_json::json!({
                                        "source": rt.source,
                                        "destination": rt.destination,
                                        "next_hop": rt.next_hop,
                                        "metric": rt.metric,
                                    })
                                })
                                .collect();
                            output_list.push(serde_json::json!({
                                "name": r.name,
                                "switches": switches,
                                "routes": routes_list,
                                "link": {
                                    "name": r.link_name,
                                    "ip": r.link_ip,
                                    "mac": r.link_mac,
                                }
                            }));
                        }
                        Ok(serde_json::Value::Array(output_list))
                    }
                    RouterCommand::Create { name } => {
                        router_ops.create(&name)?;
                        Ok(serde_json::json!({
                            "status": "success",
                            "message": "Created router",
                            "name": name,
                        }))
                    }
                    RouterCommand::Rm { name } => {
                        router_ops.remove(&name)?;
                        Ok(serde_json::json!({
                            "status": "success",
                            "message": "Removed router",
                            "name": name,
                        }))
                    }
                    RouterCommand::Attach { router, switch } => {
                        router_ops.attach(&router, &switch)?;
                        Ok(serde_json::json!({
                            "status": "success",
                            "message": "Attached switch to router",
                            "router": router,
                            "switch": switch,
                        }))
                    }
                    RouterCommand::Detach { router, switch } => {
                        router_ops.detach(&router, &switch)?;
                        Ok(serde_json::json!({
                            "status": "success",
                            "message": "Detached switch from router",
                            "router": router,
                            "switch": switch,
                        }))
                    }
                    RouterCommand::Link(subcommand) => match subcommand {
                        RouterLinkCommand::Set {
                            router,
                            link,
                            ip,
                            mac,
                        } => {
                            router_ops.set_link(&router, &link, &ip, &mac)?;
                            Ok(serde_json::json!({
                                "status": "success",
                                "message": "Set router link",
                                "router": router,
                                "link": link,
                                "ip": ip,
                                "mac": mac,
                            }))
                        }
                        RouterLinkCommand::Unset { router } => {
                            router_ops.unset_link(&router)?;
                            Ok(serde_json::json!({
                                "status": "success",
                                "message": "Unset router link",
                                "router": router,
                            }))
                        }
                    },
                    RouterCommand::Route(subcommand) => match subcommand {
                        RouterRouteCommand::Add {
                            router,
                            source,
                            destination,
                            next_hop,
                            metric,
                        } => {
                            router_ops.add_route(
                                &router,
                                &source,
                                &destination,
                                next_hop.as_deref(),
                                metric,
                            )?;
                            Ok(serde_json::json!({
                                "status": "success",
                                "message": "Added route",
                                "router": router,
                                "source": source,
                                "destination": destination,
                                "next_hop": next_hop,
                                "metric": metric,
                            }))
                        }
                        RouterRouteCommand::Rm {
                            router,
                            source,
                            destination,
                        } => {
                            router_ops.rm_route(&router, &source, &destination)?;
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
                            let output_list: Vec<_> = routes
                                .iter()
                                .map(|r| {
                                    serde_json::json!({
                                        "source": r.source,
                                        "destination": r.destination,
                                        "next_hop": r.next_hop,
                                        "metric": r.metric,
                                    })
                                })
                                .collect();
                            Ok(serde_json::Value::Array(output_list))
                        }
                    },
                },
                Command::Sync => {
                    interface_ops.sync()?;
                    switch_ops.sync()?;
                    router_ops.sync()?;
                    Ok(serde_json::json!({
                        "status": "success",
                        "message": "All flows synced from database state",
                    }))
                }
                Command::Init | Command::Deinit => unreachable!(),
            }
        }
    }
}

fn create_infrastructure(config: &Config) -> Result<()> {
    let _ = std::process::Command::new("ovs-vsctl")
        .args(["add-br", &config.bridge_name])
        .status()?;

    Ok(())
}

fn destroy_infrastructure(config: &Config, storage: &Database) -> Result<()> {
    let _ = std::process::Command::new("ovs-vsctl")
        .args(["del-br", &config.bridge_name])
        .status()?;

    if let Ok(interfaces) = storage.list_interfaces() {
        for interface in interfaces {
            let _ = Interface::delete(&interface.name);
        }
    }

    Ok(())
}

fn ensure_infrastructure(config: &Config) -> Result<()> {
    let bridges = switches::ovs_vsctl(&["list-br"])?;
    if !bridges.lines().any(|l| l == config.bridge_name) {
        create_infrastructure(config)?;
    }

    Ok(())
}
