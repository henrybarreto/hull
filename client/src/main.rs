use anyhow::{Context, Result, anyhow};
use clap::{Arg, ArgAction, Command as ClapCommand};
use hull::config::{Config, get_root_path, get_socket_path};
use hull::protocol::{
    Command, InterfaceCommand, Request, RouterCommand, RouterLinkCommand, RouterRouteCommand,
    SwitchCommand, SwitchPortCommand, error_response,
};
use serde_json::Value;
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use tracing::{debug, trace};
use tracing_subscriber::filter::LevelFilter;

fn main() {
    run();
}

/// Run the Hull CLI.
pub fn run() {
    init_logging();
    if let Err(e) = run_inner() {
        let _ = output(&error_response(format!("{e:#}")));
        std::process::exit(1);
    }
}

fn init_logging() {
    let subscriber = tracing_subscriber::fmt()
        .with_timer(tracing_subscriber::fmt::time::SystemTime)
        .with_target(false)
        .with_max_level(LevelFilter::OFF)
        .with_writer(std::io::stderr)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn run_inner() -> Result<()> {
    debug!("starting hull client");
    let mut cli = build_cli();
    let matches = cli.clone().get_matches();

    if matches.subcommand_name().is_none() {
        cli.print_long_help()?;
        let mut stdout = io::stdout().lock();
        writeln!(stdout)?;
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

/// Build the Hull CLI definition.
pub fn build_cli() -> ClapCommand {
    ClapCommand::new("hull")
        .version("0.1.0")
        .author("Henry Barreto <me@henrybarreto.dev>")
        .about("Simple and lean ovs network mangement")
        .arg_required_else_help(true)
        .arg(config_arg())
        .subcommand(ClapCommand::new("init").about("Initialize hull project"))
        .subcommand(ClapCommand::new("deinit").about("Deinitialize hull and remove all data"))
        .subcommand(interface_subcommand())
        .subcommand(switch_subcommand())
        .subcommand(router_subcommand())
        .subcommand(
            ClapCommand::new("sync").about("Remove all OVS flows and re-apply from database state"),
        )
}

fn request_from_matches(matches: &clap::ArgMatches) -> Result<Request> {
    let config = matches.get_one::<PathBuf>("config").cloned();
    let command = command_from_matches(matches)?;

    let bridge_name = matches
        .subcommand_name()
        .filter(|name| *name == "init")
        .map(|_| Config::default().bridge_name);

    Ok(Request {
        config,
        bridge_name,
        command,
    })
}

fn command_from_matches(matches: &clap::ArgMatches) -> Result<Command> {
    trace!("resolving cli command");
    match matches.subcommand() {
        Some(("init", _)) => Ok(Command::Init),
        Some(("deinit", _)) => Ok(Command::Deinit),
        Some(("interface", sub_m)) => interface_command_from_matches(sub_m),
        Some(("switch", sub_m)) => switch_command_from_matches(sub_m),
        Some(("router", sub_m)) => router_command_from_matches(sub_m),
        Some(("sync", _)) => Ok(Command::Sync),
        _ => unreachable!(),
    }
}

fn interface_command_from_matches(sub_m: &clap::ArgMatches) -> Result<Command> {
    trace!("resolving interface subcommand");
    match sub_m.subcommand() {
        Some(("ls", _)) => Ok(Command::Interface(InterfaceCommand::Ls)),
        Some(("create", m)) => Ok(Command::Interface(InterfaceCommand::Create {
            name: required_string(m, "name", "missing interface name")?,
            mac: optional_string(m, "mac"),
        })),
        Some(("rm", m)) => Ok(Command::Interface(InterfaceCommand::Rm {
            name: required_string(m, "name", "missing interface name")?,
        })),
        _ => unreachable!(),
    }
}

fn switch_command_from_matches(sub_m: &clap::ArgMatches) -> Result<Command> {
    trace!("resolving switch subcommand");
    match sub_m.subcommand() {
        Some(("ls", _)) => Ok(Command::Switch(SwitchCommand::Ls)),
        Some(("create", m)) => Ok(Command::Switch(SwitchCommand::Create {
            name: required_string(m, "name", "missing switch name")?,
            ip: required_string(m, "ip", "missing switch ip")?,
            mask: required_string(m, "mask", "missing switch mask")?
                .parse::<u8>()
                .context("failed to parse switch mask")?,
        })),
        Some(("rm", m)) => Ok(Command::Switch(SwitchCommand::Rm {
            name: required_string(m, "name", "missing switch name")?,
        })),
        Some(("port", port_m)) => match port_m.subcommand() {
            Some(("ls", _)) => Ok(Command::Switch(SwitchCommand::Port(SwitchPortCommand::Ls))),
            Some(("create", m)) => Ok(Command::Switch(SwitchCommand::Port(
                SwitchPortCommand::Create {
                    switch: required_string(m, "switch", "missing switch name")?,
                    name: required_string(m, "name", "missing port name")?,
                    interface: required_string(m, "interface", "missing interface name")?,
                },
            ))),
            Some(("rm", m)) => Ok(Command::Switch(SwitchCommand::Port(
                SwitchPortCommand::Rm {
                    switch: required_string(m, "switch", "missing switch name")?,
                    name: required_string(m, "name", "missing port name")?,
                },
            ))),
            _ => unreachable!(),
        },
        _ => unreachable!(),
    }
}

fn router_command_from_matches(sub_m: &clap::ArgMatches) -> Result<Command> {
    trace!("resolving router subcommand");
    match sub_m.subcommand() {
        Some(("ls", _)) => Ok(Command::Router(RouterCommand::Ls)),
        Some(("create", m)) => Ok(Command::Router(RouterCommand::Create {
            name: required_string(m, "name", "missing router name")?,
        })),
        Some(("rm", m)) => Ok(Command::Router(RouterCommand::Rm {
            name: required_string(m, "name", "missing router name")?,
        })),
        Some(("attach", m)) => Ok(Command::Router(RouterCommand::Attach {
            router: required_string(m, "router", "missing router name")?,
            switch: required_string(m, "switch", "missing switch name")?,
        })),
        Some(("detach", m)) => Ok(Command::Router(RouterCommand::Detach {
            router: required_string(m, "router", "missing router name")?,
            switch: required_string(m, "switch", "missing switch name")?,
        })),
        Some(("link", link_m)) => router_link_command_from_matches(link_m),
        Some(("route", route_m)) => router_route_command_from_matches(route_m),
        _ => unreachable!(),
    }
}

fn router_link_command_from_matches(link_m: &clap::ArgMatches) -> Result<Command> {
    trace!("resolving router link subcommand");
    match link_m.subcommand() {
        Some(("set", m)) => Ok(Command::Router(RouterCommand::Link(
            RouterLinkCommand::Set {
                router: required_string(m, "router", "missing router name")?,
                port: required_string(m, "port", "missing port name")?,
                ip: required_string(m, "ip", "missing link ip")?,
                mac: required_string(m, "mac", "missing link mac")?,
            },
        ))),
        Some(("unset", m)) => Ok(Command::Router(RouterCommand::Link(
            RouterLinkCommand::Unset {
                router: required_string(m, "router", "missing router name")?,
            },
        ))),
        _ => unreachable!(),
    }
}

fn router_route_command_from_matches(route_m: &clap::ArgMatches) -> Result<Command> {
    trace!("resolving router route subcommand");
    match route_m.subcommand() {
        Some(("add", m)) => {
            let route_tail: Vec<String> = m
                .get_many::<String>("route_tail")
                .map(|vals| vals.cloned().collect())
                .unwrap_or_default();
            let (next_hop, next_hop_mac, metric) = parse_route_tail(route_tail.as_slice())?;

            Ok(Command::Router(RouterCommand::Route(
                RouterRouteCommand::Add {
                    router: required_string(m, "router", "missing router name")?,
                    source: required_string(m, "source", "missing route source")?,
                    destination: required_string(m, "destination", "missing route destination")?,
                    next_hop,
                    next_hop_mac,
                    metric,
                },
            )))
        }
        Some(("rm", m)) => Ok(Command::Router(RouterCommand::Route(
            RouterRouteCommand::Rm {
                router: required_string(m, "router", "missing router name")?,
                source: required_string(m, "source", "missing route source")?,
                destination: required_string(m, "destination", "missing route destination")?,
            },
        ))),
        Some(("ls", m)) => Ok(Command::Router(RouterCommand::Route(
            RouterRouteCommand::Ls {
                router: required_string(m, "router", "missing router name")?,
            },
        ))),
        _ => unreachable!(),
    }
}

fn parse_route_tail(route_tail: &[String]) -> Result<(Option<String>, Option<String>, u32)> {
    trace!(items = route_tail.len(), "parsing route tail");
    match route_tail {
        [] => Ok((None, None, 0)),
        [metric] => Ok((
            None,
            None,
            metric
                .parse::<u32>()
                .map_err(|_| anyhow!("invalid route metric '{metric}'"))?,
        )),
        [next_hop, next_hop_mac] => Ok((Some(next_hop.clone()), Some(next_hop_mac.clone()), 0)),
        [next_hop, next_hop_mac, metric] => Ok((
            Some(next_hop.clone()),
            Some(next_hop_mac.clone()),
            metric
                .parse::<u32>()
                .map_err(|_| anyhow!("invalid route metric '{metric}'"))?,
        )),
        _ => Err(anyhow!(
            "route add expects either <metric> or <next_hop> <next_hop_mac> [metric]"
        )),
    }
}

fn required_string(m: &clap::ArgMatches, key: &str, msg: &str) -> Result<String> {
    trace!(key = %key, "reading required cli arg");
    m.get_one::<String>(key)
        .cloned()
        .ok_or_else(|| anyhow!("{msg}"))
}

fn optional_string(m: &clap::ArgMatches, key: &str) -> Option<String> {
    trace!(key = %key, "reading optional cli arg");
    m.get_one::<String>(key).cloned()
}

fn send_request(request: &Request) -> Result<Value> {
    let root_path = get_root_path();
    let socket_path = get_socket_path(&root_path);
    trace!(socket_path = %socket_path.display(), "connecting to daemon");
    let mut stream = UnixStream::connect(&socket_path)
        .with_context(|| format!("failed to connect to hulld at '{}'", socket_path.display()))?;

    trace!("sending request to daemon");
    serde_json::to_writer(&mut stream, request)?;
    stream.shutdown(Shutdown::Write)?;

    trace!("waiting for daemon response");
    let response = serde_json::from_reader(stream)?;
    Ok(response)
}

fn is_error_response(value: &Value) -> bool {
    trace!("checking cli response status");
    value
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "error")
}

fn output(value: &Value) -> Result<()> {
    trace!("writing cli output");
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn config_arg() -> Arg {
    Arg::new("config")
        .short('c')
        .long("config")
        .value_name("FILE")
        .help("Sets a custom config file")
        .value_parser(clap::value_parser!(std::path::PathBuf))
}

fn interface_subcommand() -> ClapCommand {
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
                )
                .arg(
                    Arg::new("mac")
                        .long("mac")
                        .required(false)
                        .value_name("MAC")
                        .help("Optional MAC address"),
                ),
        )
        .subcommand(
            ClapCommand::new("rm")
                .about("Remove an interface")
                .arg(Arg::new("name").required(true).help("Interface name")),
        )
}

fn switch_subcommand() -> ClapCommand {
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
        )
}

fn router_subcommand() -> ClapCommand {
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
        .subcommand(router_link_subcommand())
        .subcommand(router_route_subcommand())
}

fn router_link_subcommand() -> ClapCommand {
    ClapCommand::new("link")
        .about("Manage router bridge port")
        .subcommand(
            ClapCommand::new("set")
                .about("Set router bridge port")
                .arg(Arg::new("router").required(true).help("Router name"))
                .arg(Arg::new("port").required(true).help("Bridge port name"))
                .arg(Arg::new("ip").required(true).help("IP address"))
                .arg(Arg::new("mac").required(true).help("MAC address")),
        )
        .subcommand(
            ClapCommand::new("unset")
                .about("Unset router bridge port")
                .arg(Arg::new("router").required(true).help("Router name")),
        )
}

fn router_route_subcommand() -> ClapCommand {
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
                    Arg::new("route_tail")
                        .required(false)
                        .index(4)
                        .num_args(0..=3)
                        .action(ArgAction::Append)
                        .help("Route args: [metric] or [next hop IP next hop MAC [metric]]"),
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
        )
}
