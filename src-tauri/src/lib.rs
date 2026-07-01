use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::path::{Path, PathBuf};
use std::time::{Instant, Duration};
use tokio::sync::oneshot;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};

// ── Types ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct GameVersion {
    pub uuid: String,
    pub version: String,
    pub url: String,
    pub launch_path: String,
    pub size_bytes: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub description: String,
    pub versions: Vec<GameVersion>,
    pub notes: Option<String>,
    pub title_notes: Option<String>,
    pub app_id: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ServerModpack {
    pub uuid: String,
    pub id: u32,
    pub game_title: String,
    pub modpack_title: String,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub uploaded_at: String,
    pub url: String,
    pub notes: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub struct QueryResult {
    pub games: Vec<Game>,
    pub modpacks: Vec<ServerModpack>,
}

#[derive(serde::Deserialize)]
struct RawServerGame {
    id: u32,
    uuid: Option<String>,
    title: String,
    version: String,
    file_name: String,
    file_size_bytes: u64,
    launch_exe: String,
    app_id: Option<String>,
    notes: Option<String>,
    title_notes: Option<String>,
    uploaded_at: String,
}

#[derive(serde::Deserialize)]
struct RawServerModpack {
    id: u32,
    uuid: Option<String>,
    game_title: String,
    modpack_title: String,
    file_name: String,
    file_size_bytes: u64,
    notes: Option<String>,
    uploaded_at: String,
}

#[derive(serde::Deserialize)]
struct RawServerResponse {
    games: Vec<RawServerGame>,
    modpacks: Vec<RawServerModpack>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct DownloadStatus {
    pub status: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ModpackStatus {
    pub status: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub installed_uploaded_at: Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct DownloadProgressPayload {
    game_id: String,
    version: String,
    modpack_title: Option<String>,
    downloaded_bytes: u64,
    total_bytes: u64,
    speed_bytes_per_sec: f64,
    status: String,
    error: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct StagedState {
    pub game_id: String,
    pub version: String,
    pub modpack: Option<String>,
    pub swap_phase: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub struct StorageSizes {
    pub game_id: String,
    pub game_name: String,
    pub total_bytes: u64,
    pub versions: Vec<VersionSize>,
    pub modpacks: Vec<ModpackSize>,
}

#[derive(serde::Serialize, Clone)]
pub struct VersionSize {
    pub version: String,
    pub size_bytes: u64,
    pub staged: bool,
}

#[derive(serde::Serialize, Clone)]
pub struct ModpackSize {
    pub modpack_title: String,
    pub size_bytes: u64,
    pub staged: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ModpackManifest {
    pub modpack_title: String,
    pub files: Vec<String>,
}

// ── Download Manager ───────────────────────────────────────────────────

struct DownloadTask {
    job_key: String,
    is_modpack: bool,
    app: AppHandle,
    game_id: String,
    game_name: String,
    version: String,
    modpack_title: Option<String>,
    url: String,
    game_folder: String,
    size_bytes: u64,
    api_key: String,
    cancel_rx: oneshot::Receiver<()>,
}

pub struct DownloadManager {
    jobs: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
    queue: Arc<Mutex<VecDeque<DownloadTask>>>,
    active_count: Arc<Mutex<u32>>,
    queue_has_work: Arc<Mutex<bool>>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            active_count: Arc::new(Mutex::new(0)),
            queue_has_work: Arc::new(Mutex::new(false)),
        }
    }
}

// ── Path helpers ───────────────────────────────────────────────────────

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .trim_end_matches(|c: char| c == '.' || c == ' ')
        .to_string()
}

fn cache_root(game_folder: &str) -> PathBuf {
    PathBuf::from(game_folder).join(".nakama")
}

fn downloads_dir(game_folder: &str) -> PathBuf {
    cache_root(game_folder).join("_downloads")
}

fn game_cache_dir(game_folder: &str, game_name: &str) -> PathBuf {
    cache_root(game_folder).join(sanitize_filename(game_name))
}

fn version_cache_dir(game_folder: &str, game_name: &str, version: &str) -> PathBuf {
    game_cache_dir(game_folder, game_name)
        .join("versions")
        .join(sanitize_filename(version))
}

fn modpack_cache_dir(game_folder: &str, game_name: &str, modpack_title: &str) -> PathBuf {
    game_cache_dir(game_folder, game_name)
        .join("modpacks")
        .join(sanitize_filename(modpack_title))
}

fn vanilla_dir(game_folder: &str, game_name: &str, version: &str) -> PathBuf {
    version_cache_dir(game_folder, game_name, version).join(".vanilla")
}

fn staging_dir(game_folder: &str, game_name: &str) -> PathBuf {
    PathBuf::from(game_folder).join(game_name)
}

fn state_file_path(staging: &Path) -> PathBuf {
    staging.join(".nakama-state")
}

fn manifest_path(game_folder: &str, game_name: &str, modpack_title: &str) -> PathBuf {
    modpack_cache_dir(game_folder, game_name, modpack_title).join(".manifest.json")
}

// ── Utility functions ──────────────────────────────────────────────────

fn url_encode(input: &str) -> String {
    let mut encoded = String::new();
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => {
                encoded.push_str("%20");
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

fn read_state(staging: &Path) -> Option<StagedState> {
    let path = state_file_path(staging);
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            return serde_json::from_str(&content).ok();
        }
    }
    None
}

fn write_state(staging: &Path, state: &StagedState) -> Result<(), String> {
    std::fs::create_dir_all(staging).map_err(|e| format!("Failed to create staging dir: {}", e))?;
    let path = state_file_path(staging);
    let json = serde_json::to_string(state).map_err(|e| format!("Failed to serialize state: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write state file: {}", e))?;
    Ok(())
}

fn read_manifest(game_folder: &str, game_name: &str, modpack_title: &str) -> Option<ModpackManifest> {
    let path = manifest_path(game_folder, game_name, modpack_title);
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            return serde_json::from_str(&content).ok();
        }
    }
    None
}

fn write_manifest(game_folder: &str, game_name: &str, modpack_title: &str, manifest: &ModpackManifest) -> Result<(), String> {
    let path = manifest_path(game_folder, game_name, modpack_title);
    let parent = path.parent().unwrap();
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create modpack cache dir: {}", e))?;
    let json = serde_json::to_string(manifest).map_err(|e| format!("Failed to serialize manifest: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write manifest: {}", e))?;
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    let mut total = 0u64;
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn move_dir(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.exists() {
        return Ok(());
    }
    if dst.exists() {
        std::fs::remove_dir_all(dst).map_err(|e| format!("Failed to remove target before move: {}", e))?;
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent dir: {}", e))?;
    }
    std::fs::rename(src, dst).map_err(|e| format!("Failed to move directory: {}", e))?;
    Ok(())
}

fn move_file(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.exists() {
        return Ok(());
    }
    if dst.exists() {
        std::fs::remove_file(dst).map_err(|e| format!("Failed to remove target file: {}", e))?;
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent dir: {}", e))?;
    }
    std::fs::rename(src, dst).map_err(|e| format!("Failed to move file: {}", e))?;
    Ok(())
}

// ── Commands ───────────────────────────────────────────────────────────

#[tauri::command]
async fn get_games_list(server_url: String, api_key: String) -> Result<QueryResult, String> {
    if server_url.trim().is_empty() || server_url == "mock" {
        let mock_json = include_str!("mock_games.json");
        let games: Vec<Game> = serde_json::from_str(mock_json)
            .map_err(|e| format!("Failed to parse mock games: {}", e))?;

        let mock_modpacks = vec![
            ServerModpack {
                uuid: "mock-modpack-uuid-001".to_string(),
                id: 1,
                game_title: "Cosmo Explorer".to_string(),
                modpack_title: "Cool Mod".to_string(),
                file_name: "Cosmo_Explorer_Cool_Mod.zip".to_string(),
                file_size_bytes: 15_000_000,
                uploaded_at: "2026-06-28T02:00:00Z".to_string(),
                url: "mock://cosmo-explorer/modpack/cool-mod".to_string(),
                notes: Some("Adds new planets and ships.".to_string()),
            },
            ServerModpack {
                uuid: "mock-modpack-uuid-002".to_string(),
                id: 2,
                game_title: "Cosmo Explorer".to_string(),
                modpack_title: "HD Textures".to_string(),
                file_name: "Cosmo_Explorer_HD_Textures.zip".to_string(),
                file_size_bytes: 25_000_000,
                uploaded_at: "2026-06-28T03:00:00Z".to_string(),
                url: "mock://cosmo-explorer/modpack/hd-textures".to_string(),
                notes: None,
            },
            ServerModpack {
                uuid: "mock-modpack-uuid-003".to_string(),
                id: 3,
                game_title: "Cyber Sentinel".to_string(),
                modpack_title: "Redux Mod".to_string(),
                file_name: "Cyber_Sentinel_Redux_Mod.zip".to_string(),
                file_size_bytes: 35_000_000,
                uploaded_at: "2026-06-28T04:00:00Z".to_string(),
                url: "mock://cyber-sentinel/modpack/redux-mod".to_string(),
                notes: Some("Complete gameplay overhaul with new missions.".to_string()),
            },
        ];

        return Ok(QueryResult { games, modpacks: mock_modpacks });
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let base_url = if server_url.ends_with("/query") {
        server_url.trim_end_matches("/query").to_string()
    } else {
        server_url.clone()
    };
    let query_url = format!("{}/query", base_url.trim_end_matches('/'));
    let base_url = base_url.trim_end_matches('/').to_string();

    let mut request = client.get(&query_url);
    if !api_key.is_empty() {
        request = request.header("X-API-Key", &api_key);
    }

    let response = request.send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Server returned error ({}): {}", status, err_text));
    }

    let raw_resp = response.json::<RawServerResponse>()
        .await
        .map_err(|e| format!("Failed to parse server response: {}", e))?;

    let mut raw_games = raw_resp.games;
    raw_games.sort_by(|a, b| b.uploaded_at.cmp(&a.uploaded_at));

    let mut games_map: HashMap<String, Game> = HashMap::new();

    for raw_game in raw_games {
        let game_id = raw_game.title.to_lowercase().replace(" ", "-");
        let uuid = raw_game.uuid.unwrap_or_else(|| format!("{}-{}", game_id, raw_game.version));
        let download_url = format!("{}/download/game/{}", base_url, url_encode(&uuid));

        let ver = GameVersion {
            uuid,
            version: raw_game.version.clone(),
            url: download_url,
            launch_path: raw_game.launch_exe.clone(),
            size_bytes: raw_game.file_size_bytes,
        };

        if let Some(existing_game) = games_map.get_mut(&raw_game.title) {
            existing_game.versions.push(ver);
        } else {
            let game = Game {
                id: game_id,
                name: raw_game.title.clone(),
                icon_url: None,
                description: format!("Uploaded at {}", raw_game.uploaded_at),
                versions: vec![ver],
                notes: raw_game.notes.clone(),
                title_notes: raw_game.title_notes.clone(),
                app_id: raw_game.app_id.clone(),
            };
            games_map.insert(raw_game.title.clone(), game);
        }
    }

    let games: Vec<Game> = games_map.into_values().collect();

    let modpacks = raw_resp.modpacks.into_iter().map(|raw_mp| {
        let mp_uuid = raw_mp.uuid.unwrap_or_else(|| format!("mp-{}-{}", raw_mp.id, raw_mp.modpack_title));
        let download_url = format!("{}/download/modpack/{}", base_url, url_encode(&mp_uuid));

        ServerModpack {
            uuid: mp_uuid,
            id: raw_mp.id,
            game_title: raw_mp.game_title,
            modpack_title: raw_mp.modpack_title,
            file_name: raw_mp.file_name,
            file_size_bytes: raw_mp.file_size_bytes,
            uploaded_at: raw_mp.uploaded_at,
            url: download_url,
            notes: raw_mp.notes,
        }
    }).collect();

    Ok(QueryResult { games, modpacks })
}

#[tauri::command]
async fn get_staged_state(
    game_folder: String,
    game_name: String,
) -> Result<Option<StagedState>, String> {
    let staging = staging_dir(&game_folder, &game_name);
    Ok(read_state(&staging))
}

#[tauri::command]
async fn get_download_status(
    game_manager: tauri::State<'_, DownloadManager>,
    game_folder: String,
    game_name: String,
    version: String,
    game_id: String,
) -> Result<DownloadStatus, String> {
    let version_dir = version_cache_dir(&game_folder, &game_name, &version);
    let zip_tmp = downloads_dir(&game_folder).join(format!("{}_{}.zip.tmp", sanitize_filename(&game_name), sanitize_filename(&version)));

    let job_key = format!("{}:{}", game_id, version);
    let is_active = {
        let jobs = game_manager.jobs.lock().unwrap();
        jobs.contains_key(&job_key)
    };

    if is_active {
        let downloaded = tokio::fs::metadata(&zip_tmp).await.map(|m| m.len()).unwrap_or(0);
        return Ok(DownloadStatus {
            status: "Downloading".to_string(),
            downloaded_bytes: downloaded,
            total_bytes: 0,
        });
    }

    if version_dir.exists() && version_dir.is_dir() {
        return Ok(DownloadStatus {
            status: "Downloaded".to_string(),
            downloaded_bytes: 0,
            total_bytes: 0,
        });
    }

    // Check if version is currently staged (moved out of cache into staging)
    let staging = staging_dir(&game_folder, &game_name);
    if let Some(state) = read_state(&staging) {
        if state.version == version {
            return Ok(DownloadStatus {
                status: "Downloaded".to_string(),
                downloaded_bytes: 0,
                total_bytes: 0,
            });
        }
    }

    if zip_tmp.exists() {
        let downloaded = tokio::fs::metadata(&zip_tmp).await.map(|m| m.len()).unwrap_or(0);
        if downloaded > 0 {
            return Ok(DownloadStatus {
                status: "Paused".to_string(),
                downloaded_bytes: downloaded,
                total_bytes: 0,
            });
        }
    }

    Ok(DownloadStatus {
        status: "NotDownloaded".to_string(),
        downloaded_bytes: 0,
        total_bytes: 0,
    })
}

#[tauri::command]
async fn get_modpack_status(
    game_manager: tauri::State<'_, DownloadManager>,
    game_folder: String,
    game_name: String,
    version: String,
    game_id: String,
    modpack_title: String,
) -> Result<ModpackStatus, String> {
    let mp_dir = modpack_cache_dir(&game_folder, &game_name, &modpack_title);
    let zip_tmp = downloads_dir(&game_folder).join(format!("{}_{}_modpack_{}.zip.tmp",
        sanitize_filename(&game_name), sanitize_filename(&version), sanitize_filename(&modpack_title)));

    let job_key = format!("{}:{}:{}", game_id, version, modpack_title);
    let is_active = {
        let jobs = game_manager.jobs.lock().unwrap();
        jobs.contains_key(&job_key)
    };

    if is_active {
        let downloaded = tokio::fs::metadata(&zip_tmp).await.map(|m| m.len()).unwrap_or(0);
        return Ok(ModpackStatus {
            status: "Downloading".to_string(),
            downloaded_bytes: downloaded,
            total_bytes: 0,
            installed_uploaded_at: None,
        });
    }

    if mp_dir.exists() && mp_dir.is_dir() && manifest_path(&game_folder, &game_name, &modpack_title).exists() {
        return Ok(ModpackStatus {
            status: "Downloaded".to_string(),
            downloaded_bytes: 0,
            total_bytes: 0,
            installed_uploaded_at: None,
        });
    }

    // Check if modpack is currently applied (moved from cache to staging)
    let staging = staging_dir(&game_folder, &game_name);
    if let Some(state) = read_state(&staging) {
        if state.version == version && state.modpack.as_deref() == Some(&modpack_title) {
            return Ok(ModpackStatus {
                status: "Downloaded".to_string(),
                downloaded_bytes: 0,
                total_bytes: 0,
                installed_uploaded_at: None,
            });
        }
    }

    if zip_tmp.exists() {
        let downloaded = tokio::fs::metadata(&zip_tmp).await.map(|m| m.len()).unwrap_or(0);
        if downloaded > 0 {
            return Ok(ModpackStatus {
                status: "Paused".to_string(),
                downloaded_bytes: downloaded,
                total_bytes: 0,
                installed_uploaded_at: None,
            });
        }
    }

    Ok(ModpackStatus {
        status: "NotDownloaded".to_string(),
        downloaded_bytes: 0,
        total_bytes: 0,
        installed_uploaded_at: None,
    })
}

#[tauri::command]
async fn get_storage_sizes(
    game_folder: String,
    games_json: String,
) -> Result<Vec<StorageSizes>, String> {
    let games: Vec<Game> = serde_json::from_str(&games_json)
        .map_err(|e| format!("Failed to parse games JSON: {}", e))?;

    let mut results = Vec::new();
    for game in &games {
        let cache = game_cache_dir(&game_folder, &game.name);
        let staging = staging_dir(&game_folder, &game.name);
        let staged = read_state(&staging);

        let mut versions = Vec::new();
        let versions_dir = cache.join("versions");
        if versions_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&versions_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let ver_name = entry.file_name().to_string_lossy().to_string();
                        let size = dir_size(&entry.path());
                        let is_staged = staged.as_ref().map_or(false, |s| s.version == ver_name);
                        versions.push(VersionSize {
                            version: ver_name,
                            size_bytes: size,
                            staged: is_staged,
                        });
                    }
                }
            }
        }

        // Include staged version if not already in cache list
        if let Some(ref s) = staged {
            if !versions.iter().any(|v| v.version == s.version) && staging.exists() {
                let ver_size = if s.modpack.is_some() {
                    // Subtract modpack files from staging size
                    let mp_size = if let Some(ref mp) = s.modpack {
                        let manifest = read_manifest(&game_folder, &game.name, mp);
                        manifest.map_or(0, |m| {
                            m.files.iter().filter_map(|f| {
                                let p = staging.join(f);
                                if p.exists() { std::fs::metadata(&p).ok().map(|meta| meta.len()) } else { None }
                            }).sum::<u64>()
                        })
                    } else { 0 };
                    dir_size(&staging).saturating_sub(mp_size)
                } else {
                    dir_size(&staging)
                };
                versions.push(VersionSize {
                    version: s.version.clone(),
                    size_bytes: ver_size,
                    staged: true,
                });
            }
            // Also update size for cached version that IS staged (it was 0 since dir was moved)
            for v in &mut versions {
                if v.staged {
                    let ver_size = if s.modpack.is_some() {
                        let mp_size = if let Some(ref mp) = s.modpack {
                            let manifest = read_manifest(&game_folder, &game.name, mp);
                            manifest.map_or(0, |m| {
                                m.files.iter().filter_map(|f| {
                                    let p = staging.join(f);
                                    if p.exists() { std::fs::metadata(&p).ok().map(|meta| meta.len()) } else { None }
                                }).sum::<u64>()
                            })
                        } else { 0 };
                        dir_size(&staging).saturating_sub(mp_size)
                    } else {
                        dir_size(&staging)
                    };
                    v.size_bytes = ver_size;
                }
            }
        }

        let mut modpacks = Vec::new();
        let mps_dir = cache.join("modpacks");
        if mps_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&mps_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let mp_name = entry.file_name().to_string_lossy().to_string();
                        let size = dir_size(&entry.path());
                        let is_staged = staged.as_ref().map_or(false, |s| s.modpack.as_deref() == Some(&mp_name));
                        modpacks.push(ModpackSize {
                            modpack_title: mp_name,
                            size_bytes: size,
                            staged: is_staged,
                        });
                    }
                }
            }
        }

        let staging_size = if staging.exists() { dir_size(&staging) } else { 0 };
        let cache_size = dir_size(&cache);
        let total = staging_size + cache_size;

        results.push(StorageSizes {
            game_id: game.id.clone(),
            game_name: game.name.clone(),
            total_bytes: total,
            versions,
            modpacks,
        });
    }

    Ok(results)
}

// ── Pause ──────────────────────────────────────────────────────────────

#[tauri::command]
fn pause_download(
    game_manager: tauri::State<'_, DownloadManager>,
    game_id: String,
    version: String,
) -> Result<(), String> {
    let job_key = format!("{}:{}", game_id, version);
    let mut jobs = game_manager.jobs.lock().unwrap();
    if let Some(cancel_tx) = jobs.remove(&job_key) {
        let _ = cancel_tx.send(());
        Ok(())
    } else {
        Err("No active download found for this game version".to_string())
    }
}

#[tauri::command]
fn pause_download_modpack(
    game_manager: tauri::State<'_, DownloadManager>,
    game_id: String,
    version: String,
    modpack_title: String,
) -> Result<(), String> {
    let job_key = format!("{}:{}:{}", game_id, version, modpack_title);
    let mut jobs = game_manager.jobs.lock().unwrap();
    if let Some(cancel_tx) = jobs.remove(&job_key) {
        let _ = cancel_tx.send(());
        Ok(())
    } else {
        Err("No active download found for this modpack".to_string())
    }
}

#[tauri::command]
async fn cancel_download(
    game_manager: tauri::State<'_, DownloadManager>,
    game_folder: String,
    game_name: String,
    version: String,
    game_id: String,
) -> Result<(), String> {
    // Try to pause first
    let job_key = format!("{}:{}", game_id, version);
    {
        let mut jobs = game_manager.jobs.lock().unwrap();
        if let Some(cancel_tx) = jobs.remove(&job_key) {
            let _ = cancel_tx.send(());
        }
    }

    // Delete partial download file
    let zip_tmp = downloads_dir(&game_folder).join(format!("{}_{}.zip.tmp",
        sanitize_filename(&game_name), sanitize_filename(&version)));
    if zip_tmp.exists() {
        tokio::fs::remove_file(&zip_tmp).await
            .map_err(|e| format!("Failed to delete temp file: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
async fn cancel_download_modpack(
    game_manager: tauri::State<'_, DownloadManager>,
    game_folder: String,
    game_name: String,
    version: String,
    game_id: String,
    modpack_title: String,
) -> Result<(), String> {
    let job_key = format!("{}:{}:{}", game_id, version, modpack_title);
    {
        let mut jobs = game_manager.jobs.lock().unwrap();
        if let Some(cancel_tx) = jobs.remove(&job_key) {
            let _ = cancel_tx.send(());
        }
    }

    let zip_tmp = downloads_dir(&game_folder).join(format!("{}_{}_modpack_{}.zip.tmp",
        sanitize_filename(&game_name), sanitize_filename(&version), sanitize_filename(&modpack_title)));
    if zip_tmp.exists() {
        tokio::fs::remove_file(&zip_tmp).await
            .map_err(|e| format!("Failed to delete temp file: {}", e))?;
    }
    Ok(())
}

// ── Download Queue ─────────────────────────────────────────────────────

async fn process_download_queue(manager: Arc<DownloadManager>) {
    loop {
        let task = {
            let mut queue = manager.queue.lock().unwrap();
            queue.pop_front()
        };

        let task = match task {
            Some(t) => t,
            None => {
                let mut has_work = manager.queue_has_work.lock().unwrap();
                *has_work = false;
                return;
            }
        };

        let result = if task.is_modpack {
            perform_modpack_download(
                task.app.clone(), task.game_id.clone(), task.game_name.clone(),
                task.version.clone(), task.modpack_title.clone().unwrap_or_default(),
                task.url.clone(), task.game_folder.clone(), task.size_bytes,
                task.api_key.clone(), task.cancel_rx,
            ).await
        } else {
            perform_download(
                task.app.clone(), task.game_id.clone(), task.game_name.clone(),
                task.version.clone(), task.url.clone(), task.game_folder.clone(),
                task.size_bytes, task.api_key.clone(), task.cancel_rx,
            ).await
        };

        // Drop locks before awaiting
        {
            let mut jobs = manager.jobs.lock().unwrap();
            jobs.remove(&task.job_key);
        }
        {
            let mut active = manager.active_count.lock().unwrap();
            *active = active.saturating_sub(1);
        }

        if let Err(e) = result {
            let _ = task.app.emit("download-progress", DownloadProgressPayload {
                game_id: task.game_id.clone(),
                version: task.version.clone(),
                modpack_title: task.modpack_title.clone(),
                downloaded_bytes: 0,
                total_bytes: task.size_bytes,
                speed_bytes_per_sec: 0.0,
                status: "failed".to_string(),
                error: Some(e),
            });
        }

        // 1s gap between downloads
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn enqueue_download(manager: Arc<DownloadManager>, task: DownloadTask) {
    let mut queue = manager.queue.lock().unwrap();
    queue.push_back(task);
    let mut active = manager.active_count.lock().unwrap();
    *active += 1;

    let mut has_work = manager.queue_has_work.lock().unwrap();
    if !*has_work {
        *has_work = true;
        drop(has_work);
        drop(queue);
        drop(active);
        let mgr = manager.clone();
        tokio::spawn(async move {
            process_download_queue(mgr).await;
        });
    }
}

// ── Start Download ─────────────────────────────────────────────────────

#[tauri::command]
async fn start_download(
    app: AppHandle,
    game_manager: tauri::State<'_, DownloadManager>,
    game_id: String,
    game_name: String,
    version: String,
    url: String,
    game_folder: String,
    size_bytes: u64,
    api_key: String,
    _uuid: String,
) -> Result<(), String> {
    let job_key = format!("{}:{}", game_id, version);
    let mut jobs = game_manager.jobs.lock().unwrap();

    if jobs.contains_key(&job_key) {
        return Err("Download already in progress".to_string());
    }

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    jobs.insert(job_key.clone(), cancel_tx);

    let manager = Arc::new(DownloadManager {
        jobs: Arc::clone(&game_manager.jobs),
        queue: Arc::clone(&game_manager.queue),
        active_count: Arc::clone(&game_manager.active_count),
        queue_has_work: Arc::clone(&game_manager.queue_has_work),
    });

    let task = DownloadTask {
        job_key,
        is_modpack: false,
        app,
        game_id,
        game_name,
        version,
        modpack_title: None,
        url,
        game_folder,
        size_bytes,
        api_key,
        cancel_rx,
    };

    enqueue_download(manager, task);
    Ok(())
}

#[tauri::command]
async fn start_download_modpack(
    app: AppHandle,
    game_manager: tauri::State<'_, DownloadManager>,
    game_id: String,
    game_name: String,
    version: String,
    modpack_title: String,
    url: String,
    game_folder: String,
    size_bytes: u64,
    api_key: String,
    _uuid: String,
) -> Result<(), String> {
    let job_key = format!("{}:{}:{}", game_id, version, modpack_title);
    let mut jobs = game_manager.jobs.lock().unwrap();

    if jobs.contains_key(&job_key) {
        return Err("Modpack download already in progress".to_string());
    }

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    jobs.insert(job_key.clone(), cancel_tx);

    let manager = Arc::new(DownloadManager {
        jobs: Arc::clone(&game_manager.jobs),
        queue: Arc::clone(&game_manager.queue),
        active_count: Arc::clone(&game_manager.active_count),
        queue_has_work: Arc::clone(&game_manager.queue_has_work),
    });

    let task = DownloadTask {
        job_key,
        is_modpack: true,
        app,
        game_id,
        game_name,
        version,
        modpack_title: Some(modpack_title),
        url,
        game_folder,
        size_bytes,
        api_key,
        cancel_rx,
    };

    enqueue_download(manager, task);
    Ok(())
}

// ── Perform Downloads ──────────────────────────────────────────────────

async fn perform_download(
    app: AppHandle,
    game_id: String,
    game_name: String,
    version: String,
    url: String,
    game_folder: String,
    size_bytes: u64,
    api_key: String,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    let d_dir = downloads_dir(&game_folder);
    tokio::fs::create_dir_all(&d_dir).await
        .map_err(|e| format!("Failed to create downloads dir: {}", e))?;

    let zip_tmp = d_dir.join(format!("{}_{}.zip.tmp", sanitize_filename(&game_name), sanitize_filename(&version)));

    if url.starts_with("mock://") {
        let target_dir = version_cache_dir(&game_folder, &game_name, &version);
        perform_mock_download(app, game_id, game_name, version, zip_tmp, target_dir, size_bytes, cancel_rx).await?;
        return Ok(());
    }

    let client = reqwest::Client::new();
    let mut downloaded = 0u64;
    if zip_tmp.exists() {
        if let Ok(metadata) = tokio::fs::metadata(&zip_tmp).await {
            downloaded = metadata.len();
        }
    }

    let mut request = client.get(&url);
    if downloaded > 0 {
        request = request.header("Range", format!("bytes={}-", downloaded));
    }
    if !api_key.is_empty() {
        request = request.header("X-API-Key", &api_key);
    }

    let response = request.send().await.map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();

    let resumed = downloaded > 0;
    let mut file = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        OpenOptions::new().write(true).append(true).open(&zip_tmp).await
            .map_err(|e| format!("Failed to open temp file in append mode: {}", e))?
    } else if status.is_success() {
        if resumed {
            let _ = tokio::fs::remove_file(&zip_tmp).await;
            downloaded = 0;
        }
        OpenOptions::new().write(true).create(true).truncate(true).open(&zip_tmp).await
            .map_err(|e| format!("Failed to create temp file: {}", e))?
    } else {
        return Err(format!("Server returned HTTP status {}", status));
    };

    let total_bytes = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        response.headers().get(reqwest::header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.rfind('/').and_then(|i| s[i+1..].trim().parse::<u64>().ok()))
            .unwrap_or(size_bytes)
    } else {
        response.content_length().unwrap_or(size_bytes)
    };

    let mut stream = response.bytes_stream();
    let mut last_emit = Instant::now();
    let mut last_downloaded = downloaded;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: None,
        downloaded_bytes: downloaded, total_bytes, speed_bytes_per_sec: 0.0,
        status: "downloading".to_string(), error: None,
    });

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                let _ = file.flush().await;
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(), version: version.clone(), modpack_title: None,
                    downloaded_bytes: downloaded, total_bytes, speed_bytes_per_sec: 0.0,
                    status: "paused".to_string(), error: None,
                });
                return Ok(());
            }
            chunk_result = stream.next() => {
                match chunk_result {
                    Some(Ok(chunk)) => {
                        file.write_all(&chunk).await.map_err(|e| format!("Failed to write chunk: {}", e))?;
                        downloaded += chunk.len() as u64;

                        let now = Instant::now();
                        let elapsed = now.duration_since(last_emit);
                        if elapsed >= Duration::from_millis(300) {
                            let speed = (downloaded - last_downloaded) as f64 / elapsed.as_secs_f64();
                            let _ = app.emit("download-progress", DownloadProgressPayload {
                                game_id: game_id.clone(), version: version.clone(), modpack_title: None,
                                downloaded_bytes: downloaded, total_bytes, speed_bytes_per_sec: speed,
                                status: "downloading".to_string(), error: None,
                            });
                            last_emit = now;
                            last_downloaded = downloaded;
                        }
                    }
                    Some(Err(e)) => return Err(format!("Stream error: {}", e)),
                    None => break,
                }
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: None,
        downloaded_bytes: downloaded, total_bytes, speed_bytes_per_sec: 0.0,
        status: "extracting".to_string(), error: None,
    });

    let target_dir = version_cache_dir(&game_folder, &game_name, &version);
    let zip_clone = zip_tmp.clone();
    let target_clone = target_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip(&zip_clone, &target_clone))
        .await.map_err(|e| format!("Extraction task joined with error: {}", e))??;

    let _ = tokio::fs::remove_file(&zip_tmp).await;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: None,
        downloaded_bytes: total_bytes, total_bytes, speed_bytes_per_sec: 0.0,
        status: "completed".to_string(), error: None,
    });

    Ok(())
}

async fn perform_modpack_download(
    app: AppHandle,
    game_id: String,
    game_name: String,
    version: String,
    modpack_title: String,
    url: String,
    game_folder: String,
    size_bytes: u64,
    api_key: String,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    let d_dir = downloads_dir(&game_folder);
    tokio::fs::create_dir_all(&d_dir).await
        .map_err(|e| format!("Failed to create downloads dir: {}", e))?;

    let zip_tmp = d_dir.join(format!("{}_{}_modpack_{}.zip.tmp",
        sanitize_filename(&game_name), sanitize_filename(&version), sanitize_filename(&modpack_title)));

    if url.starts_with("mock://") {
        let target_dir = modpack_cache_dir(&game_folder, &game_name, &modpack_title);
        perform_mock_modpack_download(app, game_id, version, modpack_title, zip_tmp, target_dir, size_bytes, cancel_rx).await?;
        return Ok(());
    }

    let client = reqwest::Client::new();
    let mut downloaded = 0u64;
    if zip_tmp.exists() {
        if let Ok(metadata) = tokio::fs::metadata(&zip_tmp).await {
            downloaded = metadata.len();
        }
    }

    let mut request = client.get(&url);
    if downloaded > 0 {
        request = request.header("Range", format!("bytes={}-", downloaded));
    }
    if !api_key.is_empty() {
        request = request.header("X-API-Key", &api_key);
    }

    let response = request.send().await.map_err(|e| format!("Request failed: {}", e))?;
    let status = response.status();

    let resumed = downloaded > 0;
    let mut file = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        OpenOptions::new().write(true).append(true).open(&zip_tmp).await
            .map_err(|e| format!("Failed to open temp file in append mode: {}", e))?
    } else if status.is_success() {
        if resumed {
            let _ = tokio::fs::remove_file(&zip_tmp).await;
            downloaded = 0;
        }
        OpenOptions::new().write(true).create(true).truncate(true).open(&zip_tmp).await
            .map_err(|e| format!("Failed to create temp file: {}", e))?
    } else {
        return Err(format!("Server returned HTTP status {}", status));
    };

    let total_bytes = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        response.headers().get(reqwest::header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.rfind('/').and_then(|i| s[i+1..].trim().parse::<u64>().ok()))
            .unwrap_or(size_bytes)
    } else {
        response.content_length().unwrap_or(size_bytes)
    };

    let mut stream = response.bytes_stream();
    let mut last_emit = Instant::now();
    let mut last_downloaded = downloaded;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: downloaded, total_bytes, speed_bytes_per_sec: 0.0,
        status: "downloading".to_string(), error: None,
    });

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
                    downloaded_bytes: downloaded, total_bytes, speed_bytes_per_sec: 0.0,
                    status: "paused".to_string(), error: None,
                });
                return Ok(());
            }
            chunk_result = stream.next() => {
                match chunk_result {
                    Some(Ok(chunk)) => {
                        file.write_all(&chunk).await.map_err(|e| format!("Failed to write chunk: {}", e))?;
                        downloaded += chunk.len() as u64;

                        let now = Instant::now();
                        let elapsed = now.duration_since(last_emit);
                        if elapsed >= Duration::from_millis(300) {
                            let speed = (downloaded - last_downloaded) as f64 / elapsed.as_secs_f64();
                            let _ = app.emit("download-progress", DownloadProgressPayload {
                                game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
                                downloaded_bytes: downloaded, total_bytes, speed_bytes_per_sec: speed,
                                status: "downloading".to_string(), error: None,
                            });
                            last_emit = now;
                            last_downloaded = downloaded;
                        }
                    }
                    Some(Err(e)) => return Err(format!("Stream error: {}", e)),
                    None => break,
                }
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: downloaded, total_bytes, speed_bytes_per_sec: 0.0,
        status: "extracting".to_string(), error: None,
    });

    let target_dir = modpack_cache_dir(&game_folder, &game_name, &modpack_title);
    let zip_clone = zip_tmp.clone();
    let target_clone = target_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip(&zip_clone, &target_clone))
        .await.map_err(|e| format!("Extraction task joined with error: {}", e))??;

    let _ = tokio::fs::remove_file(&zip_tmp).await;

    // Collect extracted files for manifest
    let mut files = Vec::new();
    let mut dirs = vec![target_dir.clone()];
    while let Some(dir) = dirs.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    // ponytail: skip .vanilla and .manifest.json from manifest
                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                    if name != ".vanilla" && name != ".manifest.json" {
                        dirs.push(p);
                    }
                } else {
                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                    if name != ".manifest.json" {
                        if let Ok(rel) = p.strip_prefix(&target_dir) {
                            files.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
            }
        }
    }

    let manifest = ModpackManifest {
        modpack_title: modpack_title.clone(),
        files,
    };
    write_manifest(&game_folder, &game_name, &modpack_title, &manifest)?;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: total_bytes, total_bytes, speed_bytes_per_sec: 0.0,
        status: "completed".to_string(), error: None,
    });

    Ok(())
}

// ── Mock Downloads ─────────────────────────────────────────────────────

async fn perform_mock_download(
    app: AppHandle,
    game_id: String,
    game_name: String,
    version: String,
    zip_tmp_path: PathBuf,
    target_dir: PathBuf,
    size_bytes: u64,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    let mut downloaded = 0u64;
    if zip_tmp_path.exists() {
        if let Ok(metadata) = tokio::fs::metadata(&zip_tmp_path).await {
            downloaded = metadata.len();
        }
    }

    let mut file = OpenOptions::new().write(true).create(true).append(true).open(&zip_tmp_path).await
        .map_err(|e| format!("Failed to open mock temp file: {}", e))?;

    let mut last_emit = Instant::now();
    let mut last_downloaded = downloaded;
    let chunk_size = 1_500_000u64;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: None,
        downloaded_bytes: downloaded, total_bytes: size_bytes, speed_bytes_per_sec: 0.0,
        status: "downloading".to_string(), error: None,
    });

    while downloaded < size_bytes {
        tokio::select! {
            _ = &mut cancel_rx => {
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(), version: version.clone(), modpack_title: None,
                    downloaded_bytes: downloaded, total_bytes: size_bytes, speed_bytes_per_sec: 0.0,
                    status: "paused".to_string(), error: None,
                });
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                let bytes_to_write = std::cmp::min(chunk_size, size_bytes - downloaded);
                let buffer = vec![0u8; bytes_to_write as usize];
                file.write_all(&buffer).await.map_err(|e| format!("Failed to write mock data: {}", e))?;
                downloaded += bytes_to_write;

                let now = Instant::now();
                let elapsed = now.duration_since(last_emit);
                let speed = (downloaded - last_downloaded) as f64 / elapsed.as_secs_f64();
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(), version: version.clone(), modpack_title: None,
                    downloaded_bytes: downloaded, total_bytes: size_bytes, speed_bytes_per_sec: speed,
                    status: "downloading".to_string(), error: None,
                });
                last_emit = now;
                last_downloaded = downloaded;
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush mock file: {}", e))?;
    drop(file);

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: None,
        downloaded_bytes: downloaded, total_bytes: size_bytes, speed_bytes_per_sec: 0.0,
        status: "extracting".to_string(), error: None,
    });

    tokio::time::sleep(Duration::from_secs(1)).await;
    tokio::fs::create_dir_all(&target_dir).await
        .map_err(|e| format!("Failed to create mock game folder: {}", e))?;

    let bat = target_dir.join(format!("{}.bat", game_name.replace(" ", "")));
    let bat_content = format!(r#"@echo off
title {} - {}
color 0B
echo ===================================================
echo           WELCOME TO {} {}!
echo ===================================================
echo.
echo Launching simulated engines...
echo Files Verified: OK
echo Environment: DEV_MOCK
echo.
echo Press any key to exit {}...
pause > nul
"#, game_name, version, game_name, version, game_name);
    tokio::fs::write(&bat, bat_content).await
        .map_err(|e| format!("Failed to write mock launch bat: {}", e))?;

    let _ = tokio::fs::remove_file(&zip_tmp_path).await;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: None,
        downloaded_bytes: size_bytes, total_bytes: size_bytes, speed_bytes_per_sec: 0.0,
        status: "completed".to_string(), error: None,
    });

    Ok(())
}

async fn perform_mock_modpack_download(
    app: AppHandle,
    game_id: String,
    version: String,
    modpack_title: String,
    zip_tmp_path: PathBuf,
    target_dir: PathBuf,
    size_bytes: u64,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    let mut downloaded = 0u64;
    if zip_tmp_path.exists() {
        if let Ok(metadata) = tokio::fs::metadata(&zip_tmp_path).await {
            downloaded = metadata.len();
        }
    }

    let mut file = OpenOptions::new().write(true).create(true).append(true).open(&zip_tmp_path).await
        .map_err(|e| format!("Failed to open mock temp file: {}", e))?;

    let mut last_emit = Instant::now();
    let mut last_downloaded = downloaded;
    let chunk_size = 1_000_000u64;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: downloaded, total_bytes: size_bytes, speed_bytes_per_sec: 0.0,
        status: "downloading".to_string(), error: None,
    });

    while downloaded < size_bytes {
        tokio::select! {
            _ = &mut cancel_rx => {
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
                    downloaded_bytes: downloaded, total_bytes: size_bytes, speed_bytes_per_sec: 0.0,
                    status: "paused".to_string(), error: None,
                });
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                let bytes_to_write = std::cmp::min(chunk_size, size_bytes - downloaded);
                let buffer = vec![0u8; bytes_to_write as usize];
                file.write_all(&buffer).await.map_err(|e| format!("Failed to write mock data: {}", e))?;
                downloaded += bytes_to_write;

                let now = Instant::now();
                let elapsed = now.duration_since(last_emit);
                let speed = (downloaded - last_downloaded) as f64 / elapsed.as_secs_f64();
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
                    downloaded_bytes: downloaded, total_bytes: size_bytes, speed_bytes_per_sec: speed,
                    status: "downloading".to_string(), error: None,
                });
                last_emit = now;
                last_downloaded = downloaded;
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush mock file: {}", e))?;
    drop(file);

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: downloaded, total_bytes: size_bytes, speed_bytes_per_sec: 0.0,
        status: "extracting".to_string(), error: None,
    });

    tokio::time::sleep(Duration::from_secs(1)).await;
    tokio::fs::create_dir_all(&target_dir).await
        .map_err(|e| format!("Failed to create mock modpack folder: {}", e))?;

    // Create a mock readme in the modpack
    let readme = target_dir.join("readme.txt");
    tokio::fs::write(&readme, format!("Mock {} modpack files\n", modpack_title)).await
        .map_err(|e| format!("Failed to write mock modpack readme: {}", e))?;

    let _ = tokio::fs::remove_file(&zip_tmp_path).await;

    // Collect files for manifest
    let mut files = Vec::new();
    if readme.exists() {
        files.push("readme.txt".to_string());
    }
    let manifest = ModpackManifest {
        modpack_title: modpack_title.clone(),
        files,
    };
    // We need game_folder and game_name from the path context — use target_dir structure
    // ponytail: derive from target_dir path components
    let manifest_path = target_dir.join(".manifest.json");
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let json = serde_json::to_string(&manifest).unwrap_or_default();
    std::fs::write(&manifest_path, json).ok();

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(), version: version.clone(), modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: size_bytes, total_bytes: size_bytes, speed_bytes_per_sec: 0.0,
        status: "completed".to_string(), error: None,
    });

    Ok(())
}

// ── Zip extraction ─────────────────────────────────────────────────────

fn extract_zip(zip_path: &Path, target_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("Failed to open downloaded archive: {}", e))?;

    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Failed to parse zip archive: {}", e))?;

    std::fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create target directory: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| format!("Failed to read zip entry {}: {}", i, e))?;

        let outpath = match file.enclosed_name() {
            Some(path) => target_dir.join(path),
            None => continue,
        };

        if file.name().ends_with('/') {
            std::fs::create_dir_all(&outpath)
                .map_err(|e| format!("Failed to create folder {:?}: {}", outpath, e))?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)
                        .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                }
            }
            let mut outfile = std::fs::File::create(&outpath)
                .map_err(|e| format!("Failed to create file {:?}: {}", outpath, e))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract file contents: {}", e))?;
        }
    }

    Ok(())
}

// ── Apply / Swap ───────────────────────────────────────────────────────

#[tauri::command]
async fn apply_permutation(
    app: AppHandle,
    game_folder: String,
    game_id: String,
    game_name: String,
    version: String,
    modpack: Option<String>,
) -> Result<(), String> {
    let staging = staging_dir(&game_folder, &game_name);
    let current_state = read_state(&staging);

    // 1. Evict current modpack (if any)
    if let Some(ref cs) = current_state {
        if let Some(ref current_mp) = cs.modpack {
            let manifest = read_manifest(&game_folder, &game_name, current_mp);
            let vanilla_dir = vanilla_dir(&game_folder, &game_name, &cs.version);

            write_state(&staging, &StagedState {
                game_id: game_id.clone(),
                version: version.clone(),
                modpack: modpack.clone(),
                swap_phase: Some("evicting_modpack".to_string()),
            })?;

            if let Some(ref manifest) = manifest {
                for file_path in &manifest.files {
                    let staging_file = staging.join(file_path);
                    let vanilla_backup = vanilla_dir.join(file_path);
                    let mp_cache_file = modpack_cache_dir(&game_folder, &game_name, current_mp).join(file_path);

                    // Move modpack file from staging back to cache
                    if staging_file.exists() {
                        move_file(&staging_file, &mp_cache_file)?;
                    }

                    // Restore vanilla backup if it exists
                    if vanilla_backup.exists() {
                        move_file(&vanilla_backup, &staging_file)?;
                    }
                }
            }
        }
    }

    // 2. Swap version (if different)
    let need_version_swap = current_state.as_ref().map_or(true, |cs| cs.version != version);

    if need_version_swap {
        let swap_phase = if modpack.is_some() { "staging_version" } else { "staging_version" };
        write_state(&staging, &StagedState {
            game_id: game_id.clone(),
            version: version.clone(),
            modpack: modpack.clone(),
            swap_phase: Some(swap_phase.to_string()),
        })?;

        // Move current staging version back to cache
        if let Some(ref cs) = current_state {
            let old_version_dir = version_cache_dir(&game_folder, &game_name, &cs.version);
            move_dir(&staging, &old_version_dir)?;
        } else if staging.exists() {
            // ponytail: staging exists but no state — clear it
            std::fs::remove_dir_all(&staging).map_err(|e| format!("Failed to clear staging: {}", e))?;
        }

        // Move new version from cache to staging
        let new_version_dir = version_cache_dir(&game_folder, &game_name, &version);
        if !new_version_dir.exists() {
            return Err("Requested version is not cached. Download it first.".to_string());
        }
        move_dir(&new_version_dir, &staging)?;
        // Move the .vanilla folder too (it was inside the version dir, so it comes along)
    }

    // 3. Apply new modpack (if any)
    if let Some(ref mp) = modpack {
        write_state(&staging, &StagedState {
            game_id: game_id.clone(),
            version: version.clone(),
            modpack: Some(mp.clone()),
            swap_phase: Some("applying_modpack".to_string()),
        })?;

        let manifest = read_manifest(&game_folder, &game_name, mp);
        let vanilla = vanilla_dir(&game_folder, &game_name, &version);
        let mp_dir = modpack_cache_dir(&game_folder, &game_name, mp);

        if let Some(ref manifest) = manifest {
            for file_path in &manifest.files {
                let staging_file = staging.join(file_path);
                let mp_file = mp_dir.join(file_path);

                if !mp_file.exists() {
                    continue;
                }

                // Backup existing staging file to vanilla if it exists
                if staging_file.exists() {
                    let backup_path = vanilla.join(file_path);
                    move_file(&staging_file, &backup_path)?;
                }

                // Move modpack file to staging
                move_file(&mp_file, &staging_file)?;
            }
        }
    }

    // 4. Finalize
    write_state(&staging, &StagedState {
        game_id,
        version: version.clone(),
        modpack: modpack.clone(),
        swap_phase: None,
    })?;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_name.clone(),
        version: version.clone(),
        modpack_title: modpack.clone(),
        downloaded_bytes: 0,
        total_bytes: 0,
        speed_bytes_per_sec: 0.0,
        status: "applied".to_string(),
        error: None,
    });

    Ok(())
}

// ── Delete ─────────────────────────────────────────────────────────────

#[tauri::command]
async fn delete_version(
    game_folder: String,
    game_name: String,
    version: String,
) -> Result<(), String> {
    let staging = staging_dir(&game_folder, &game_name);
    let current = read_state(&staging);
    if let Some(ref cs) = current {
        if cs.version == version {
            return Err("Cannot delete the currently staged version. Apply a different version first.".to_string());
        }
    }

    let v_dir = version_cache_dir(&game_folder, &game_name, &version);
    if v_dir.exists() {
        std::fs::remove_dir_all(&v_dir)
            .map_err(|e| format!("Failed to delete version: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
async fn delete_modpack(
    game_folder: String,
    game_name: String,
    modpack_title: String,
) -> Result<(), String> {
    let staging = staging_dir(&game_folder, &game_name);
    let current = read_state(&staging);
    if let Some(ref cs) = current {
        if cs.modpack.as_deref() == Some(&modpack_title) {
            return Err("Cannot delete the currently applied modpack. Apply a different modpack or 'none' first.".to_string());
        }
    }

    let mp_dir = modpack_cache_dir(&game_folder, &game_name, &modpack_title);
    if mp_dir.exists() {
        std::fs::remove_dir_all(&mp_dir)
            .map_err(|e| format!("Failed to delete modpack: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
async fn delete_game(
    game_folder: String,
    game_name: String,
) -> Result<(), String> {
    let staging = staging_dir(&game_folder, &game_name);
    let cache = game_cache_dir(&game_folder, &game_name);

    // Delete staging (warns about all files)
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| format!("Failed to delete game staging folder: {}", e))?;
    }

    // Delete cache
    if cache.exists() {
        std::fs::remove_dir_all(&cache)
            .map_err(|e| format!("Failed to delete game cache: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
async fn move_game_folder(
    old_folder: String,
    new_folder: String,
) -> Result<(), String> {
    let old = PathBuf::from(&old_folder);
    let new = PathBuf::from(&new_folder);

    if !old.exists() {
        return Ok(());
    }

    tokio::fs::create_dir_all(&new).await
        .map_err(|e| format!("Failed to create new game folder: {}", e))?;

    // Move .nakama cache if it exists
    let old_cache = old.join(".nakama");
    let new_cache = new.join(".nakama");
    if old_cache.exists() {
        if new_cache.exists() {
            tokio::fs::remove_dir_all(&new_cache).await
                .map_err(|e| format!("Failed to remove existing cache at new location: {}", e))?;
        }
        tokio::fs::rename(&old_cache, &new_cache).await
            .map_err(|e| format!("Failed to move cache folder: {}", e))?;
    }

    // Move staging folders (game directories that have .nakama-state)
    if old.exists() {
        let mut dirs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&old) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let state_file = path.join(".nakama-state");
                    if state_file.exists() {
                        let dir_name = entry.file_name();
                        dirs.push((path, new.join(&dir_name)));
                    }
                }
            }
        }

        for (src, dst) in dirs {
            if dst.exists() {
                tokio::fs::remove_dir_all(&dst).await
                    .map_err(|e| format!("Failed to remove existing staging: {}", e))?;
            }
            tokio::fs::rename(&src, &dst).await
                .map_err(|e| format!("Failed to move staging folder: {}", e))?;
        }
    }

    Ok(())
}

// ── Launch ─────────────────────────────────────────────────────────────

#[tauri::command]
async fn launch_game(
    game_folder: String,
    game_name: String,
    _version: String,
    launch_path: String,
) -> Result<(), String> {
    let staging = staging_dir(&game_folder, &game_name);
    let exec_path = staging.join(&launch_path);

    if !exec_path.exists() {
        return Err(format!("Launch executable not found at: {:?}", exec_path));
    }

    #[cfg(target_os = "windows")]
    {
        let is_bat = exec_path.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("bat"));
        let mut cmd = if is_bat {
            let mut c = std::process::Command::new("cmd");
            c.args(&["/c", "start", "", exec_path.to_str().unwrap()]);
            c
        } else {
            std::process::Command::new(&exec_path)
        };

        cmd.current_dir(&staging);
        cmd.spawn()
            .map_err(|e| format!("Failed to launch game executable: {}", e))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new(&exec_path)
            .current_dir(&staging)
            .spawn()
            .map_err(|e| format!("Failed to launch game: {}", e))?;
    }

    Ok(())
}

// ── Folder picker ──────────────────────────────────────────────────────

#[tauri::command]
async fn select_directory() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("powershell")
            .args(&[
                "-NoProfile",
                "-Command",
                "Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.FolderBrowserDialog; $f.Description = 'Select Default Game Folder'; if($f.ShowDialog() -eq 'OK') { $f.SelectedPath }"
            ])
            .output()
            .map_err(|e| format!("Failed to open folder dialog: {}", e))?;

        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            Err("Cancelled".to_string())
        } else {
            Ok(path)
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Unsupported OS for folder dialog".to_string())
    }
}

// ── Entry ──────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(DownloadManager::default())
        .invoke_handler(tauri::generate_handler![
            get_games_list,
            get_download_status,
            get_modpack_status,
            get_staged_state,
            get_storage_sizes,
            start_download,
            start_download_modpack,
            pause_download,
            pause_download_modpack,
            cancel_download,
            cancel_download_modpack,
            apply_permutation,
            delete_version,
            delete_modpack,
            delete_game,
            move_game_folder,
            launch_game,
            select_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
