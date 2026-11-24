use crate::Cli;
use clap::{CommandFactory, ValueEnum};
use clap_complete::{CompletionCandidate, engine};
use std::{collections::BTreeMap, ffi::OsString, io::Write, process::ExitCode, time::Duration};
use voloo::{client::Client, daemon, protocol::Uid};

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
    words.push(prefix);

    let directory = std::env::current_dir().ok();
    if let Ok(candidates) =
        engine::complete(&mut Cli::command(), words, index, directory.as_deref())
    {
        let mut out = std::io::stdout().lock();
        for candidate in candidates {
            if let Some(value) = candidate.get_value().to_str()
                && !value.chars().any(char::is_control)
                && writeln!(out, "{value}").is_err()
            {
                break;
            }
        }
    }
    ExitCode::SUCCESS
}

pub(crate) fn devices() -> Vec<CompletionCandidate> {
    let Some(info) = daemon::read_info() else {
        return Vec::new();
    };
    // One short, authenticated snapshot read. Tab must never start or replace the daemon.
    let client = Client {
        info,
        source: "cli",
    };
    let Ok((200, rows)) = client.send("GET", "/v1/devices", None, Duration::from_millis(100))
    else {
        return Vec::new();
    };
    let Some(rows) = rows.as_array() else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    let mut names = BTreeMap::new();
    for row in rows {
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
