use crate::Cli;
use clap::{CommandFactory, ValueEnum, parser::ValueSource};
use clap_complete::{ArgValueCandidates, CompletionCandidate, engine};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io::Write,
    process::ExitCode,
    time::Duration,
};
use voloo::{client::Client, config::Config, daemon, discover, home, observe, protocol::Uid};

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Shell {
    Bash,
    Zsh,
    Fish,
    #[value(name = "powershell", alias = "pwsh")]
    Pwsh,
}

pub(crate) fn register(shell: Shell) -> ExitCode {
    let script = match shell {
        Shell::Bash => include_str!("completion/voloo.bash"),
        Shell::Zsh => include_str!("completion/voloo.zsh"),
        Shell::Fish => include_str!("completion/voloo.fish"),
        Shell::Pwsh => include_str!("completion/voloo.ps1"),
    };
    match std::io::stdout().write_all(script.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

pub(crate) fn suggest(mut words: Vec<OsString>) -> ExitCode {
    let Some(prefix) = words
        .first()
        .and_then(|word| word.to_str())
        .and_then(|word| word.strip_prefix('='))
        .map(OsString::from)
    else {
        return ExitCode::SUCCESS;
    };
    words[0] = "voloo".into();
    let index = words.len();
    let mut command = Cli::command();
    let matches = command
        .clone()
        .ignore_errors(true)
        .try_get_matches_from(&words)
        .ok();
    let update = matches
        .as_ref()
        .and_then(|root| root.subcommand_matches("firmware"))
        .and_then(|firmware| firmware.subcommand_matches("update"));
    if update.is_some_and(|args| args.get_flag("all")) {
        // The completion engine does not parse conflicts between flags and positionals.
        // Leave global options available after --all, but disable both positionals.
        command = command.mut_subcommand("firmware", |firmware| {
            firmware.mut_subcommand("update", |update| {
                update
                    .mut_arg("device", |arg| arg.add(ArgValueCandidates::new(Vec::new)))
                    .mut_arg("file", |arg| arg.add(ArgValueCandidates::new(Vec::new)))
            })
        });
    }
    let mut active = matches.as_ref();
    while let Some((_, subcommand)) = active.and_then(|args| args.subcommand()) {
        active = Some(subcommand);
    }
    words.push(prefix.clone());

    let directory = std::env::current_dir().ok();
    if let Ok(candidates) = engine::complete(&mut command, words, index, directory.as_deref()) {
        let mut out = std::io::stdout().lock();
        for candidate in candidates {
            let Some(mut value) = candidate.get_value().to_str() else {
                continue;
            };
            if let Some(id) = candidate.get_id().and_then(|id| id.strip_prefix("arg::")) {
                if active.is_some_and(|args| {
                    args.try_contains_id(id).unwrap_or(false)
                        && args.value_source(id) == Some(ValueSource::CommandLine)
                }) || (id == "all"
                    && update
                        .is_some_and(|args| args.contains_id("device") || args.contains_id("file")))
                {
                    continue;
                }
                // -h and -V are complete by themselves. The engine treats them as
                // the start of a short-flag cluster and can suggest -hh or -VV.
                match prefix.to_str() {
                    Some("-h") if id == "help" => value = "-h",
                    Some("-V") if id == "version" => value = "-V",
                    Some("-h" | "-V") => continue,
                    _ => {}
                }
            }
            if !value.chars().any(char::is_control) && writeln!(out, "{value}").is_err() {
                break;
            }
        }
    }
    ExitCode::SUCCESS
}

pub(crate) fn devices() -> Vec<CompletionCandidate> {
    let rows = match cached_rows("/v1/devices") {
        Some(rows) if rows.iter().any(|row| row["online"] == true) => rows,
        // USB metadata can provide UIDs even before voloo ls starts the daemon.
        // Nicknames come from the daemon; opening serial ports on Tab would race it.
        _ => {
            let scan_usb = Config::load().is_ok_and(|config| config.scan_usb);
            return serial_ports(scan_usb)
                .into_iter()
                .filter(|port| port.is_voloo)
                .filter_map(|port| port.uid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|uid| uid.to_string().into())
                .collect();
        }
    };

    let mut candidates = Vec::new();
    let mut names = BTreeMap::new();
    for row in &rows {
        let online = row["online"].as_bool() == Some(true);
        if online && let Some(uid) = row["uid"].as_str().and_then(|uid| uid.parse::<Uid>().ok()) {
            candidates.push(CompletionCandidate::new(uid.to_string()).display_order(Some(1)));
        }
        if let Some(name) = row["nickname"].as_str().filter(|name| portable_name(name)) {
            let (count, any_online) = names.entry(name).or_insert((0, false));
            *count += 1;
            *any_online |= online;
        }
    }
    for (name, (count, online)) in names {
        if count == 1 && online {
            candidates.push(CompletionCandidate::new(name).display_order(Some(0)));
        }
    }
    candidates
}

fn cached_rows(path: &str) -> Option<Vec<Value>> {
    // One short authenticated read from a daemon that is already running.
    let client = Client {
        info: daemon::read_info()?,
        source: "cli",
    };
    match client.send("GET", path, None, Duration::from_millis(100)) {
        Ok((200, Value::Array(rows))) => Some(rows),
        _ => None,
    }
}

fn serial_ports(scan_usb: bool) -> Vec<discover::PortInfo> {
    if scan_usb {
        discover::list_ports().unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub(crate) fn ports() -> Vec<CompletionCandidate> {
    let Ok(config) = Config::load() else {
        return Vec::new();
    };
    let detected = serial_ports(config.scan_usb);
    config
        .static_ports
        .into_iter()
        .map(|(path, _)| path)
        .chain(detected.into_iter().map(|port| port.path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(Into::into)
        .collect()
}

pub(crate) fn log_uids() -> Vec<CompletionCandidate> {
    let rows = cached_rows("/v1/devices").unwrap_or_default();
    rows.iter()
        .chain(recent_log("operations").iter())
        .chain(recent_log("events").iter())
        .chain(recent_log("trace").iter())
        .filter_map(|row| row["uid"].as_str()?.parse::<Uid>().ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|uid| uid.to_string().into())
        .collect()
}

pub(crate) fn trace_ids() -> Vec<CompletionCandidate> {
    let cached = cached_rows("/v1/trace?limit=200").unwrap_or_default();
    let on_disk = recent_log("trace");
    let mut seen = BTreeSet::new();
    cached
        .iter()
        .rev()
        .chain(on_disk.iter().rev())
        .filter_map(|row| row["trace_id"].as_str())
        .filter(|id| {
            id.starts_with('t')
                && id[1..].bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
                && seen.insert(*id)
        })
        .take(50)
        .map(Into::into)
        .collect()
}

fn recent_log(stem: &str) -> Vec<Value> {
    home::logs()
        .map(|dir| observe::read_log_tail(&dir, stem))
        .unwrap_or_default()
}

// All supported shells can insert these as one argument (quoting spaces as needed).
// Other nicknames can still be addressed by their UIDs.
fn portable_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && !name.starts_with('-')
        && name.parse::<Uid>().is_err()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
}
