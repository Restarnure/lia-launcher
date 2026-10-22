pub mod accounts;
pub mod app;
pub mod content;
pub mod crypto;
pub mod ctx;
pub mod error;
pub mod instances;
pub mod launch;
pub mod loaders;
pub mod logparse;
pub mod migrate;
pub mod modrinth;
pub mod mojang;
pub mod net;
pub mod news;
pub mod paths;
pub mod servers;
pub mod settings;
pub mod skins;
pub mod tasks;
pub mod util;

use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_window_state::StateFlags;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .with_denylist(&["ms-login"])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = Arc::new(app::AppState::new(app.handle().clone()));
            app::startup(&state);
            state.allow_assets();
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::app_info,
            app::open_url,
            app::open_path,
            app::task_cancel,
            app::settings_get,
            app::settings_set,
            app::java_probe,
            app::versions_list,
            app::loader_versions,
            app::loader_games,
            app::instances_list,
            app::instance_get,
            app::instance_create,
            app::instance_update,
            app::instance_set_icon_file,
            app::instance_delete,
            app::instance_duplicate,
            app::instance_folder,
            app::instance_launch,
            app::instance_repair,
            app::instance_kill,
            app::game_log,
            app::games_running,
            app::featured_get,
            app::featured_play,
            app::server_ping,
            app::content_list,
            app::content_toggle,
            app::content_remove,
            app::content_add,
            app::content_identify,
            app::content_updates,
            app::content_update,
            app::worlds_list,
            app::screenshots_list,
            app::game_file_trash,
            app::mr_search,
            app::mr_project,
            app::mr_versions,
            app::mr_install,
            app::mr_install_modpack,
            app::mrpack_import,
            app::accounts_list,
            app::account_add_offline,
            app::account_select,
            app::account_remove,
            app::account_login_ms,
            app::account_skin,
            app::skin_upload,
            app::cape_set,
            app::mcdir_scan,
            app::mcdir_import,
            app::news_list,
            app::news_body,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
