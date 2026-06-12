use anyhow::{Context, Result, anyhow};
use clap::{Arg, ArgAction, Command as ClapCommand};
use hull::protocol::{
    Command, Request, RouterCommand, RouterRouteCommand, SwitchCommand, SwitchPortCommand,
    error_response,
};
use hull::{get_root_path, get_socket_path};
use serde_json::Value;
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;

fn main() {
    run();
}

/// Run the Hull CLI.
pub fn run() {
    if let Err(e) = run_inner() {
        let _ = output(&error_response(format!("{e:#}")));
        std::process::exit(1);
    }
}

fn run_inner() -> Result<()> {
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
        .about("Single-node eBPF network controller")
        .arg_required_else_help(true)
        .subcommand(ClapCommand::new("init").about("Initialize hull project"))
        .subcommand(ClapCommand::new("deinit").about("Deinitialize hull and remove all data"))
        .subcommand(switch_subcommand())
        .subcommand(router_subcommand())
        .subcommand(ClapCommand::new("sync").about("Re-apply dataplane state from the database"))
}

fn request_from_matches(matches: &clap::ArgMatches) -> Result<Request> {
    Ok(Request {
        command: command_from_matches(matches)?,
    })
}

fn command_from_matches(matches: &clap::ArgMatches) -> Result<Command> {
    match matches.subcommand() {
        Some(("init", _)) => Ok(Command::Init),
        Some(("deinit", _)) => Ok(Command::Deinit),
        Some(("switch", sub_m)) => switch_command_from_matches(sub_m),
        Some(("router", sub_m)) => router_command_from_matches(sub_m),
        Some(("sync", _)) => Ok(Command::Sync),
        _ => Err(anyhow!("missing command; run 'hull --help'")),
    }
}

fn switch_command_from_matches(sub_m: &clap::ArgMatches) -> Result<Command> {
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
                    ip: m.get_one::<String>("ip").cloned(),
                    mac: m.get_one::<String>("mac").cloned(),
                },
            ))),
            Some(("rm", m)) => Ok(Command::Switch(SwitchCommand::Port(
                SwitchPortCommand::Rm {
                    switch: required_string(m, "switch", "missing switch name")?,
                    name: required_string(m, "name", "missing port name")?,
                },
            ))),
            _ => Err(anyhow!(
                "missing switch port subcommand; run 'hull switch port --help'"
            )),
        },
        _ => Err(anyhow!(
            "missing switch subcommand; run 'hull switch --help'"
        )),
    }
}

fn router_command_from_matches(sub_m: &clap::ArgMatches) -> Result<Command> {
    match sub_m.subcommand() {
        Some(("ls", _)) => Ok(Command::Router(RouterCommand::Ls)),
        Some(("show", m)) => Ok(Command::Router(RouterCommand::Show {
            name: required_string(m, "name", "missing router name")?,
        })),
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
        Some(("route", route_m)) => router_route_command_from_matches(route_m),
        _ => Err(anyhow!(
            "missing router subcommand; run 'hull router --help'"
        )),
    }
}

fn router_route_command_from_matches(route_m: &clap::ArgMatches) -> Result<Command> {
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
        _ => Err(anyhow!(
            "missing router route subcommand; run 'hull router route --help'"
        )),
    }
}

fn parse_route_tail(route_tail: &[String]) -> Result<(Option<String>, Option<String>, u32)> {
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
    m.get_one::<String>(key)
        .cloned()
        .ok_or_else(|| anyhow!("{msg}"))
}

fn send_request(request: &Request) -> Result<Value> {
    let root_path = get_root_path();
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
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn switch_subcommand() -> ClapCommand {
    ClapCommand::new("switch")
        .about("Manage L2 switches")
        .subcommand_required(true)
        .arg_required_else_help(true)
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
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(ClapCommand::new("ls").about("List all ports"))
                .subcommand(
                    ClapCommand::new("create")
                        .about("Create a new port on a switch")
                        .arg(Arg::new("switch").required(true).help("Switch name"))
                        .arg(Arg::new("name").required(true).help("Port name"))
                        .arg(
                            Arg::new("ip")
                                .long("ip")
                                .value_name("IPV4")
                                .help("Static port IPv4 (e.g., 10.0.0.10)"),
                        )
                        .arg(
                            Arg::new("mac")
                                .long("mac")
                                .value_name("MAC")
                                .help("Static port MAC (e.g., 52:54:00:12:34:56)"),
                        ),
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
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(ClapCommand::new("ls").about("List routers"))
        .subcommand(
            ClapCommand::new("show")
                .about("Show router details")
                .arg(Arg::new("name").required(true).help("Router name")),
        )
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
        .subcommand(router_route_subcommand())
}

fn router_route_subcommand() -> ClapCommand {
    ClapCommand::new("route")
        .about("Manage router routing table")
        .subcommand_required(true)
        .arg_required_else_help(true)
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
                        .help("Destination CIDR"),
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
