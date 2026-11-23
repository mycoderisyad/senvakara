//! The shells installed on this PC that a terminal tab can start
//! (TERM-01, task TAB-NEW-02): Windows PowerShell, PowerShell 7, Command
//! Prompt, Git Bash and every WSL distribution. A shell that is not found is
//! not listed.
//!
//! Looking takes a moment (`wsl.exe` has to answer), so [`detect`] blocks:
//! `App::local_shells` runs it off the async workers. Choosing among the
//! places a shell can live in is pure and tested without a file system.

use senvakara_adapters_pty::ShellKind;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// One shell a terminal tab can start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalShell {
    /// Stable name: "powershell", "pwsh", "cmd", "git-bash" or "wsl:<distro>".
    pub id: String,
    /// What the user reads: "PowerShell 7", "Ubuntu-24.04".
    pub name: String,
    pub kind: ShellKind,
}

/// The first folder of `path_var` (a `PATH` value) that holds `program`.
fn find_on_path(
    program: &str,
    path_var: Option<&OsStr>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    std::env::split_paths(path_var?)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(program))
        .find(|file| exists(file))
}

/// Where PowerShell 7 is: the `pwsh.exe` on `PATH`, otherwise the default
/// install folder under one of `roots` (the Program Files folders).
fn pwsh_path(
    on_path: Option<PathBuf>,
    roots: &[PathBuf],
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    on_path.or_else(|| {
        roots
            .iter()
            .map(|root| root.join("PowerShell").join("7").join("pwsh.exe"))
            .find(|file| exists(file))
    })
}

/// Where Git Bash is: `Git\bin\bash.exe` under one of `roots`, otherwise the
/// `bin\bash.exe` of the Git for Windows that owns the `git.exe` on `PATH`
/// (`<Git>\cmd\git.exe`, `<Git>\bin\git.exe` or `<Git>\mingw64\bin\git.exe`).
/// A bare `bash.exe` on `PATH` is not used: in `System32` that name is the
/// WSL launcher.
fn git_bash_path(
    roots: &[PathBuf],
    git_on_path: Option<&Path>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let installed = roots
        .iter()
        .map(|root| root.join("Git").join("bin").join("bash.exe"))
        .find(|file| exists(file));
    installed.or_else(|| {
        git_on_path?
            .ancestors()
            .skip(1)
            .take(3)
            .map(|dir| dir.join("bin").join("bash.exe"))
            .find(|file| exists(file))
    })
}

/// The list in the order the UI shows it: Windows PowerShell, PowerShell 7,
/// Command Prompt, Git Bash, then the WSL distributions as `wsl.exe` listed
/// them (default first). Windows PowerShell and Command Prompt ship with
/// Windows and are always there.
fn listed(pwsh: Option<PathBuf>, git_bash: Option<PathBuf>, wsl: Vec<String>) -> Vec<LocalShell> {
    let shell = |id: &str, name: &str, kind| LocalShell {
        id: id.to_string(),
        name: name.to_string(),
        kind,
    };
    let mut shells = vec![shell(
        "powershell",
        "Windows PowerShell",
        ShellKind::PowerShell,
    )];
    if let Some(path) = pwsh {
        shells.push(shell("pwsh", "PowerShell 7", ShellKind::Pwsh { path }));
    }
    shells.push(shell("cmd", "Command Prompt", ShellKind::Cmd));
    if let Some(path) = git_bash {
        shells.push(shell("git-bash", "Git Bash", ShellKind::GitBash { path }));
    }
    for distro in wsl {
        let kind = ShellKind::Wsl {
            distro: Some(distro.clone()),
        };
        shells.push(shell(&format!("wsl:{distro}"), &distro, kind));
    }
    shells
}

/// Installed WSL distributions, default first. Empty when WSL is missing or
/// does not answer.
pub(crate) fn wsl_list() -> std::io::Result<Vec<String>> {
    let mut command = std::process::Command::new("wsl.exe");
    command.args(["-l", "-q"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up in a windowed app.
        command.creation_flags(0x0800_0000);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_wsl_list(&output.stdout))
}

/// The names in the output of `wsl.exe -l -q`, which is UTF-16LE.
fn parse_wsl_list(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.len() >= 2 && bytes[1] == 0 {
        let (pairs, _) = bytes.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Look for the local shells. Blocking: it reads the file system and waits
/// for `wsl.exe`; run it off the UI thread and off the async workers.
pub(crate) fn detect() -> Vec<LocalShell> {
    // A Store install of PowerShell 7 is an app execution alias: it is there
    // and can be started, but following it like a link fails.
    let exists = |file: &Path| std::fs::symlink_metadata(file).is_ok();
    let path_var = std::env::var_os("PATH");
    let mut roots: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        // Where a per-user install of Git for Windows goes.
        roots.push(PathBuf::from(local).join("Programs"));
    }
    let pwsh = pwsh_path(
        find_on_path("pwsh.exe", path_var.as_deref(), &exists),
        &roots,
        &exists,
    );
    let git = find_on_path("git.exe", path_var.as_deref(), &exists);
    let git_bash = git_bash_path(&roots, git.as_deref(), &exists);
    listed(pwsh, git_bash, wsl_list().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |file| files.iter().any(|known| Path::new(known) == file)
    }

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from(r"C:\Program Files")]
    }

    #[test]
    fn parses_utf16_wsl_output() {
        let s = "Ubuntu-24.04\r\ndocker-desktop\r\n";
        let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_wsl_list(&bytes), ["Ubuntu-24.04", "docker-desktop"]);
        assert!(parse_wsl_list(&[]).is_empty());
    }

    #[test]
    fn a_program_is_found_in_the_first_path_folder_that_has_it() {
        let exists = only(&[r"D:\tools\pwsh.exe", r"E:\later\pwsh.exe"]);
        let path = std::ffi::OsString::from(r"C:\Windows;;D:\tools;E:\later");
        assert_eq!(
            find_on_path("pwsh.exe", Some(&path), &exists),
            Some(PathBuf::from(r"D:\tools\pwsh.exe"))
        );
        assert_eq!(find_on_path("git.exe", Some(&path), &exists), None);
        assert_eq!(find_on_path("pwsh.exe", None, &exists), None);
    }

    #[test]
    fn powershell_7_on_path_wins_over_the_default_folder() {
        let exists = only(&[r"C:\Program Files\PowerShell\7\pwsh.exe"]);
        let on_path = PathBuf::from(r"D:\tools\pwsh.exe");
        assert_eq!(
            pwsh_path(Some(on_path.clone()), &roots(), &exists),
            Some(on_path)
        );
        assert_eq!(
            pwsh_path(None, &roots(), &exists),
            Some(PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe"))
        );
        assert_eq!(pwsh_path(None, &roots(), &only(&[])), None);
    }

    #[test]
    fn git_bash_is_found_in_program_files_or_next_to_git_on_path() {
        let default = only(&[r"C:\Program Files\Git\bin\bash.exe"]);
        assert_eq!(
            git_bash_path(&roots(), None, &default),
            Some(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"))
        );
        // Installed somewhere else: found through the git.exe on PATH.
        let moved = only(&[r"D:\Apps\Git\bin\bash.exe"]);
        for git in [
            r"D:\Apps\Git\cmd\git.exe",
            r"D:\Apps\Git\bin\git.exe",
            r"D:\Apps\Git\mingw64\bin\git.exe",
        ] {
            assert_eq!(
                git_bash_path(&roots(), Some(Path::new(git)), &moved),
                Some(PathBuf::from(r"D:\Apps\Git\bin\bash.exe")),
                "{git}"
            );
        }
        // A git.exe that is not Git for Windows brings no bash.
        let other = Path::new(r"D:\scoop\shims\git.exe");
        assert_eq!(git_bash_path(&roots(), Some(other), &moved), None);
        assert_eq!(git_bash_path(&roots(), None, &only(&[])), None);
    }

    #[test]
    fn missing_shells_are_left_out_and_the_order_is_fixed() {
        let ids = |shells: Vec<LocalShell>| -> Vec<String> {
            shells.into_iter().map(|shell| shell.id).collect()
        };
        assert_eq!(ids(listed(None, None, Vec::new())), ["powershell", "cmd"]);
        let all = listed(
            Some(r"C:\pwsh.exe".into()),
            Some(r"C:\bash.exe".into()),
            vec!["Ubuntu-24.04".into(), "Debian".into()],
        );
        assert_eq!(all[4].name, "Ubuntu-24.04");
        assert_eq!(
            all[4].kind,
            ShellKind::Wsl {
                distro: Some("Ubuntu-24.04".into())
            }
        );
        assert_eq!(
            ids(all),
            [
                "powershell",
                "pwsh",
                "cmd",
                "git-bash",
                "wsl:Ubuntu-24.04",
                "wsl:Debian"
            ]
        );
    }
}
