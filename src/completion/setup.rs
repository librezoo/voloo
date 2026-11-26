//! Install a small loader that uses the completion script from the installed Voloo.
mod process;

use super::Shell;
use std::{
    env,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const MARKER: &str = "# >>> voloo tab completion >>>";

const BASH_LOADER: &str = r#"
if [ -n "${BASH_VERSION-}" ] && command -v voloo >/dev/null 2>&1 &&
   ! command -v _voloo_complete >/dev/null 2>&1; then
    case $- in
        *i*) eval "$(command voloo completions bash)" ;;
    esac
fi
"#;

const ZSH_LOADER: &str = r#"
if command -v voloo >/dev/null 2>&1; then
    source <(command voloo completions zsh)
fi
"#;

const FISH_LOADER: &str = r#"
if command -sq voloo
    command voloo completions fish | source
end
"#;

const POWERSHELL_LOADER: &str = r#"
if (![Console]::IsInputRedirected -and ![Console]::IsOutputRedirected -and
    (Get-Command voloo -ErrorAction SilentlyContinue)) {
    & {
        $completion = voloo completions powershell | Out-String
        if ($completion) { Invoke-Expression $completion }
    }
}
"#;

struct Setup {
    shell: Shell,
    paths: Vec<PathBuf>,
}

pub(crate) fn interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal()
}

pub(crate) fn offer() {
    if env::var_os("VOLOO_NO_COMPLETION_PROMPT").is_some_and(|value| value == "1") {
        return;
    }
    let Some(host) = process::shell() else { return };
    let declined = voloo::home::dir().map(|dir| {
        dir.join("completions")
            .join(format!("declined-{}", host.shell.name()))
    });
    if declined.as_ref().is_some_and(|path| path.exists()) {
        return;
    }
    let result = (|| {
        let setup = Setup::new(host.shell, Some(&host.executable))?;
        let paths = setup.pending()?;
        if paths.is_empty() {
            return Ok(());
        }
        eprintln!("Enable Voloo Tab completion for {}?", host.shell.name());
        for path in &paths {
            eprintln!("  {}", path.display());
        }
        eprint!("Install for new terminals? [y/N] ");
        io::stderr().flush()?;
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer)? == 0 {
            return Ok(());
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => setup.write(&paths)?,
            "" | "n" | "no" => {
                if let Some(path) = declined {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(path)?;
                }
                eprintln!("Skipped. To enable later, run: voloo completions install");
            }
            _ => {}
        }
        Ok::<(), io::Error>(())
    })();
    if let Err(error) = result {
        eprintln!("voloo: could not set up Tab completion: {error}");
    }
}

pub(crate) fn install(requested: Option<Shell>) -> ExitCode {
    let host = process::shell();
    let shell = requested.or_else(|| host.as_ref().map(|host| host.shell));
    let Some(shell) = shell else {
        eprintln!("Specify a shell: voloo completions install bash|zsh|fish|powershell");
        return ExitCode::from(4);
    };
    let executable = host
        .as_ref()
        .filter(|host| host.shell == shell)
        .map(|host| host.executable.as_os_str());
    let result = Setup::new(shell, executable).and_then(|setup| setup.write(&setup.pending()?));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("voloo: could not install Tab completion: {error}");
            ExitCode::from(3)
        }
    }
}

impl Setup {
    fn new(shell: Shell, executable: Option<&OsStr>) -> io::Result<Self> {
        let paths = if shell == Shell::Pwsh {
            vec![powershell_profile(executable)?]
        } else {
            let home =
                user_home().ok_or_else(|| io::Error::other("cannot find the home directory"))?;
            match shell {
                Shell::Bash => {
                    let login = [".bash_profile", ".bash_login", ".profile"]
                        .map(|name| home.join(name))
                        .into_iter()
                        .find(|path| path.exists())
                        .unwrap_or_else(|| home.join(".bash_profile"));
                    vec![home.join(".bashrc"), login]
                }
                Shell::Zsh => vec![absolute_env("ZDOTDIR").unwrap_or(home).join(".zshrc")],
                Shell::Fish => vec![
                    absolute_env("XDG_CONFIG_HOME")
                        .unwrap_or_else(|| home.join(".config"))
                        .join("fish/completions/voloo.fish"),
                ],
                Shell::Pwsh => unreachable!(),
            }
        };
        Ok(Self { shell, paths })
    }

    fn pending(&self) -> io::Result<Vec<&Path>> {
        self.paths
            .iter()
            .filter_map(|path| match read_startup(path) {
                Ok(Some(data)) if self.shell == Shell::Fish || self.has_loader(&data) => None,
                Ok(_) => Some(Ok(path.as_path())),
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    fn has_loader(&self, data: &[u8]) -> bool {
        let text = profile_text(data);
        let command = format!("voloo completions {}", self.shell.name());
        text.lines().any(|line| {
            let line = line.trim();
            line == MARKER || (!line.starts_with('#') && line.contains(&command))
        })
    }

    fn write(&self, paths: &[&Path]) -> io::Result<()> {
        if paths.is_empty() {
            eprintln!(
                "Voloo Tab completion is already configured for {}.",
                self.shell.name()
            );
            return Ok(());
        }
        for path in paths {
            self.write_one(path)
                .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?;
        }
        eprintln!(
            "Voloo Tab completion for {} is ready in new terminals.",
            self.shell.name()
        );
        Ok(())
    }

    fn write_one(&self, path: &Path) -> io::Result<()> {
        let loader = match self.shell {
            Shell::Bash => BASH_LOADER,
            Shell::Zsh => ZSH_LOADER,
            Shell::Fish => FISH_LOADER,
            Shell::Pwsh => POWERSHELL_LOADER,
        };
        let block = format!(
            "\n{MARKER}\n{}# <<< voloo tab completion <<<\n",
            loader.trim_start()
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _ = read_startup(path)?;
        if self.shell == Shell::Fish {
            // A user-owned completion file takes precedence over our loader.
            return match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut file) => {
                    let result = file.write_all(block.as_bytes());
                    if result.is_err() {
                        drop(file);
                        let _ = fs::remove_file(path);
                    }
                    result
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                Err(error) => Err(error),
            };
        }

        // Recheck under a lock so simultaneous Voloo invocations append only once.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock()?;
        let mut original = Vec::new();
        file.read_to_end(&mut original)?;
        if self.has_loader(&original) {
            return Ok(());
        }
        if self.shell == Shell::Pwsh
            && profile_text(&original).contains("# SIG # Begin signature block")
        {
            return Err(io::Error::other("PowerShell profile is signed"));
        }
        let block = if original.starts_with(&[0xff, 0xfe]) {
            block.encode_utf16().flat_map(u16::to_le_bytes).collect()
        } else if original.starts_with(&[0xfe, 0xff]) {
            block.encode_utf16().flat_map(u16::to_be_bytes).collect()
        } else {
            block.into_bytes()
        };
        if let Err(error) = file.write_all(&block) {
            let _ = file.set_len(original.len() as u64);
            return Err(error);
        }
        Ok(())
    }
}

fn read_startup(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            let mut file = File::open(path)?;
            // Windows enforces the writer's lock for other file handles.
            file.lock_shared()?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            _ => Err(io::Error::other(format!(
                "{} is a dangling link",
                path.display()
            ))),
        },
        Ok(_) => Err(io::Error::other(format!(
            "{} is not a file",
            path.display()
        ))),
        Err(error) => Err(error),
    }
}

fn profile_text(bytes: &[u8]) -> String {
    let decode = if bytes.starts_with(&[0xff, 0xfe]) {
        u16::from_le_bytes
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        u16::from_be_bytes
    } else {
        return String::from_utf8_lossy(bytes).into_owned();
    };
    let units: Vec<_> = bytes[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| decode(pair))
        .collect();
    String::from_utf16_lossy(&units)
}

fn user_home() -> Option<PathBuf> {
    absolute_env("HOME").or_else(|| absolute_env("USERPROFILE"))
}

fn absolute_env(key: &str) -> Option<PathBuf> {
    let value = env::var_os(key)?;
    #[cfg(windows)]
    if let Some(path) = value.to_str()
        && let [b'/', drive, b'/', ..] = path.as_bytes()
        && drive.is_ascii_alphabetic()
    {
        // MSYS can pass Git Bash configuration directories as /c/Users/... .
        return Some(PathBuf::from(format!("{}:{}", *drive as char, &path[2..])));
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

fn powershell_profile(executable: Option<&OsStr>) -> io::Result<PathBuf> {
    let default = if cfg!(windows) { "pwsh.exe" } else { "pwsh" };
    let find = |exe: &OsStr| {
        let result = Command::new(exe)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::OutputEncoding = [Text.Encoding]::UTF8; [Console]::Write($PROFILE.CurrentUserAllHosts)",
            ])
            .output()
            .ok()?;
        if !result.status.success() {
            return None;
        }
        let path = PathBuf::from(
            std::str::from_utf8(&result.stdout)
                .ok()?
                .trim_start_matches('\u{feff}'),
        );
        path.is_absolute().then_some(path)
    };
    executable
        .or(Some(OsStr::new(default)))
        .and_then(find)
        .or_else(|| {
            (cfg!(windows) && executable.is_none())
                .then(|| find(OsStr::new("powershell.exe")))
                .flatten()
        })
        .ok_or_else(|| io::Error::other("cannot read the current PowerShell profile path"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_profile_keeps_its_encoding_and_existing_signature() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("profile.ps1");
        let setup = Setup {
            shell: Shell::Pwsh,
            paths: vec![path.clone()],
        };
        let original = "# 中文";
        let mut utf16 = vec![0xff, 0xfe];
        utf16.extend(original.encode_utf16().flat_map(u16::to_le_bytes));
        fs::write(&path, &utf16).unwrap();
        setup.write_one(&path).unwrap();
        let current = fs::read(&path).unwrap();
        assert!(current.starts_with(&utf16));
        let text = profile_text(&current);
        assert!(text.starts_with(original) && text.contains("voloo completions powershell"));
        assert!(setup.pending().unwrap().is_empty());

        let signed = b"# SIG # Begin signature block\n";
        fs::write(&path, signed).unwrap();
        assert!(setup.write_one(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), signed);
    }
}
