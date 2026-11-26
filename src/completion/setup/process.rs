use crate::completion::Shell;
use std::{collections::BTreeMap, ffi::OsString, path::Path};

pub(super) struct ShellProcess {
    pub(super) shell: Shell,
    pub(super) executable: OsString,
}

type Processes = BTreeMap<u32, (u32, OsString)>;
const MAX_LAUNCHERS: usize = 6;

pub(super) fn shell() -> Option<ShellProcess> {
    let processes = snapshot()?;
    let mut pid = std::process::id();
    // Follow only Voloo's launchers. An unrelated script may have a shell
    // higher in its process tree that is not the invoking shell.
    for _ in 0..MAX_LAUNCHERS {
        let (parent, _) = processes.get(&pid)?;
        pid = *parent;
        let (_, executable) = processes.get(&pid)?;
        let basename = Path::new(executable)
            .file_name()?
            .to_str()?
            .to_ascii_lowercase();
        let name = basename.trim_start_matches('-').trim_end_matches(".exe");
        let shell = match name {
            "bash" => Shell::Bash,
            "zsh" => Shell::Zsh,
            "fish" => Shell::Fish,
            "pwsh" | "powershell" => Shell::Pwsh,
            "node" | "nodejs" | "cmd" | "voloo" => continue,
            _ => return None,
        };
        #[cfg(windows)]
        let executable = if shell == Shell::Pwsh {
            executable_path(pid).unwrap_or_else(|| executable.clone())
        } else {
            executable.clone()
        };
        #[cfg(unix)]
        let executable = executable.clone();
        return Some(ShellProcess { shell, executable });
    }
    None
}

#[cfg(target_os = "linux")]
fn snapshot() -> Option<Processes> {
    let mut processes = Processes::new();
    let mut pid = std::process::id();
    for _ in 0..=MAX_LAUNCHERS {
        let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
        let Ok(stat) = std::fs::read_to_string(dir.join("stat")) else {
            break;
        };
        // The command name can contain spaces and parentheses. The fields
        // after the last ')' start with process state and then the parent PID.
        let (name, rest) = stat.rsplit_once(')')?;
        let parent = rest.split_whitespace().nth(1)?.parse().ok()?;
        let executable = std::fs::read_link(dir.join("exe"))
            .map(std::path::PathBuf::into_os_string)
            .unwrap_or_else(|_| name.split_once('(').map_or("", |(_, name)| name).into());
        processes.insert(pid, (parent, executable));
        pid = parent;
    }
    Some(processes)
}

#[cfg(windows)]
fn executable_path(pid: u32) -> Option<OsString> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
    };
    let mut path = vec![0u16; 32_768];
    let mut len = path.len() as u32;
    // SAFETY: the handle is non-inheritable and the buffer has len UTF-16 units.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let ok = QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        (ok != 0).then(|| OsString::from_wide(&path[..len as usize]))
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn snapshot() -> Option<Processes> {
    let ps = std::process::Command::new("ps")
        .args(["-ax", "-o", "pid=,ppid=,comm="])
        .output()
        .ok()?;
    if !ps.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&ps.stdout)
            .lines()
            .filter_map(|line| {
                let (pid, rest) = line.trim().split_once(char::is_whitespace)?;
                let (parent, executable) = rest.trim_start().split_once(char::is_whitespace)?;
                Some((
                    pid.parse().ok()?,
                    (parent.parse().ok()?, executable.trim().into()),
                ))
            })
            .collect(),
    )
}

#[cfg(windows)]
fn snapshot() -> Option<Processes> {
    use std::{mem::size_of, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };
    // SAFETY: the snapshot is non-inheritable. The entry is initialized with
    // dwSize, and the OS writes at most the size of PROCESSENTRY32W.
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..std::mem::zeroed()
        };
        let mut result = BTreeMap::new();
        let mut found = Process32FirstW(handle, &mut entry);
        while found != 0 {
            let name = entry
                .szExeFile
                .split(|ch| *ch == 0)
                .next()
                .unwrap_or_default();
            result.insert(
                entry.th32ProcessID,
                (entry.th32ParentProcessID, OsString::from_wide(name)),
            );
            found = Process32NextW(handle, &mut entry);
        }
        CloseHandle(handle);
        Some(result)
    }
}
