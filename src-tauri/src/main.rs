#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::Mutex,
    thread,
    time::Duration,
};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    path::BaseDirectory,
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, PhysicalPosition, Runtime, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const MAIN_WINDOW_LABEL: &str = "main";
const SETTINGS_WINDOW_LABEL: &str = "settings";
const TRAY_ID: &str = "main-tray";

struct RuntimeState {
    child: Mutex<Option<Child>>,
    status: Mutex<FrpcStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatePayload {
    config: String,
    status: FrpcStatus,
    autostart_enabled: bool,
    auto_start_frpc_on_launch: bool,
    config_path: String,
    log_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum FrpcStatus {
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSettings {
    #[serde(default = "default_auto_start_frpc_on_launch")]
    auto_start_frpc_on_launch: bool,
}

fn default_auto_start_frpc_on_launch() -> bool {
    true
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        .manage(RuntimeState {
            child: Mutex::new(None),
            status: Mutex::new(FrpcStatus::Stopped),
        })
        .setup(|app| {
            ensure_bootstrap_files(app.handle())?;
            cleanup_frpc_processes_for_app(app.handle())?;
            build_app_menu(app)?;
            build_tray(app)?;
            attach_menu_event_handlers(app);
            attach_hide_on_close(app.handle(), MAIN_WINDOW_LABEL)?;
            sync_runtime_status(app.handle(), &app.state::<RuntimeState>())?;

            let app_handle = app.handle().clone();
            if load_settings(&app_handle)?.auto_start_frpc_on_launch {
                let _ = start_frpc_internal(&app_handle, &app_handle.state::<RuntimeState>());
            }
            if let Err(err) =
                show_app_in_dock(&app_handle).and_then(|_| show_main_window(&app_handle))
            {
                eprintln!("failed to show main window during setup: {err}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_state,
            save_config,
            start_frpc,
            stop_frpc,
            restart_frpc,
            set_autostart,
            set_auto_start_frpc_on_launch,
            read_logs,
            open_settings_window,
            open_main_window,
            hide_current_window,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build tauri application")
        .run(|app, event| match event {
            tauri::RunEvent::Ready => {
                if let Err(err) = show_app_in_dock(app).and_then(|_| show_main_window(app)) {
                    eprintln!("failed to show main window during ready: {err}");
                }
            }
            tauri::RunEvent::Reopen { .. } => {
                let _ = show_app_in_dock(app).and_then(|_| show_main_window(app));
            }
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                let state = app.state::<RuntimeState>();
                let _ = stop_frpc_internal(app, &state);
            }
            _ => {}
        });
}

fn build_app_menu(app: &mut tauri::App<tauri::Wry>) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let close_window = MenuItem::with_id(
        &handle,
        "hide_window_menu",
        "关闭窗口",
        true,
        Some("CmdOrCtrl+W"),
    )?;
    let quit = MenuItem::with_id(&handle, "quit", "退出 Quay", true, Some("CmdOrCtrl+Q"))?;
    let file = Submenu::with_items(&handle, "File", true, &[&close_window, &quit])?;

    let undo = PredefinedMenuItem::undo(&handle, None)?;
    let redo = PredefinedMenuItem::redo(&handle, None)?;
    let separator_1 = PredefinedMenuItem::separator(&handle)?;
    let cut = PredefinedMenuItem::cut(&handle, None)?;
    let copy = PredefinedMenuItem::copy(&handle, None)?;
    let paste = PredefinedMenuItem::paste(&handle, None)?;
    let select_all = MenuItem::with_id(
        &handle,
        "editor_select_all",
        "Select All",
        true,
        Some("CmdOrCtrl+A"),
    )?;
    let edit = Submenu::with_items(
        &handle,
        "Edit",
        true,
        &[&undo, &redo, &separator_1, &cut, &copy, &paste, &select_all],
    )?;

    let menu = Menu::with_items(&handle, &[&file, &edit])?;
    app.set_menu(menu)?;

    Ok(())
}

fn attach_menu_event_handlers(app: &mut tauri::App<tauri::Wry>) {
    app.on_menu_event(|app, event| match event.id().as_ref() {
        "hide_window_menu" => {
            let _ = hide_frontmost_window(app);
        }
        "show_main" => {
            let _ = show_app_in_dock(app).and_then(|_| show_main_window(app));
        }
        "show_settings" => {
            let _ = show_app_in_dock(app).and_then(|_| show_settings_window(app));
        }
        "frpc_action" => {
            let state = app.state::<RuntimeState>();
            let current = sync_runtime_status(app, &state).unwrap_or(FrpcStatus::Stopped);
            let _ = match current {
                FrpcStatus::Running => stop_frpc_internal(app, &state),
                FrpcStatus::Stopped => start_frpc_internal(app, &state),
                FrpcStatus::Error => restart_frpc_internal(app, &state),
            };
        }
        "editor_select_all" => {
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                let _ = window.emit("editor-shortcut", "select-all");
            }
        }
        "quit" => {
            let state = app.state::<RuntimeState>();
            let _ = stop_frpc_internal(app, &state);
            app.exit(0);
        }
        _ => {}
    });
}

fn attach_hide_on_close<R: Runtime>(app: &AppHandle<R>, label: &str) -> Result<(), String> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("missing window {label}"))?;
    let app_handle = app.clone();
    let label = label.to_string();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hide_window_and_maybe_hide_dock(&app_handle, &label);
        }
    });
    Ok(())
}

fn build_tray(app: &mut tauri::App<tauri::Wry>) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let menu = build_tray_menu(&handle, FrpcStatus::Stopped)?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_template_icon())
        .icon_as_template(true)
        .tooltip("Quay")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .build(&handle)?;

    Ok(())
}

fn build_tray_menu<R: Runtime>(
    app: &AppHandle<R>,
    status: FrpcStatus,
) -> Result<Menu<R>, Box<dyn std::error::Error>> {
    let show_main = MenuItem::with_id(app, "show_main", "打开主窗口", true, None::<&str>)?;
    let show_settings = MenuItem::with_id(app, "show_settings", "打开设置", true, None::<&str>)?;
    let action_label = match status {
        FrpcStatus::Running => "停止 frpc",
        FrpcStatus::Stopped => "启动 frpc",
        FrpcStatus::Error => "重新启动",
    };
    let frpc_action = MenuItem::with_id(app, "frpc_action", action_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    Menu::with_items(app, &[&show_main, &show_settings, &frpc_action, &quit]).map_err(Into::into)
}

fn update_tray_menu<R: Runtime>(app: &AppHandle<R>, status: FrpcStatus) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_tray_menu(app, status) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn tray_template_icon() -> Image<'static> {
    Image::from_bytes(include_bytes!("../icons/tray-template.png"))
        .expect("embedded tray template icon must be a valid PNG")
        .to_owned()
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let window = if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        window
    } else {
        WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, WebviewUrl::App("index.html".into()))
            .title("Quay")
            .inner_size(920.0, 760.0)
            .center()
            .resizable(true)
            .maximizable(false)
            .title_bar_style(tauri::TitleBarStyle::Transparent)
            .hidden_title(true)
            .traffic_light_position(tauri::Position::Logical(tauri::LogicalPosition {
                x: 14.0,
                y: 16.0,
            }))
            .build()
            .map_err(|err| err.to_string())?
    };
    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;
    window
        .set_content_protected(false)
        .map_err(|err| err.to_string())?;
    center_on_primary_monitor(&window)?;
    activate_app(app)?;
    window.set_focus().map_err(|err| err.to_string())?;
    Ok(())
}

fn center_on_primary_monitor<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    let Some(monitor) = window.primary_monitor().map_err(|err| err.to_string())? else {
        return window.center().map_err(|err| err.to_string());
    };
    let size = window.outer_size().map_err(|err| err.to_string())?;
    let work_area = monitor.work_area();
    let x = work_area.position.x + ((work_area.size.width.saturating_sub(size.width)) / 2) as i32;
    let y = work_area.position.y + ((work_area.size.height.saturating_sub(size.height)) / 2) as i32;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|err| err.to_string())
}

fn show_settings_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        window.show().map_err(|err| err.to_string())?;
        window.unminimize().map_err(|err| err.to_string())?;
        window
            .set_content_protected(false)
            .map_err(|err| err.to_string())?;
        center_on_primary_monitor(&window)?;
        activate_app(app)?;
        window.set_focus().map_err(|err| err.to_string())?;
        return Ok(());
    }

    let builder = WebviewWindowBuilder::new(
        app,
        SETTINGS_WINDOW_LABEL,
        WebviewUrl::App("index.html?view=settings".into()),
    )
    .title("设置")
    .inner_size(560.0, 380.0)
    .center()
    .resizable(false)
    .maximizable(false)
    .title_bar_style(tauri::TitleBarStyle::Transparent)
    .hidden_title(true)
    .traffic_light_position(tauri::Position::Logical(tauri::LogicalPosition {
        x: 14.0,
        y: 16.0,
    }));

    let window = builder.build().map_err(|err| err.to_string())?;
    window
        .set_content_protected(false)
        .map_err(|err| err.to_string())?;
    center_on_primary_monitor(&window)?;
    activate_app(app)?;
    window.set_focus().map_err(|err| err.to_string())?;
    attach_hide_on_close(app, SETTINGS_WINDOW_LABEL)?;
    Ok(())
}

fn activate_app<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            let ns_app = objc2_app_kit::NSApplication::sharedApplication(mtm);
            #[allow(non_upper_case_globals)]
            {
                let _ = ns_app
                    .setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Regular);
            }
            ns_app.unhide(None);
            #[allow(deprecated)]
            ns_app.activateIgnoringOtherApps(true);
            ns_app.activate();
        } else {
            app.run_on_main_thread(|| {
                if let Some(mtm) = objc2::MainThreadMarker::new() {
                    let ns_app = objc2_app_kit::NSApplication::sharedApplication(mtm);
                    #[allow(non_upper_case_globals)]
                    {
                        let _ = ns_app.setActivationPolicy(
                            objc2_app_kit::NSApplicationActivationPolicy::Regular,
                        );
                    }
                    ns_app.unhide(None);
                    #[allow(deprecated)]
                    ns_app.activateIgnoringOtherApps(true);
                    ns_app.activate();
                }
            })
            .map_err(|err| err.to_string())?;
        }
    }

    Ok(())
}
fn show_app_in_dock<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    app.set_dock_visibility(true)
        .map_err(|err| err.to_string())?;

    #[cfg(target_os = "macos")]
    app.show().map_err(|err| err.to_string())?;

    Ok(())
}

fn hide_window_and_maybe_hide_dock<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(label) {
        window.hide().map_err(|err| err.to_string())?;
    }

    hide_dock_if_no_visible_windows(app)
}

fn hide_frontmost_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    for label in [SETTINGS_WINDOW_LABEL, MAIN_WINDOW_LABEL] {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        if window.is_focused().map_err(|err| err.to_string())? {
            return hide_window_and_maybe_hide_dock(app, label);
        }
    }

    for label in [MAIN_WINDOW_LABEL, SETTINGS_WINDOW_LABEL] {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        if window.is_visible().map_err(|err| err.to_string())? {
            return hide_window_and_maybe_hide_dock(app, label);
        }
    }

    hide_dock_if_no_visible_windows(app)
}

fn hide_dock_if_no_visible_windows<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    for label in [MAIN_WINDOW_LABEL, SETTINGS_WINDOW_LABEL] {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        if window.is_visible().map_err(|err| err.to_string())? {
            return Ok(());
        }
    }

    #[cfg(target_os = "macos")]
    app.hide().map_err(|err| err.to_string())?;

    app.set_dock_visibility(false)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn open_settings_window(app: AppHandle) -> Result<(), String> {
    show_app_in_dock(&app).and_then(|_| show_settings_window(&app))
}

#[tauri::command]
fn open_main_window(app: AppHandle) -> Result<(), String> {
    show_app_in_dock(&app).and_then(|_| show_main_window(&app))
}

#[tauri::command]
fn hide_current_window(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    hide_window_and_maybe_hide_dock(&app, window.label())
}

#[tauri::command]
fn get_app_state(
    app: AppHandle,
    state: State<'_, RuntimeState>,
) -> Result<AppStatePayload, String> {
    let config_path = app_config_path(&app)?;
    if !config_path.exists() {
        ensure_bootstrap_files(&app)?;
    }

    let config = fs::read_to_string(&config_path).map_err(|err| err.to_string())?;
    let autostart_enabled = app
        .autolaunch()
        .is_enabled()
        .map_err(|err| err.to_string())?;
    let settings = load_settings(&app)?;
    let log_path = app_log_path(&app)?;
    let status = sync_runtime_status(&app, &state)?;

    Ok(AppStatePayload {
        config,
        status,
        autostart_enabled,
        auto_start_frpc_on_launch: settings.auto_start_frpc_on_launch,
        config_path: config_path.display().to_string(),
        log_path: log_path.display().to_string(),
    })
}

#[tauri::command]
fn read_logs(app: AppHandle) -> Result<Vec<String>, String> {
    let log_path = app_log_path(&app)?;
    if !log_path.exists() {
        return Ok(vec![]);
    }

    let file = File::open(log_path).map_err(|err| err.to_string())?;
    let reader = BufReader::new(file);
    let mut lines = reader
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    const MAX_LINES: usize = 120;
    if lines.len() > MAX_LINES {
        lines = lines.split_off(lines.len() - MAX_LINES);
    }
    Ok(lines)
}

#[tauri::command]
fn save_config(app: AppHandle, content: String) -> Result<(), String> {
    let config_path = app_config_path(&app)?;
    fs::write(config_path, content).map_err(|err| err.to_string())
}

#[tauri::command]
fn start_frpc(app: AppHandle, state: State<'_, RuntimeState>) -> Result<(), String> {
    start_frpc_internal(&app, &state)
}

#[tauri::command]
fn stop_frpc(app: AppHandle, state: State<'_, RuntimeState>) -> Result<(), String> {
    stop_frpc_internal(&app, &state)
}

#[tauri::command]
fn restart_frpc(app: AppHandle, state: State<'_, RuntimeState>) -> Result<(), String> {
    restart_frpc_internal(&app, &state)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        app.autolaunch().enable().map_err(|err| err.to_string())
    } else {
        app.autolaunch().disable().map_err(|err| err.to_string())
    }
}

#[tauri::command]
fn set_auto_start_frpc_on_launch(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = load_settings(&app)?;
    settings.auto_start_frpc_on_launch = enabled;
    save_settings(&app, &settings)
}

fn restart_frpc_internal<R: Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, RuntimeState>,
) -> Result<(), String> {
    let _ = stop_frpc_internal(app, state);
    start_frpc_internal(app, state)
}

fn start_frpc_internal<R: Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, RuntimeState>,
) -> Result<(), String> {
    let mut guard = state.child.lock().map_err(|_| "状态锁已损坏".to_string())?;
    if let Some(child) = guard.as_mut() {
        match child.try_wait().map_err(|err| err.to_string())? {
            None => {
                set_status(app, state, FrpcStatus::Running);
                return Ok(());
            }
            Some(_) => {
                *guard = None;
            }
        }
    }

    let binary_path = resolve_known_resource_path(app, &["resources/frpc", "_up_/frpc", "frpc"])?;
    let config_path = app_config_path(app)?;
    let log_path = app_log_path(app)?;

    if !config_path.exists() {
        ensure_bootstrap_files(app)?;
    }

    terminate_frpc_processes_for_config(&config_path, None)?;
    prepare_log_file(&log_path)?;
    let stdout = open_log_file(&log_path)?;
    let stderr = open_log_file(&log_path)?;

    let mut command = Command::new(binary_path);
    command
        .arg("-c")
        .arg(&config_path)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    #[cfg(unix)]
    {
        command.process_group(0);
    }

    let child = command
        .spawn()
        .map_err(|err| format!("启动 frpc 失败: {err}"))?;

    *guard = Some(child);
    drop(guard);
    set_status(app, state, FrpcStatus::Running);
    Ok(())
}

fn stop_frpc_internal<R: Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, RuntimeState>,
) -> Result<(), String> {
    let mut guard = state.child.lock().map_err(|_| "状态锁已损坏".to_string())?;
    if let Some(child) = guard.take() {
        terminate_managed_child(child);
    }
    drop(guard);
    cleanup_frpc_processes_for_app(app)?;
    set_status(app, state, FrpcStatus::Stopped);
    Ok(())
}

fn terminate_managed_child(mut child: Child) {
    let pid = child.id();

    #[cfg(unix)]
    terminate_process_group(pid);

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    let group = format!("-{pid}");
    let _ = Command::new("kill").args(["-TERM", &group]).status();
    thread::sleep(Duration::from_millis(300));

    if process_is_alive(pid) {
        let _ = Command::new("kill").args(["-KILL", &group]).status();
    }
}

fn sync_runtime_status<R: Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, RuntimeState>,
) -> Result<FrpcStatus, String> {
    let next = {
        let mut guard = state.child.lock().map_err(|_| "状态锁已损坏".to_string())?;
        if let Some(child) = guard.as_mut() {
            match child.try_wait().map_err(|err| err.to_string())? {
                None => FrpcStatus::Running,
                Some(status) => {
                    *guard = None;
                    status_to_state(status)
                }
            }
        } else {
            let current = state
                .status
                .lock()
                .map_err(|_| "状态锁已损坏".to_string())?;
            match *current {
                FrpcStatus::Running => FrpcStatus::Stopped,
                other => other,
            }
        }
    };
    set_status(app, state, next);
    Ok(next)
}

fn status_to_state(status: ExitStatus) -> FrpcStatus {
    if status.success() {
        FrpcStatus::Stopped
    } else {
        FrpcStatus::Error
    }
}

fn cleanup_frpc_processes_for_app<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let config_path = app_config_path(app)?;
    terminate_frpc_processes_for_config(&config_path, None)
}

fn terminate_frpc_processes_for_config(
    config_path: &PathBuf,
    exclude_pid: Option<u32>,
) -> Result<(), String> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .map_err(|err| format!("无法扫描 frpc 进程: {err}"))?;
    if !output.status.success() {
        return Ok(());
    }

    let config_text = config_path.to_string_lossy();
    let current_pid = std::process::id();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut targets = Vec::new();

    for line in stdout.lines() {
        let line = line.trim_start();
        let Some((pid_text, command)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_text.trim().parse::<u32>() else {
            continue;
        };
        if pid == current_pid || exclude_pid == Some(pid) {
            continue;
        }
        if command.contains("frpc") && command.contains(config_text.as_ref()) {
            targets.push(pid);
        }
    }

    for pid in &targets {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }

    if !targets.is_empty() {
        thread::sleep(Duration::from_millis(300));
    }

    for pid in targets {
        if process_is_alive(pid) {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
    }

    Ok(())
}

fn process_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn set_status<R: Runtime>(app: &AppHandle<R>, state: &State<'_, RuntimeState>, next: FrpcStatus) {
    if let Ok(mut status) = state.status.lock() {
        if *status != next {
            *status = next;
            let _ = app.emit("frpc-status", next);
            update_tray_menu(app, next);
        }
    }
}

fn ensure_bootstrap_files<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let config_path = app_config_path(app)?;
    if !config_path.exists() {
        let default_config = resolve_known_resource_path(app, &["resources/frpc.toml", "_up_/frpc.toml", "frpc.toml"])?;
        fs::copy(default_config, &config_path).map_err(|err| err.to_string())?;
    } else {
        migrate_legacy_config_if_needed(&config_path)?;
    }

    let settings_path = app_settings_path(app)?;
    if !settings_path.exists() {
        save_settings(app, &PersistedSettings::default())?;
    }

    Ok(())
}

fn migrate_legacy_config_if_needed(path: &PathBuf) -> Result<(), String> {
    let raw = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let migrated = quote_legacy_toml_string_values(&raw);
    if migrated != raw {
        fs::write(path, migrated).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn quote_legacy_toml_string_values(raw: &str) -> String {
    raw.lines()
        .map(|line| {
            let Some((key_part, value_part)) = line.split_once('=') else {
                return line.to_string();
            };

            let key = key_part.trim();
            let value = value_part.trim();
            let should_quote = matches!(
                key,
                "server_addr"
                    | "token"
                    | "user"
                    | "type"
                    | "local_ip"
                    | "plugin"
                    | "sk"
                    | "role"
                    | "server_name"
            );

            if !should_quote
                || value.is_empty()
                || value.starts_with('"')
                || value.starts_with('\'')
            {
                return line.to_string();
            }

            let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
            format!("{}= \"{}\"", key_part, escaped)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn app_config_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|err| format!("无法定位配置目录: {err}"))?;
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir.join("frpc.toml"))
}

fn app_log_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|err| format!("无法定位日志目录: {err}"))?;
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir.join("frpc.log"))
}

fn app_settings_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|err| format!("无法定位配置目录: {err}"))?;
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir.join("settings.json"))
}

fn load_settings<R: Runtime>(app: &AppHandle<R>) -> Result<PersistedSettings, String> {
    let settings_path = app_settings_path(app)?;
    if !settings_path.exists() {
        return Ok(PersistedSettings::default());
    }

    let raw = fs::read_to_string(settings_path).map_err(|err| err.to_string())?;
    serde_json::from_str(&raw).map_err(|err| err.to_string())
}

fn save_settings<R: Runtime>(
    app: &AppHandle<R>,
    settings: &PersistedSettings,
) -> Result<(), String> {
    let settings_path = app_settings_path(app)?;
    let raw = serde_json::to_string_pretty(settings).map_err(|err| err.to_string())?;
    fs::write(settings_path, raw).map_err(|err| err.to_string())
}

fn resolve_resource_path<R: Runtime>(app: &AppHandle<R>, name: &str) -> Result<PathBuf, String> {
    let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(name);
    if dev_path.exists() {
        return Ok(dev_path);
    }

    let bundled = app
        .path()
        .resolve(name, BaseDirectory::Resource)
        .map_err(|err| err.to_string())?;
    if bundled.exists() {
        Ok(bundled)
    } else {
        Err(format!("缺少资源文件: {name}"))
    }
}

fn resolve_known_resource_path<R: Runtime>(app: &AppHandle<R>, candidates: &[&str]) -> Result<PathBuf, String> {
    for candidate in candidates {
        if let Ok(path) = resolve_resource_path(app, candidate) {
            return Ok(path);
        }
    }

    Err(format!("缺少资源文件: {}", candidates.join(", ")))
}

fn prepare_log_file(path: &PathBuf) -> Result<(), String> {
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn open_log_file(path: &PathBuf) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| err.to_string())
}
