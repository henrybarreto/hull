use clap::{Arg, Command};
use std::path::PathBuf;

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
        .map(|s| s.as_str())
        .unwrap_or("text")
    {
        "json" => hull::daemon::LogFormat::Json,
        _ => hull::daemon::LogFormat::Text,
    };

    let log_file = matches
        .get_one::<PathBuf>("log-file")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("/var/logs/hull/hulld.log"));

    if let Err(e) = hull::daemon::run(log_format, log_file) {
        eprintln!("hulld failed: {:#}", e);
        std::process::exit(1);
    }
}
