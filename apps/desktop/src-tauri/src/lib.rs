//! Senvakara desktop shell. Thin Tauri glue over the Rust core and adapters:
//! no business logic and no secrets live in the webview (docs/ARCHITECTURE.md §2).

mod android;
mod android_tools;
mod files;
mod hosts;
mod logging;
mod mirror;
mod state;
mod terminal;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            logging::init(app.path().app_log_dir()?);
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let state = state::AppState::open(data_dir.join("senvakara.db"))?;
            logging::info("app started");
            app.manage(state);
            android::start_local(app.handle().clone());
            Ok(())
        })
        .manage(terminal::Terminals::default())
        .manage(files::Transfers::default())
        .manage(android::Android::default())
        .manage(android_tools::AndroidTools::default())
        .manage(mirror::Mirrors::default())
        .invoke_handler(tauri::generate_handler![
            terminal::term_open,
            terminal::term_write,
            terminal::term_resize,
            terminal::term_close,
            terminal::wsl_distros,
            hosts::hosts_list,
            hosts::host_create,
            hosts::host_update,
            hosts::host_delete,
            hosts::host_check_key,
            hosts::host_trust_key,
            hosts::host_set_passphrase,
            hosts::host_connect,
            hosts::host_disconnect,
            files::sftp_list,
            files::sftp_download,
            files::sftp_upload,
            files::transfer_cancel,
            android::devices_list,
            android::adb_status,
            android::device_info,
            android::device_shell,
            android::device_rename,
            android::device_forget,
            android_tools::device_screenshot,
            android_tools::device_screenshot_save,
            android_tools::logcat_start,
            android_tools::logcat_stop,
            android_tools::device_packages,
            android_tools::device_uninstall,
            android_tools::device_install_apk,
            android_tools::device_files_list,
            android_tools::device_push,
            android_tools::device_pull,
            mirror::mirror_start,
            mirror::mirror_input,
            mirror::mirror_stop,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
