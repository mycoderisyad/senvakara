//! Senvakara desktop shell. Thin Tauri glue over the Rust core and adapters:
//! no business logic and no secrets live in the webview (docs/ARCHITECTURE.md §2).

mod terminal;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(terminal::Terminals::default())
        .invoke_handler(tauri::generate_handler![
            terminal::term_open,
            terminal::term_write,
            terminal::term_resize,
            terminal::term_close,
            terminal::wsl_distros,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
