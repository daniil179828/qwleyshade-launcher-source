use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};
use tauri::{AppHandle, Emitter, Manager};

const CHANGELOG_URL: &str =
    "https://raw.githubusercontent.com/daniil179828/Changelog/main/changelog.json";
const REPO_ARCHIVE_URL: &str =
    "https://github.com/daniil179828/QWLEY-SHADE/archive/refs/heads/main.zip";
const INSTALL_DIR_NAME: &str = "QWLEY SHADE";

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct UpdateInfo {
    local: String,
    remote: String,
    needs_update: bool,
    installed: bool,
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

fn install_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .desktop_dir()
        .map_err(|e| e.to_string())?
        .join(INSTALL_DIR_NAME);
    Ok(dir)
}

fn is_installed(app: &AppHandle) -> bool {
    install_dir(app)
        .map(|d| d.is_dir() && std::fs::read_dir(&d).map(|mut it| it.next().is_some()).unwrap_or(false))
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

const LFS_ENDPOINT: &str =
    "https://github.com/daniil179828/QWLEY-SHADE.git/info/lfs/objects/batch";

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
        if stripped == ".gitattributes" {
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
                    Ok(_) => { written = true; break; }
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

fn find_setup(dest: &std::path::Path) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dest).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false)
            && p
                .file_name()
                .map(|n| n.to_string_lossy().contains("ReShade_Setup"))
                .unwrap_or(false)
        {
            return Some(p);
        }
    }
    None
}

fn install(app: AppHandle, launch_reshade: bool) -> Result<String, String> {
    let dest = install_dir(&app)?;
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    let res = ureq::get(REPO_ARCHIVE_URL)
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
                    Progress { percent, downloaded, total },
                );
            }
        }
    }

    let _ = app.emit(
        "download-progress",
        Progress { percent: 100.0, downloaded, total },
    );

    extract(&bytes, &dest)?;
    let _ = std::fs::write(dest.join(".qwley_installed"), "1");

    if launch_reshade {
        if let Some(setup) = find_setup(&dest) {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let via_cmd = std::process::Command::new("cmd")
                    .args(["/C", "start", "", &setup.to_string_lossy()])
                    .creation_flags(0x08000000)
                    .spawn()
                    .is_ok();
                if !via_cmd {
                    let _ = std::process::Command::new(&setup).spawn();
                }
            }
            #[cfg(not(windows))]
            {
                let _ = std::process::Command::new(&setup).spawn();
            }
        }
    }

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

    Ok(UpdateInfo {
        needs_update: true,
        installed: is_installed(&app),
        local,
        remote,
    })
}

#[tauri::command]
async fn get_changelog() -> Result<Vec<ChangelogEntry>, String> {
    tauri::async_runtime::spawn_blocking(fetch_changelog)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn start_update(app: AppHandle, launch_reshade: Option<bool>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || install(app, launch_reshade.unwrap_or(false)))
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
        let version = data["version"].as_str().ok_or("missing version")?.to_string();
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
        .invoke_handler(tauri::generate_handler![
            check_update,
            start_update,
            get_changelog,
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
