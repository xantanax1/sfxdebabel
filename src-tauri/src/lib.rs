mod audio;
mod commands;
mod db;
mod error;
mod library;
mod license;
mod models;
mod providers;

use tauri::Manager;

use crate::audio::export::export_temp_dir;
use crate::commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| export_temp_dir());
            let state = AppState::new(&data_dir).map_err(std::io::Error::other)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search_sfx,
            commands::play_sfx,
            commands::preview_sfx_with_fx,
            commands::render_fx_waveform,
            commands::stop_sfx,
            commands::set_volume,
            commands::get_playback_status,
            commands::prepare_drag_wav,
            commands::prepare_drag_file,
            commands::get_settings,
            commands::save_settings,
            commands::get_cache_stats,
            commands::clear_cache,
            commands::pick_library_folder,
            commands::start_library_index,
            commands::cancel_library_index,
            commands::get_library_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SFX de Babel");
}
