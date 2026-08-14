#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, process::Command};
use tauri::Window;

// ИМПОРТ ДЛЯ СКРЫТИЯ CMD:
#[cfg(windows)]
use std::os::windows::process::CommandExt;

const TREE_API: &str = "https://api.github.com/repos/daniil179828/QWLEY-SHADE/git/trees/main?recursive=1";
const RAW_ROOT: &str = "https://raw.githubusercontent.com/daniil179828/QWLEY-SHADE/main/";
const MEDIA_ROOT: &str = "https://media.githubusercontent.com/media/daniil179828/QWLEY-SHADE/main/";
const PIPE_NAME: &str = r"\\.\pipe\fuckoffmaxey";
const OFFSETS_BASE: &str = "https://offsets.imtheo.lol";

#[derive(Deserialize)] struct TreeResponse { tree: Vec<TreeItem> }
#[derive(Deserialize)] struct TreeItem { path: String, #[serde(rename="type")] item_type: String, sha: String }
#[derive(Serialize, Deserialize, Default)] struct Manifest { files: BTreeMap<String, String> }

// ═══════════════════════════════════════════════════════════════
// СТРУКТУРА ОФФСЕТОВ (записывается в JSON)
// ═══════════════════════════════════════════════════════════════
#[derive(Serialize, Deserialize, Debug)]
struct RobloxOffsets {
    version: String,
    #[serde(rename = "visualEnginePointer")]
    visual_engine_pointer: String,
    #[serde(rename = "visualEngineToRenderView")]
    visual_engine_to_render_view: String,
    #[serde(rename = "renderViewToDevice")]
    render_view_to_device: String,
    #[serde(rename = "deviceToSwapChain")]
    device_to_swap_chain: String,
}

fn log(window: &Window, message: impl Into<String>) { let _ = window.emit("launcher-log", message.into()); }
fn app_folder() -> Result<PathBuf, String> { tauri::api::path::desktop_dir().map(|p| p.join("qwleyshade")).ok_or("Desktop folder was not found".into()) }
fn safe_path(path: &str) -> bool { !path.is_empty() && path.split('/').all(|p| !p.is_empty() && !p.starts_with('.') && p != "..") }
fn lfs(path: &str) -> bool { let p=path.to_ascii_lowercase(); p.ends_with(".exe") || p.ends_with(".dll") || p.ends_with(".ini") }
fn encode_path(path: &str) -> String { urlencoding::encode(path).replace("%2F", "/") }

// ═══════════════════════════════════════════════════════════════
// ОПРЕДЕЛЕНИЕ ВЕРСИИ ROBLOX
// Ищет папку version-* в %LOCALAPPDATA%\Roblox\Versions\
// которая содержит RobloxPlayerBeta.exe
// ═══════════════════════════════════════════════════════════════
fn detect_roblox_version() -> Result<String, String> {
    let local_app_data = std::env::var("LOCALAPPDATA")
        .map_err(|_| "LOCALAPPDATA environment variable not found".to_string())?;
    
    let versions_dir = Path::new(&local_app_data).join("Roblox").join("Versions");
    if !versions_dir.exists() {
        return Err("Roblox Versions directory not found".to_string());
    }

    let mut best: Option<(std::time::SystemTime, String)> = None;

    for entry in fs::read_dir(&versions_dir).map_err(|e| format!("Cannot read Versions dir: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        
        // Папки версий начинаются с "version-"
        if !name.starts_with("version-") {
            continue;
        }

        let exe_path = entry.path().join("RobloxPlayerBeta.exe");
        if !exe_path.exists() {
            continue;
        }

        // Берём самую свежую по дате модификации
        let modified = entry.path().metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        if best.is_none() || modified > best.as_ref().unwrap().0 {
            best = Some((modified, name));
        }
    }

    match best {
        Some((_, version)) => Ok(version),
        None => Err("No Roblox version folder found with RobloxPlayerBeta.exe".to_string()),
    }
}

// ═══════════════════════════════════════════════════════════════
// ЗАГРУЗКА И ПАРСИНГ ОФФСЕТОВ ИЗ offsets.hpp
// ═══════════════════════════════════════════════════════════════
async fn fetch_offsets(client: &reqwest::Client, version: &str) -> Result<RobloxOffsets, String> {
    let url = format!("{}/version-{}/offsets.hpp", OFFSETS_BASE, version);

    let response = client.get(&url)
        .header("User-Agent", "QWLEY-SHADE-Launcher")
        .send().await
        .map_err(|e| format!("Failed to fetch offsets: {e}"))?;

    if response.status() != reqwest::StatusCode::OK {
        return Err(format!("Offsets API returned HTTP {}", response.status()));
    }

    let body = response.text().await.map_err(|e| e.to_string())?;
    let lines: Vec<&str> = body.split('\n').collect();

    let mut v_ptr = String::from("Not Found");
    let mut v_rv = String::from("Not Found");
    let mut v_dev = String::from("Not Found");
    let mut v_job = String::from("Not Found");

    let mut inside_visual_engine_block = false;

    for line in &lines {
        // Убираем комментарии
        let clean = line
            .split("//").next()
            .and_then(|s| s.split("/*").next())
            .unwrap_or("")
            .trim()
            .to_string();
        let clean_lower = clean.to_ascii_lowercase();

        // Определяем блок VisualEngine
        if clean_lower.contains("visualengine") 
            && (clean_lower.contains("namespace") || clean_lower.contains("struct")) {
            inside_visual_engine_block = true;
            continue;
        }

        // Закрывающая скобка — выходим из блока
        if clean == "}" || clean == "};" {
            inside_visual_engine_block = false;
        }

        // Внутри блока VisualEngine ищем Pointer и RenderView
        if inside_visual_engine_block {
            if clean.contains("uintptr_t") && clean.contains("Pointer") {
                if let Some(hex) = extract_hex(&clean) {
                    v_ptr = hex;
                }
            }
            if clean.contains("uintptr_t") && clean.contains("RenderView") {
                if let Some(hex) = extract_hex(&clean) {
                    v_rv = hex;
                }
            }
        }

        // DeviceD3D11 и JobStart — глобально
        if clean_lower.contains("deviced3d11") {
            if let Some(hex) = extract_hex(&clean) {
                v_dev = hex;
            }
        }
        if clean_lower.contains("jobstart") {
            if let Some(hex) = extract_hex(&clean) {
                v_job = hex;
            }
        }
    }

    Ok(RobloxOffsets {
        version: version.to_string(),
        visual_engine_pointer: v_ptr,
        visual_engine_to_render_view: v_rv,
        render_view_to_device: v_dev,
        device_to_swap_chain: v_job,
    })
}

/// Извлекает hex-значение вида 0xABCDEF из строки
fn extract_hex(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if let Some(start) = lower.find("0x") {
        let hex_part = &text[start..];
        let end = hex_part[2..]
            .find(|c: char| !c.is_ascii_hexdigit())
            .map(|i| i + 2)
            .unwrap_or(hex_part.len());
        return Some(hex_part[..end].to_string());
    }
    None
}

// ═══════════════════════════════════════════════════════════════
// СОХРАНЕНИЕ ОФФСЕТОВ В JSON (скрытно)
// ═══════════════════════════════════════════════════════════════
fn save_offsets_json(folder: &Path, offsets: &RobloxOffsets) -> Result<(), String> {
    let json_path = folder.join("offsets.json");
    let json = serde_json::to_string_pretty(offsets).map_err(|e| e.to_string())?;
    fs::write(&json_path, json).map_err(|e| format!("Cannot write offsets.json: {e}"))?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// ГЛАВНАЯ ФУНКЦИЯ: определить версию → скачать → сохранить
// Вызывается тихо, без логирования в UI
// ═══════════════════════════════════════════════════════════════
async fn fetch_and_save_offsets(folder: &Path) -> Result<(), String> {
    let version = detect_roblox_version()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let offsets = fetch_offsets(&client, &version).await?;
    save_offsets_json(folder, &offsets)?;
    Ok(())
}

async fn update_files(window: &Window) -> Result<String, String> {
    let folder = app_folder()?;
    if !folder.exists() { return Err("QWLEY SHADE is not installed. Run QWLEY SHADE Installer first.".into()); }
    let current_exe = std::env::current_exe().ok();
    log(window, "[System] Checking QWLEY SHADE files...");
    let client=reqwest::Client::new();
    let response: TreeResponse=client.get(TREE_API).send().await.map_err(|e| format!("GitHub connection failed: {e}"))?
        .error_for_status().map_err(|e| format!("GitHub API error: {e}"))?.json().await.map_err(|e| e.to_string())?;
    let remote: Vec<TreeItem>=response.tree.into_iter().filter(|x| x.item_type=="blob" && safe_path(&x.path)).collect();
    let manifest_path=folder.join(".qwley-launcher-update.json");
    let old: Manifest=match tokio::fs::read_to_string(&manifest_path).await { Ok(s)=>serde_json::from_str(&s).unwrap_or_default(), Err(_)=>Manifest::default() };
    let remote_map: BTreeMap<String,String>=remote.iter().map(|x|(x.path.clone(),x.sha.clone())).collect();
    let mut changed=0u32;
    for item in remote {
        let output=folder.join(&item.path);
        if old.files.get(&item.path)==Some(&item.sha) && output.exists() { continue; }
        if let Some(ref exe) = current_exe { if output == *exe { log(window, &format!("[Skip] {} (running), will update after restart", item.path)); changed+=1; continue; } }
        let url=if lfs(&item.path) { format!("{MEDIA_ROOT}{}",encode_path(&item.path)) } else { format!("{RAW_ROOT}{}",encode_path(&item.path)) };
        let data=client.get(url).send().await.map_err(|e| format!("{}: {e}",item.path))?.error_for_status().map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
        if let Some(parent)=output.parent() { tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?; }
        tokio::fs::write(output,data).await.map_err(|e| e.to_string())?;
        changed+=1;
    }
    for old_path in old.files.keys() {
        if !remote_map.contains_key(old_path) && safe_path(old_path) {
            let file=folder.join(old_path);
            if let Some(ref exe) = current_exe { if file == *exe { continue; } }
            if file.exists() { let _=tokio::fs::remove_file(file).await; changed+=1; }
        }
    }
    let manifest=serde_json::to_string_pretty(&Manifest{files:remote_map}).map_err(|e| e.to_string())?;
    tokio::fs::write(manifest_path,manifest).await.map_err(|e| e.to_string())?;
    let result=if changed==0 { "[OK] QWLEY SHADE is up to date.".to_string() } else { "[OK] QWLEY SHADE files are ready.".to_string() };
    log(window,result.clone());
    log(window, "[Info] Awaiting launch command...");
    Ok(result)
}

#[tauri::command]
async fn check_updates(window: Window) -> Result<String, String> { update_files(&window).await }

#[cfg(windows)]
fn start_as_admin(file: &Path) -> Result<(), String> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr::null_mut};
    use winapi::um::shellapi::ShellExecuteW;
    fn wide(s: &OsStr) -> Vec<u16> { s.encode_wide().chain(Some(0)).collect() }
    let verb=wide(OsStr::new("runas")); let app=wide(file.as_os_str());
    let result=unsafe { ShellExecuteW(null_mut(),verb.as_ptr(),app.as_ptr(),null_mut(),null_mut(),1) } as usize;
    if result<=32 { Err(format!("Administrator launch was cancelled or failed ({result}).")) } else { Ok(()) }
}

#[cfg(not(windows))]
fn start_as_admin(_file: &Path) -> Result<(), String> { Err("This launcher is for Windows.".into()) }

fn qwley_already_running() -> bool {
    Command::new("tasklist").args(["/FI", "IMAGENAME eq qwleyshade.exe", "/NH"])
        .output().map(|out| String::from_utf8_lossy(&out.stdout).to_ascii_lowercase().contains("qwleyshade.exe")).unwrap_or(false)
}

#[cfg(windows)]
fn try_connect_pipe() -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::fileapi::{CreateFileW, OPEN_EXISTING};
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::winnt::{GENERIC_READ, GENERIC_WRITE};

    let wide: Vec<u16> = OsStr::new(PIPE_NAME)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE { return false; }
        CloseHandle(handle);
        true
    }
}

#[cfg(not(windows))]
fn try_connect_pipe() -> bool { false }

#[tauri::command]
async fn launch_qwley(window: Window) -> Result<(), String> {
    let folder=app_folder()?;
    if qwley_already_running() {
        log(&window, "[Info] QWLEY SHADE is already running. Launch skipped.");
        return Err("QWLEY SHADE is already running.".into());
    }
    let app=folder.join("qwleyshade.exe");
    let overlay=folder.join("eurotrucks2.exe");
    if !app.exists() { return Err("qwleyshade.exe was not found in Desktop\\qwleyshade.".into()); }
    if !overlay.exists() { return Err("eurotrucks2.exe was not found in Desktop\\qwleyshade.".into()); }
    log(&window,"Starting QWLEY SHADE with administrator approval...");
    start_as_admin(&app)?;

    // Poll pipe: up to 3 seconds, 100 ms interval
    log(&window, "[System] Connecting to pipe...");
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(3);
    let mut injected = false;
    loop {
        if try_connect_pipe() { injected = true; break; }
        if start.elapsed() >= timeout { break; }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    if injected {
        log(&window, "[OK] Injected");
    } else {
        log(&window, "[ERROR] Failed — pipe did not appear within 3 seconds");
    }

    restart_overlay_process(&folder).await?;
    log(&window, "[OK] Overlay (eurotrucks2.exe) started.");
    log(&window, "──────────────────────────────────────────────────────");

    // ══════════════════════════════════════════════════════════
    // СКРЫТНО: определяем версию Roblox и скачиваем оффсеты
    // Ничего не пишем в лог/UI — всё тихо в фоне
    // ══════════════════════════════════════════════════════════
    if injected {
        let _ = fetch_and_save_offsets(&folder).await;
    }

    if injected { Ok(()) } else { Err("Injection failed: pipe not available".into()) }
}

// Вспомогательная функция для скрытного запуска процессов (чтобы не было CMD)
#[cfg(windows)]
fn run_hidden(command: &mut Command) -> std::io::Result<std::process::Output> {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW).output()
}

#[cfg(not(windows))]
fn run_hidden(command: &mut Command) -> std::io::Result<std::process::Output> {
    command.output()
}

async fn restart_overlay_process(folder: &Path) -> Result<(), String> {
    let overlay = folder.join("eurotrucks2.exe");
    if !overlay.exists() { return Err("eurotrucks2.exe was not found in Desktop\\qwleyshade.".into()); }
    
    // Скрытно убиваем старый оверлей
    let _ = run_hidden(Command::new("taskkill").args(["/F", "/IM", "eurotrucks2.exe"]));
    
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    
    // Запускаем оверлей (он сам по себе без консоли, так как это .exe)
    Command::new(&overlay).current_dir(folder).spawn().map_err(|e| format!("Cannot start overlay: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn start_overlay(window: Window) -> Result<(), String> {
    let folder = app_folder()?;
    log(&window, "[System] Restarting QWLEY overlay...");
    restart_overlay_process(&folder).await?;
    log(&window, "[OK] Overlay started.");
    Ok(())
}

// Stops only the QWLEY overlay executable. It does not inspect or manage any third-party application.
#[tauri::command]
async fn stop_overlay(window: Window) -> Result<(), String> {
    log(&window, "[System] Stopping QWLEY overlay...");
    let _ = run_hidden(Command::new("taskkill").args(["/F", "/IM", "eurotrucks2.exe"]));
    log(&window, "[OK] Overlay stopped.");
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// НОВАЯ КОМАНДА: KILL ROBLOX (Скрытно, без CMD)
// ═══════════════════════════════════════════════════════════════
#[tauri::command]
async fn kill_roblox(window: Window) -> Result<(), String> {
    log(&window, "──────────────────────────────────────────────────────");
    log(&window, "[System] Killing game processes...");

    let processes_to_kill = [
        "eurotrucks2.exe",
        "Injector.exe",
        "robloxplayerbeta.exe",
        "RobloxPlayerLauncher.exe"
    ];

    for process in processes_to_kill.iter() {
        // Используем вспомогательную функцию, чтобы скрыть черное окно CMD
        let _ = run_hidden(
            Command::new("taskkill").args(["/F", "/IM", process])
        );
    }

    log(&window, "[OK]   All game processes terminated.");
    log(&window, "[Info] Launcher is still running.");
    log(&window, "──────────────────────────────────────────────────────");
    Ok(())
}

fn main() { 
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            check_updates, 
            launch_qwley, 
            start_overlay, 
            stop_overlay, 
            kill_roblox
        ])
        .run(tauri::generate_context!())
        .expect("QWLEY SHADE Launcher error"); 
}