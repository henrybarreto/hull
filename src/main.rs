use anyhow::{Result, anyhow};
use hull::config::{Config, get_config_path, get_db_path, get_root_path};
use hull::database::Database;
use hull::{interfaces, routers, switches};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

#[derive(serde::Serialize)]
struct SuccessResponse<T: serde::Serialize = ()> {
    status: &'static str,
    message: String,
    #[serde(flatten)]
    extra: T,
}

impl SuccessResponse<()> {
    /// Create a success response with no extra payload.
    fn new(message: &str) -> Self {
        Self {
            status: "success",
            message: message.to_string(),
            extra: (),
        }
    }
}

impl<T: serde::Serialize> SuccessResponse<T> {
    /// Create a success response with a payload.
    fn with_data(message: &str, extra: T) -> Self {
        Self {
            status: "success",
            message: message.to_string(),
            extra,
        }
    }
}

#[derive(serde::Serialize)]
struct ErrorResponse {
    status: &'static str,
    message: String,
}

impl ErrorResponse {
    /// Create an error response with a message.
    fn new(message: String) -> Self {
        Self {
            status: "error",
            message,
        }
    }
}

#[tokio::main]
/// CLI entry point.
async fn main() {
    let matches = hull::cli::build_cli().get_matches();

    if let Err(e) = run(&matches).await {
        let _ = output(&ErrorResponse::new(format!("{:#}", e)));
        std::process::exit(1);
    }
}

/// Execute the CLI command.
async fn run(matches: &clap::ArgMatches) -> Result<()> {
    let root_path = get_root_path()?;

    let config_override = matches.get_one::<PathBuf>("config").cloned();

    let config_path = get_config_path(&root_path, config_override);
    let db_path = get_db_path(&root_path);

    let is_hull_initialized = root_path.exists() && db_path.exists();

    if !is_hull_initialized {
        if let Some((subcommand, _)) = matches.subcommand() {
            if subcommand != "init" {
                let _ = output(&ErrorResponse::new(
                    "Hull is not initialized. Please run `hull init`.".to_string(),
                ));
                std::process::exit(1);
            }
        }
    }

    let storage = Arc::new(Database::new(db_path));

    match matches.subcommand() {
        Some(("init", _)) => {
            if root_path.exists() {
                return Err(anyhow!(
                    "Hull is already initialized at '{}'. Please run `hull deinit` first.",
                    root_path.display()
                ));
            }

            fs::create_dir_all(&root_path)?;

            let config = Config::default();
            config.save(&config_path)?;

            storage.init()?;
            create_infrastructure(&config)?;

            output(&SuccessResponse::new("Hull initialized"))?;

            return Ok(());
        }
        Some(("deinit", _)) => {
            if !root_path.exists() {
                return Err(anyhow!(
                    "Hull is not initialized. No directory found at '{}'.",
                    root_path.display()
                ));
            }

            let config = Config::load(&config_path)?;
            destroy_infrastructure(&config, &storage)?;

            fs::remove_dir_all(&root_path)?;

            output(&SuccessResponse::new(
                "Hull deinitialized and directory removed",
            ))?;

            return Ok(());
        }
        _ => {}
    }

    let config = Arc::new(Config::load(&config_path)?);
    let switch_ops = Arc::new(switches::SwitchOps::new(storage.clone(), config.clone()));
    let router_ops = Arc::new(routers::RouterOps::new(storage.clone(), config.clone()));
    let interface_ops = Arc::new(interfaces::InterfaceOps::new(storage.clone()));

    ensure_infrastructure(&config)?;

    match matches.subcommand() {
        Some(("interface", sub_m)) => match sub_m.subcommand() {
            Some(("ls", _)) => {
                let interfaces = interface_ops.list()?;
                let output_list: Vec<_> = interfaces
                    .iter()
                    .map(|i| serde_json::json!({ "name": i.name }))
                    .collect();
                output(&output_list)?;
            }
            Some(("create", m)) => {
                let name = m.get_one::<String>("name").unwrap();
                interface_ops.create(name)?;
                output(&SuccessResponse::with_data(
                    "Created interface",
                    serde_json::json!({ "name": name }),
                ))?;
            }
            Some(("rm", m)) => {
                let name = m.get_one::<String>("name").unwrap();
                interface_ops.remove(name)?;
                output(&SuccessResponse::with_data(
                    "Removed interface",
                    serde_json::json!({ "name": name }),
                ))?;
            }
            _ => unreachable!(),
        },
        Some(("switch", sub_m)) => match sub_m.subcommand() {
            Some(("ls", _)) => {
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
                output(&output_list)?;
            }
            Some(("create", m)) => {
                let name = m.get_one::<String>("name").unwrap();
                let ip = m.get_one::<String>("ip").unwrap();
                let mask = m.get_one::<String>("mask").unwrap().parse::<u8>()?;
                switch_ops.create(name, ip, mask)?;
                output(&SuccessResponse::with_data(
                    "Created switch",
                    serde_json::json!({ "name": name, "ip": ip, "mask": mask }),
                ))?;
            }
            Some(("rm", m)) => {
                let name = m.get_one::<String>("name").unwrap();
                switch_ops.remove(name)?;
                output(&SuccessResponse::with_data(
                    "Removed switch",
                    serde_json::json!({ "name": name }),
                ))?;
            }
            Some(("port", port_m)) => match port_m.subcommand() {
                Some(("ls", _)) => {
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
                    output(&output_list)?;
                }
                Some(("create", m)) => {
                    let switch = m.get_one::<String>("switch").unwrap();
                    let name = m.get_one::<String>("name").unwrap();
                    let interface = m.get_one::<String>("interface").unwrap();
                    switch_ops.create_switch_port(name, switch, interface)?;
                    output(&SuccessResponse::with_data(
                        "Created port",
                        serde_json::json!({ "name": name, "switch": switch, "interface": interface }),
                    ))?;
                }
                Some(("rm", m)) => {
                    let switch = m.get_one::<String>("switch").unwrap();
                    let name = m.get_one::<String>("name").unwrap();
                    switch_ops.remove_switch_port(name, switch)?;
                    output(&SuccessResponse::with_data(
                        "Removed port",
                        serde_json::json!({ "name": name }),
                    ))?;
                }
                _ => unreachable!(),
            },
            _ => unreachable!(),
        },
        Some(("router", sub_m)) => match sub_m.subcommand() {
            Some(("ls", _)) => {
                let routers = router_ops.list()?;
                let mut output_list = Vec::new();
                for r in routers {
                    let switches = router_ops.list_attached_switches(&r.name)?;
                    output_list.push(serde_json::json!({
                        "name": r.name,
                        "switches": switches,
                        "link": {
                            "name": r.link_name,
                            "ip": r.link_ip,
                            "mac": r.link_mac,
                        }
                    }));
                }

                output(&output_list)?;
            }
            Some(("create", m)) => {
                let name = m.get_one::<String>("name").unwrap();
                router_ops.create(name)?;
                output(&SuccessResponse::with_data(
                    "Created router",
                    serde_json::json!({ "name": name }),
                ))?;
            }
            Some(("rm", m)) => {
                let name = m.get_one::<String>("name").unwrap();
                router_ops.remove(name)?;
                output(&SuccessResponse::with_data(
                    "Removed router",
                    serde_json::json!({ "name": name }),
                ))?;
            }
            Some(("attach", m)) => {
                let router = m.get_one::<String>("router").unwrap();
                let switch = m.get_one::<String>("switch").unwrap();
                router_ops.attach(router, switch)?;
                output(&SuccessResponse::with_data(
                    "Attached switch to router",
                    serde_json::json!({ "router": router, "switch": switch }),
                ))?;
            }
            Some(("detach", m)) => {
                let router = m.get_one::<String>("router").unwrap();
                let switch = m.get_one::<String>("switch").unwrap();
                router_ops.detach(router, switch)?;
                output(&SuccessResponse::with_data(
                    "Detached switch from router",
                    serde_json::json!({ "router": router, "switch": switch }),
                ))?;
            }
            Some(("link", link_m)) => match link_m.subcommand() {
                Some(("set", m)) => {
                    let router = m.get_one::<String>("router").unwrap();
                    let link = m.get_one::<String>("link").unwrap();
                    let ip = m.get_one::<String>("ip").unwrap();
                    let mac = m.get_one::<String>("mac").unwrap();
                    router_ops.set_link(router, link, ip, mac)?;
                    output(&SuccessResponse::with_data(
                        "Set router link",
                        serde_json::json!({ "router": router, "link": link, "ip": ip, "mac": mac }),
                    ))?;
                }
                Some(("unset", m)) => {
                    let router = m.get_one::<String>("router").unwrap();
                    router_ops.unset_link(router)?;
                    output(&SuccessResponse::with_data(
                        "Unset router link",
                        serde_json::json!({ "router": router }),
                    ))?;
                }
                _ => unreachable!(),
            },

            _ => unreachable!(),
        },
        Some(("sync", _)) => {
            interface_ops.sync()?;

            // Command::new("ovs-ofctl")
            //     .args(["del-flows", &config.bridge_name])
            //     .status()?;

            switch_ops.sync()?;
            router_ops.sync()?;

            output(&SuccessResponse::new(
                "All flows synced from database state",
            ))?;
        }
        _ => unreachable!(),
    }

    Ok(())
}

/// Create the OVS bridge infrastructure.
fn create_infrastructure(config: &Config) -> Result<()> {
    let _ = Command::new("ovs-vsctl")
        .args(["add-br", &config.bridge_name])
        .status()?;

    Ok(())
}

/// Remove the OVS bridge and clean up related resources.
fn destroy_infrastructure(config: &Config, storage: &Database) -> Result<()> {
    let _ = Command::new("ovs-vsctl")
        .args(["del-br", &config.bridge_name])
        .status()?;

    if let Ok(interfaces) = storage.list_interfaces() {
        for interface in interfaces {
            let _ = crate::interfaces::Interface::delete(&interface.name);
        }
    }

    Ok(())
}

/// Ensure required infrastructure exists.
fn ensure_infrastructure(config: &Config) -> Result<()> {
    let bridges = switches::ovs_vsctl(&["list-br"])?;
    if !bridges.lines().any(|l| l == config.bridge_name) {
        create_infrastructure(config)?;
    }

    Ok(())
}

/// Serialize a response as JSON and print it.
fn output<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
