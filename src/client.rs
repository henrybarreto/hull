use crate::config::{Config, get_config_path, get_root_path, get_socket_path};
use crate::protocol::{
    Command, InterfaceCommand, Request, RouterCommand, RouterLinkCommand, RouterRouteCommand,
    SwitchCommand, SwitchPortCommand, error_response,
};
use anyhow::{Context, Result, anyhow};
use clap::{Arg, Command as ClapCommand};
use serde_json::Value;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub fn build_cli() -> ClapCommand {
    ClapCommand::new("hull")
        .version("0.1.0")
        .author("Henry Barreto <me@henrybarreto.dev>")
        .about("Simple and lean ovs network mangement")
        .arg_required_else_help(true)
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("FILE")
                .help("Sets a custom config file")
                .value_parser(clap::value_parser!(std::path::PathBuf)),
        )
        .subcommand(ClapCommand::new("init").about("Initialize hull project"))
        .subcommand(ClapCommand::new("deinit").about("Deinitialize hull and remove all data"))
        .subcommand(
            ClapCommand::new("interface")
                .about("Manage network interfaces")
                .subcommand(ClapCommand::new("ls").about("List interfaces"))
                .subcommand(
                    ClapCommand::new("create")
                        .about("Create a new interface")
                        .arg(
                            Arg::new("name")
                                .required(true)
                                .help("Interface name (e.g. tap0)"),
                        ),
                )
                .subcommand(
                    ClapCommand::new("rm")
                        .about("Remove an interface")
                        .arg(Arg::new("name").required(true).help("Interface name")),
                ),
        )
        .subcommand(
            ClapCommand::new("switch")
                .about("Manage L2 switches")
                .subcommand(ClapCommand::new("ls").about("List switches"))
                .subcommand(
                    ClapCommand::new("create")
                        .about("Create a new switch")
                        .arg(Arg::new("name").required(true).help("Switch name"))
                        .arg(
                            Arg::new("ip")
                                .required(true)
                                .help("Switch network IP (e.g., 10.0.0.0)"),
                        )
                        .arg(
                            Arg::new("mask")
                                .required(true)
                                .help("Switch network mask (e.g., 24)"),
                        ),
                )
                .subcommand(
                    ClapCommand::new("rm")
                        .about("Remove a switch")
                        .arg(Arg::new("name").required(true).help("Switch name")),
                )
                .subcommand(
                    ClapCommand::new("port")
                        .about("Manage switch ports")
                        .subcommand(ClapCommand::new("ls").about("List all ports"))
                        .subcommand(
                            ClapCommand::new("create")
                                .about("Create a new port on a switch")
                                .arg(Arg::new("switch").required(true).help("Switch name"))
                                .arg(Arg::new("name").required(true).help("Port name"))
                                .arg(Arg::new("interface").required(true).help("Interface name")),
                        )
                        .subcommand(
                            ClapCommand::new("rm")
                                .about("Remove a port")
                                .arg(Arg::new("switch").required(true).help("Switch name"))
                                .arg(Arg::new("name").required(true).help("Port name")),
                        ),
                ),
        )
        .subcommand(
            ClapCommand::new("router")
                .about("Manage L3 routers")
                .subcommand(ClapCommand::new("ls").about("List routers"))
                .subcommand(
                    ClapCommand::new("create")
                        .about("Create a new router")
                        .arg(Arg::new("name").required(true).help("Router name")),
                )
                .subcommand(
                    ClapCommand::new("rm")
                        .about("Remove a router")
                        .arg(Arg::new("name").required(true).help("Router name")),
                )
                .subcommand(
                    ClapCommand::new("attach")
                        .about("Attach a switch to a router")
                        .arg(Arg::new("router").required(true).help("Router name"))
                        .arg(Arg::new("switch").required(true).help("Switch name")),
                )
                .subcommand(
                    ClapCommand::new("detach")
                        .about("Detach a switch from a router")
                        .arg(Arg::new("router").required(true).help("Router name"))
                        .arg(Arg::new("switch").required(true).help("Switch name")),
                )
                .subcommand(
                    ClapCommand::new("link")
                        .about("Manage router uplink")
                        .subcommand(
                            ClapCommand::new("set")
                                .about("Set router link")
                                .arg(Arg::new("router").required(true).help("Router name"))
                                .arg(Arg::new("link").required(true).help("Interface name"))
                                .arg(Arg::new("ip").required(true).help("IP address"))
                                .arg(Arg::new("mac").required(true).help("MAC address")),
                        )
                        .subcommand(
                            ClapCommand::new("unset")
                                .about("Unset router uplink interface")
                                .arg(Arg::new("router").required(true).help("Router name")),
                        ),
                )
                .subcommand(
                    ClapCommand::new("route")
                        .about("Manage router routing table")
                        .subcommand(
                            ClapCommand::new("add")
                                .about("Add a route")
                                .arg(Arg::new("router").required(true).help("Router name"))
                                .arg(
                                    Arg::new("source")
                                        .required(true)
                                        .help("Source CIDR (e.g. 10.0.0.0/24)"),
                                )
                                .arg(
                                    Arg::new("destination")
                                        .required(true)
                                        .help("Destination CIDR (e.g. 0.0.0.0/0)"),
                                )
                                .arg(
                                    Arg::new("next_hop")
                                        .required(false)
                                        .help("Next hop IP (e.g. 192.168.20.1)"),
                                )
                                .arg(
                                    Arg::new("metric")
                                        .required(false)
                                        .help("Route metric (default 0)")
                                        .value_parser(clap::value_parser!(u32)),
                                ),
                        )
                        .subcommand(
                            ClapCommand::new("rm")
                                .about("Remove a route")
                                .arg(Arg::new("router").required(true).help("Router name"))
                                .arg(Arg::new("source").required(true).help("Source CIDR"))
                                .arg(
                                    Arg::new("destination")
                                        .required(true)
                                        .help("Destination CIDR"),
                                ),
                        )
                        .subcommand(
                            ClapCommand::new("ls")
                                .about("List routes for a router")
                                .arg(Arg::new("router").required(true).help("Router name")),
                        ),
                ),
        )
        .subcommand(
            ClapCommand::new("sync").about("Remove all OVS flows and re-apply from database state"),
        )
}

pub fn run() {
    if let Err(e) = run_inner() {
        let _ = output(&error_response(format!("{:#}", e)));
        std::process::exit(1);
    }
}

fn run_inner() -> Result<()> {
    let mut cli = build_cli();
    let matches = cli.clone().get_matches();

    if matches.subcommand_name().is_none() {
        cli.print_long_help()?;
        println!();
        return Ok(());
    }

    let request = request_from_matches(&matches)?;
    let response = send_request(&request)?;

    output(&response)?;

    if is_error_response(&response) {
        std::process::exit(1);
    }

    Ok(())
}

fn request_from_matches(matches: &clap::ArgMatches) -> Result<Request> {
    let config = matches.get_one::<PathBuf>("config").cloned();

    let command = match matches.subcommand() {
        Some(("init", _)) => Command::Init,
        Some(("deinit", _)) => Command::Deinit,
        Some(("interface", sub_m)) => match sub_m.subcommand() {
            Some(("ls", _)) => Command::Interface(InterfaceCommand::Ls),
            Some(("create", m)) => Command::Interface(InterfaceCommand::Create {
                name: m
                    .get_one::<String>("name")
                    .ok_or_else(|| anyhow!("missing interface name"))?
                    .to_string(),
            }),
            Some(("rm", m)) => Command::Interface(InterfaceCommand::Rm {
                name: m
                    .get_one::<String>("name")
                    .ok_or_else(|| anyhow!("missing interface name"))?
                    .to_string(),
            }),
            _ => unreachable!(),
        },
        Some(("switch", sub_m)) => match sub_m.subcommand() {
            Some(("ls", _)) => Command::Switch(SwitchCommand::Ls),
            Some(("create", m)) => Command::Switch(SwitchCommand::Create {
                name: m
                    .get_one::<String>("name")
                    .ok_or_else(|| anyhow!("missing switch name"))?
                    .to_string(),
                ip: m
                    .get_one::<String>("ip")
                    .ok_or_else(|| anyhow!("missing switch ip"))?
                    .to_string(),
                mask: m
                    .get_one::<String>("mask")
                    .ok_or_else(|| anyhow!("missing switch mask"))?
                    .parse::<u8>()
                    .context("failed to parse switch mask")?,
            }),
            Some(("rm", m)) => Command::Switch(SwitchCommand::Rm {
                name: m
                    .get_one::<String>("name")
                    .ok_or_else(|| anyhow!("missing switch name"))?
                    .to_string(),
            }),
            Some(("port", port_m)) => match port_m.subcommand() {
                Some(("ls", _)) => Command::Switch(SwitchCommand::Port(SwitchPortCommand::Ls)),
                Some(("create", m)) => {
                    Command::Switch(SwitchCommand::Port(SwitchPortCommand::Create {
                        switch: m
                            .get_one::<String>("switch")
                            .ok_or_else(|| anyhow!("missing switch name"))?
                            .to_string(),
                        name: m
                            .get_one::<String>("name")
                            .ok_or_else(|| anyhow!("missing port name"))?
                            .to_string(),
                        interface: m
                            .get_one::<String>("interface")
                            .ok_or_else(|| anyhow!("missing interface name"))?
                            .to_string(),
                    }))
                }
                Some(("rm", m)) => Command::Switch(SwitchCommand::Port(SwitchPortCommand::Rm {
                    switch: m
                        .get_one::<String>("switch")
                        .ok_or_else(|| anyhow!("missing switch name"))?
                        .to_string(),
                    name: m
                        .get_one::<String>("name")
                        .ok_or_else(|| anyhow!("missing port name"))?
                        .to_string(),
                })),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        },
        Some(("router", sub_m)) => match sub_m.subcommand() {
            Some(("ls", _)) => Command::Router(RouterCommand::Ls),
            Some(("create", m)) => Command::Router(RouterCommand::Create {
                name: m
                    .get_one::<String>("name")
                    .ok_or_else(|| anyhow!("missing router name"))?
                    .to_string(),
            }),
            Some(("rm", m)) => Command::Router(RouterCommand::Rm {
                name: m
                    .get_one::<String>("name")
                    .ok_or_else(|| anyhow!("missing router name"))?
                    .to_string(),
            }),
            Some(("attach", m)) => Command::Router(RouterCommand::Attach {
                router: m
                    .get_one::<String>("router")
                    .ok_or_else(|| anyhow!("missing router name"))?
                    .to_string(),
                switch: m
                    .get_one::<String>("switch")
                    .ok_or_else(|| anyhow!("missing switch name"))?
                    .to_string(),
            }),
            Some(("detach", m)) => Command::Router(RouterCommand::Detach {
                router: m
                    .get_one::<String>("router")
                    .ok_or_else(|| anyhow!("missing router name"))?
                    .to_string(),
                switch: m
                    .get_one::<String>("switch")
                    .ok_or_else(|| anyhow!("missing switch name"))?
                    .to_string(),
            }),
            Some(("link", link_m)) => match link_m.subcommand() {
                Some(("set", m)) => Command::Router(RouterCommand::Link(RouterLinkCommand::Set {
                    router: m
                        .get_one::<String>("router")
                        .ok_or_else(|| anyhow!("missing router name"))?
                        .to_string(),
                    link: m
                        .get_one::<String>("link")
                        .ok_or_else(|| anyhow!("missing link name"))?
                        .to_string(),
                    ip: m
                        .get_one::<String>("ip")
                        .ok_or_else(|| anyhow!("missing link ip"))?
                        .to_string(),
                    mac: m
                        .get_one::<String>("mac")
                        .ok_or_else(|| anyhow!("missing link mac"))?
                        .to_string(),
                })),
                Some(("unset", m)) => {
                    Command::Router(RouterCommand::Link(RouterLinkCommand::Unset {
                        router: m
                            .get_one::<String>("router")
                            .ok_or_else(|| anyhow!("missing router name"))?
                            .to_string(),
                    }))
                }
                _ => unreachable!(),
            },
            Some(("route", route_m)) => match route_m.subcommand() {
                Some(("add", m)) => {
                    Command::Router(RouterCommand::Route(RouterRouteCommand::Add {
                        router: m
                            .get_one::<String>("router")
                            .ok_or_else(|| anyhow!("missing router name"))?
                            .to_string(),
                        source: m
                            .get_one::<String>("source")
                            .ok_or_else(|| anyhow!("missing route source"))?
                            .to_string(),
                        destination: m
                            .get_one::<String>("destination")
                            .ok_or_else(|| anyhow!("missing route destination"))?
                            .to_string(),
                        next_hop: m.get_one::<String>("next_hop").cloned(),
                        metric: m.get_one::<u32>("metric").copied().unwrap_or(0),
                    }))
                }
                Some(("rm", m)) => Command::Router(RouterCommand::Route(RouterRouteCommand::Rm {
                    router: m
                        .get_one::<String>("router")
                        .ok_or_else(|| anyhow!("missing router name"))?
                        .to_string(),
                    source: m
                        .get_one::<String>("source")
                        .ok_or_else(|| anyhow!("missing route source"))?
                        .to_string(),
                    destination: m
                        .get_one::<String>("destination")
                        .ok_or_else(|| anyhow!("missing route destination"))?
                        .to_string(),
                })),
                Some(("ls", m)) => Command::Router(RouterCommand::Route(RouterRouteCommand::Ls {
                    router: m
                        .get_one::<String>("router")
                        .ok_or_else(|| anyhow!("missing router name"))?
                        .to_string(),
                })),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        },
        Some(("sync", _)) => Command::Sync,
        _ => unreachable!(),
    };

    Ok(Request { config, command })
}

fn send_request(request: &Request) -> Result<Value> {
    let root_path = get_root_path()?;
    let config_path = get_config_path(&root_path, request.config.clone());
    let _config = Config::load(&config_path)?;
    let socket_path = get_socket_path(&root_path);
    let mut stream = UnixStream::connect(&socket_path)
        .with_context(|| format!("failed to connect to hulld at '{}'", socket_path.display()))?;

    serde_json::to_writer(&mut stream, request)?;
    stream.shutdown(Shutdown::Write)?;

    let response = serde_json::from_reader(stream)?;
    Ok(response)
}

fn is_error_response(value: &Value) -> bool {
    value
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "error")
}

fn output(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
