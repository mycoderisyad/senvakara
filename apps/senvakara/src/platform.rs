//! Platform probing: the installed WSL distributions and the Slint backend the window uses.

/// Installed WSL distributions, default first. Empty when WSL is not installed.
/// Temporary: moves to `crates/app` with the rest of the backend (UI-MIG-06).
pub(crate) fn wsl_distros() -> Vec<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(output) = std::process::Command::new("wsl.exe")
        .args(["-l", "-q"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    // wsl.exe prints UTF-16LE.
    let units: Vec<u16> = output
        .stdout
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim().trim_start_matches('\u{feff}').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Pick the Slint backend before the first window exists.
///
/// - Renderer: Slint's software renderer unless `SLINT_BACKEND` names another. With the
///   FemtoVG/OpenGL renderer an idle window cost about 0.5 % of a core in a time-critical
///   thread of the Intel OpenGL driver even with no frames drawn; the software renderer idled
///   at 0.16 % in total (TESTING §9, 2026-11-22). `SLINT_BACKEND=winit-femtovg` brings
///   OpenGL back.
/// - Title bar: always dark (dark theme only, ADR-023), whatever the Windows setting says.
pub(crate) fn select_backend() -> anyhow::Result<()> {
    let mut selector =
        slint::BackendSelector::new().with_winit_window_attributes_hook(|attributes| {
            attributes.with_theme(Some(slint::winit_030::winit::window::Theme::Dark))
        });
    if std::env::var_os("SLINT_BACKEND").is_none() {
        selector = selector
            .backend_name("winit".into())
            .renderer_name("software".into());
    }
    selector.select()?;
    Ok(())
}
