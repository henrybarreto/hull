use clap::{Arg, Command};

/// Build the CLI command definition.
pub fn build_cli() -> Command {
    Command::new("hull")
        .version("0.1.0")
        .author("Henry Barreto <me@henrybarreto.dev>")
        .about("Simple and lean ovs network mangement")
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("FILE")
                .help("Sets a custom config file")
                .value_parser(clap::value_parser!(std::path::PathBuf)),
        )
        .subcommand(Command::new("init").about("Initialize hull project"))
        .subcommand(Command::new("deinit").about("Deinitialize hull and remove all data"))
        .subcommand(
            Command::new("interface")
                .about("Manage network interfaces")
                .subcommand(Command::new("ls").about("List interfaces"))
                .subcommand(
                    Command::new("create").about("Create a new interface").arg(
                        Arg::new("name")
                            .required(true)
                            .help("Interface name (e.g. tap0)"),
                    ),
                )
                .subcommand(
                    Command::new("rm")
                        .about("Remove an interface")
                        .arg(Arg::new("name").required(true).help("Interface name")),
                ),
        )
        .subcommand(
            Command::new("switch")
                .about("Manage L2 switches")
                .subcommand(Command::new("ls").about("List switches"))
                .subcommand(
                    Command::new("create")
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
                    Command::new("rm")
                        .about("Remove a switch")
                        .arg(Arg::new("name").required(true).help("Switch name")),
                )
                .subcommand(
                    Command::new("port")
                        .about("Manage switch ports")
                        .subcommand(Command::new("ls").about("List all ports"))
                        .subcommand(
                            Command::new("create")
                                .about("Create a new port on a switch")
                                .arg(Arg::new("switch").required(true).help("Switch name"))
                                .arg(Arg::new("name").required(true).help("Port name"))
                                .arg(Arg::new("interface").required(true).help("Interface name")),
                        )
                        .subcommand(
                            Command::new("rm")
                                .about("Remove a port")
                                .arg(Arg::new("switch").required(true).help("Switch name"))
                                .arg(Arg::new("name").required(true).help("Port name")),
                        ),
                ),
        )
        .subcommand(
            Command::new("router")
                .about("Manage L3 routers")
                .subcommand(Command::new("ls").about("List routers"))
                .subcommand(
                    Command::new("create")
                        .about("Create a new router")
                        .arg(Arg::new("name").required(true).help("Router name")),
                )
                .subcommand(
                    Command::new("rm")
                        .about("Remove a router")
                        .arg(Arg::new("name").required(true).help("Router name")),
                )
                .subcommand(
                    Command::new("attach")
                        .about("Attach a switch to a router")
                        .arg(Arg::new("router").required(true).help("Router name"))
                        .arg(Arg::new("switch").required(true).help("Switch name")),
                )
                .subcommand(
                    Command::new("detach")
                        .about("Detach a switch from a router")
                        .arg(Arg::new("router").required(true).help("Router name"))
                        .arg(Arg::new("switch").required(true).help("Switch name")),
                )
                .subcommand(
                    Command::new("link")
                        .about("Manage router uplink")
                        .subcommand(
                            Command::new("set")
                                .about("Set router link")
                                .arg(Arg::new("router").required(true).help("Router name"))
                                .arg(Arg::new("link").required(true).help("Interface name"))
                                .arg(Arg::new("ip").required(true).help("IP address"))
                                .arg(Arg::new("mac").required(true).help("MAC address")),
                        )
                        .subcommand(
                            Command::new("unset")
                                .about("Unset router uplink interface")
                                .arg(Arg::new("router").required(true).help("Router name")),
                        ),
                )
                .subcommand(
                    Command::new("route")
                        .about("Manage router routing table")
                        .subcommand(
                            Command::new("add")
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
                            Command::new("rm")
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
                            Command::new("ls")
                                .about("List routes for a router")
                                .arg(Arg::new("router").required(true).help("Router name")),
                        ),
                ),
        )
        .subcommand(
            Command::new("sync").about("Remove all OVS flows and re-apply from database state"),
        )
}
