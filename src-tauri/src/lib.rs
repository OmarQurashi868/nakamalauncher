// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::path::{Path, PathBuf};
use std::time::{Instant, Duration};
use tokio::sync::oneshot;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};

// Types
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct GameVersion {
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
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ServerModpack {
    pub id: u32,
    pub game_title: String,
    pub modpack_title: String,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub uploaded_at: String,
    pub url: String,
}

#[derive(serde::Serialize, Clone)]
pub struct QueryResult {
    pub games: Vec<Game>,
    pub modpacks: Vec<ServerModpack>,
}

#[derive(serde::Deserialize)]
struct RawServerGame {
    id: u32,
    title: String,
    version: String,
    file_name: String,
    file_size_bytes: u64,
    launch_exe: String,
    uploaded_at: String,
}

#[derive(serde::Deserialize)]
struct RawServerModpack {
    id: u32,
    game_title: String,
    modpack_title: String,
    file_name: String,
    file_size_bytes: u64,
    uploaded_at: String,
}

#[derive(serde::Deserialize)]
struct RawServerResponse {
    games: Vec<RawServerGame>,
    modpacks: Vec<RawServerModpack>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct DownloadStatus {
    pub status: String, // "Downloaded", "Downloading", "Paused", "NotDownloaded"
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ModpackStatus {
    pub status: String, // "Downloaded", "Downloading", "Paused", "NotDownloaded"
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
    status: String, // "downloading", "paused", "extracting", "completed", "failed"
    error: Option<String>,
}

pub struct DownloadManager {
    // Key: "game_id:version" or "game_id:version:modpack_title" -> cancel sender
    jobs: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// Simple helper for URL percent-encoding without external crates
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

#[tauri::command]
async fn get_games_list(server_url: String, api_key: String) -> Result<QueryResult, String> {
    if server_url.trim().is_empty() || server_url == "mock" {
        // Return mock list
        let mock_json = include_str!("mock_games.json");
        let games: Vec<Game> = serde_json::from_str(mock_json)
            .map_err(|e| format!("Failed to parse mock games: {}", e))?;
        
        // Mock modpacks for mock mode
        let mock_modpacks = vec![
            ServerModpack {
                id: 1,
                game_title: "Cosmo Explorer".to_string(),
                modpack_title: "Cool Mod".to_string(),
                file_name: "Cosmo_Explorer_Cool_Mod.zip".to_string(),
                file_size_bytes: 15000000,
                uploaded_at: "2026-06-28T02:00:00Z".to_string(),
                url: "mock://cosmo-explorer/modpack/cool-mod".to_string(),
            },
            ServerModpack {
                id: 2,
                game_title: "Cosmo Explorer".to_string(),
                modpack_title: "HD Textures".to_string(),
                file_name: "Cosmo_Explorer_HD_Textures.zip".to_string(),
                file_size_bytes: 25000000,
                uploaded_at: "2026-06-28T03:00:00Z".to_string(),
                url: "mock://cosmo-explorer/modpack/hd-textures".to_string(),
            },
            ServerModpack {
                id: 3,
                game_title: "Cyber Sentinel".to_string(),
                modpack_title: "Redux Mod".to_string(),
                file_name: "Cyber_Sentinel_Redux_Mod.zip".to_string(),
                file_size_bytes: 35000000,
                uploaded_at: "2026-06-28T04:00:00Z".to_string(),
                url: "mock://cyber-sentinel/modpack/redux-mod".to_string(),
            },
        ];

        return Ok(QueryResult {
            games,
            modpacks: mock_modpacks,
        });
    }

    // Fetch from server_url
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

    // Sort raw games by uploaded_at descending so that grouping naturally retains latest version first
    let mut raw_games = raw_resp.games;
    raw_games.sort_by(|a, b| b.uploaded_at.cmp(&a.uploaded_at));

    let mut games_map: HashMap<String, Game> = HashMap::new();

    for raw_game in raw_games {
        let game_id = raw_game.title.to_lowercase().replace(" ", "-");
        
        let encoded_title = url_encode(&raw_game.title);
        let encoded_ver = url_encode(&raw_game.version);
        let download_url = format!("{}/download/game/{}/{}", base_url.trim_end_matches('/'), encoded_title, encoded_ver);

        let ver = GameVersion {
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
            };
            games_map.insert(raw_game.title.clone(), game);
        }
    }

    let games: Vec<Game> = games_map.into_values().collect();

    // Map raw modpacks to ServerModpack (with URL included)
    let modpacks = raw_resp.modpacks.into_iter().map(|raw_mp| {
        let encoded_game_title = url_encode(&raw_mp.game_title);
        let encoded_modpack_title = url_encode(&raw_mp.modpack_title);
        let download_url = format!("{}/download/modpack/{}/{}", base_url.trim_end_matches('/'), encoded_game_title, encoded_modpack_title);
        
        ServerModpack {
            id: raw_mp.id,
            game_title: raw_mp.game_title,
            modpack_title: raw_mp.modpack_title,
            file_name: raw_mp.file_name,
            file_size_bytes: raw_mp.file_size_bytes,
            uploaded_at: raw_mp.uploaded_at,
            url: download_url,
        }
    }).collect();

    Ok(QueryResult {
        games,
        modpacks,
    })
}

#[tauri::command]
async fn get_download_status(
    game_manager: tauri::State<'_, DownloadManager>,
    game_folder: String,
    game_name: String,
    version: String,
    game_id: String,
) -> Result<DownloadStatus, String> {
    let folder_name = format!("{} ({})", game_name, version);
    let target_dir = Path::new(&game_folder).join(&folder_name);
    let zip_tmp_path = Path::new(&game_folder).join(format!("{}.zip.tmp", folder_name));

    // Check if currently downloading or paused in our manager
    let job_key = format!("{}:{}", game_id, version);
    let is_active = {
        let jobs = game_manager.jobs.lock().unwrap();
        jobs.contains_key(&job_key)
    };

    if is_active {
        let downloaded = tokio::fs::metadata(&zip_tmp_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        return Ok(DownloadStatus {
            status: "Downloading".to_string(),
            downloaded_bytes: downloaded,
            total_bytes: 0,
        });
    }

    // Check if fully downloaded and extracted
    if target_dir.exists() && target_dir.is_dir() {
        return Ok(DownloadStatus {
            status: "Downloaded".to_string(),
            downloaded_bytes: 0,
            total_bytes: 0,
        });
    }

    // Check if partially downloaded (paused)
    if zip_tmp_path.exists() {
        let downloaded = tokio::fs::metadata(&zip_tmp_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
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
    let folder_name = format!("{} ({})", game_name, version);
    let target_dir = Path::new(&game_folder).join(&folder_name);
    let zip_tmp_path = Path::new(&game_folder).join(format!("{}_modpack_{}.zip.tmp", folder_name, modpack_title));

    // Check if currently downloading or paused
    let job_key = format!("{}:{}:{}", game_id, version, modpack_title);
    let is_active = {
        let jobs = game_manager.jobs.lock().unwrap();
        jobs.contains_key(&job_key)
    };

    if is_active {
        let downloaded = tokio::fs::metadata(&zip_tmp_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        return Ok(ModpackStatus {
            status: "Downloading".to_string(),
            downloaded_bytes: downloaded,
            total_bytes: 0,
            installed_uploaded_at: None,
        });
    }

    // Check if marker file exists
    let marker_path = target_dir.join(format!(".modpack_{}", modpack_title));
    if marker_path.exists() && marker_path.is_file() {
        let uploaded_at = tokio::fs::read_to_string(&marker_path)
            .await
            .ok()
            .map(|s| s.trim().to_string());
        return Ok(ModpackStatus {
            status: "Downloaded".to_string(),
            downloaded_bytes: 0,
            total_bytes: 0,
            installed_uploaded_at: uploaded_at,
        });
    }

    // Check if partially downloaded (paused)
    if zip_tmp_path.exists() {
        let downloaded = tokio::fs::metadata(&zip_tmp_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
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
) -> Result<(), String> {
    let job_key = format!("{}:{}", game_id, version);
    let mut jobs = game_manager.jobs.lock().unwrap();

    if jobs.contains_key(&job_key) {
        return Err("Download already in progress".to_string());
    }

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    jobs.insert(job_key.clone(), cancel_tx);

    let jobs_clone = Arc::clone(&game_manager.jobs);
    let job_key_clone = job_key.clone();

    // Spawn download in tokio task
    tokio::spawn(async move {
        let result = perform_download(
            app.clone(),
            game_id.clone(),
            game_name.clone(),
            version.clone(),
            url.clone(),
            game_folder.clone(),
            size_bytes,
            api_key,
            cancel_rx,
        ).await;

        // Remove from jobs map
        let mut jobs = jobs_clone.lock().unwrap();
        jobs.remove(&job_key_clone);

        if let Err(e) = result {
            let _ = app.emit("download-progress", DownloadProgressPayload {
                game_id: game_id.clone(),
                version: version.clone(),
                modpack_title: None,
                downloaded_bytes: 0,
                total_bytes: size_bytes,
                speed_bytes_per_sec: 0.0,
                status: "failed".to_string(),
                error: Some(e),
            });
        }
    });

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
    uploaded_at: String,
    url: String,
    game_folder: String,
    size_bytes: u64,
    api_key: String,
) -> Result<(), String> {
    let job_key = format!("{}:{}:{}", game_id, version, modpack_title);
    let mut jobs = game_manager.jobs.lock().unwrap();

    if jobs.contains_key(&job_key) {
        return Err("Modpack download already in progress".to_string());
    }

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    jobs.insert(job_key.clone(), cancel_tx);

    let jobs_clone = Arc::clone(&game_manager.jobs);
    let job_key_clone = job_key.clone();

    // Spawn download in tokio task
    tokio::spawn(async move {
        let result = perform_modpack_download(
            app.clone(),
            game_id.clone(),
            game_name.clone(),
            version.clone(),
            modpack_title.clone(),
            uploaded_at.clone(),
            url.clone(),
            game_folder.clone(),
            size_bytes,
            api_key,
            cancel_rx,
        ).await;

        // Remove from jobs map
        let mut jobs = jobs_clone.lock().unwrap();
        jobs.remove(&job_key_clone);

        if let Err(e) = result {
            let _ = app.emit("download-progress", DownloadProgressPayload {
                game_id: game_id.clone(),
                version: version.clone(),
                modpack_title: Some(modpack_title),
                downloaded_bytes: 0,
                total_bytes: size_bytes,
                speed_bytes_per_sec: 0.0,
                status: "failed".to_string(),
                error: Some(e),
            });
        }
    });

    Ok(())
}

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
    let folder_name = format!("{} ({})", game_name, version);
    let dest_dir = PathBuf::from(&game_folder);
    
    tokio::fs::create_dir_all(&dest_dir)
        .await
        .map_err(|e| format!("Failed to create game folder: {}", e))?;

    let zip_tmp_path = dest_dir.join(format!("{}.zip.tmp", folder_name));

    if url.starts_with("mock://") {
        perform_mock_download(
            app,
            game_id,
            game_name,
            version,
            zip_tmp_path,
            dest_dir.join(&folder_name),
            size_bytes,
            cancel_rx,
        ).await?;
        return Ok(());
    }

    let client = reqwest::Client::new();
    let mut downloaded = 0u64;
    if zip_tmp_path.exists() {
        if let Ok(metadata) = tokio::fs::metadata(&zip_tmp_path).await {
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

    let response = request.send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status();
    let mut file = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        OpenOptions::new()
            .write(true)
            .append(true)
            .open(&zip_tmp_path)
            .await
            .map_err(|e| format!("Failed to open temp file in append mode: {}", e))?
    } else if status.is_success() {
        downloaded = 0;
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&zip_tmp_path)
            .await
            .map_err(|e| format!("Failed to create temp file: {}", e))?
    } else {
        return Err(format!("Server returned HTTP status {}", status));
    };

    let total_bytes = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        if let Some(range_header) = response.headers().get(reqwest::header::CONTENT_RANGE) {
            if let Ok(range_str) = range_header.to_str() {
                if let Some(slash_idx) = range_str.rfind('/') {
                    if let Ok(total) = range_str[slash_idx + 1..].trim().parse::<u64>() {
                        total
                    } else {
                        size_bytes
                    }
                } else {
                    size_bytes
                }
            } else {
                size_bytes
            }
        } else {
            size_bytes
        }
    } else if let Some(content_length) = response.content_length() {
        content_length
    } else {
        size_bytes
    };

    let mut stream = response.bytes_stream();
    let mut last_emit = Instant::now();
    let mut last_downloaded = downloaded;
    let mut speed;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: None,
        downloaded_bytes: downloaded,
        total_bytes,
        speed_bytes_per_sec: 0.0,
        status: "downloading".to_string(),
        error: None,
    });

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(),
                    version: version.clone(),
                    modpack_title: None,
                    downloaded_bytes: downloaded,
                    total_bytes,
                    speed_bytes_per_sec: 0.0,
                    status: "paused".to_string(),
                    error: None,
                });
                return Ok(());
            }
            chunk_result = stream.next() => {
                match chunk_result {
                    Some(Ok(chunk)) => {
                        file.write_all(&chunk)
                            .await
                            .map_err(|e| format!("Failed to write chunk: {}", e))?;
                        
                        downloaded += chunk.len() as u64;

                        let now = Instant::now();
                        let elapsed = now.duration_since(last_emit);
                        if elapsed >= Duration::from_millis(300) {
                            let bytes_since_last = downloaded - last_downloaded;
                            speed = (bytes_since_last as f64) / elapsed.as_secs_f64();
                            
                            let _ = app.emit("download-progress", DownloadProgressPayload {
                                game_id: game_id.clone(),
                                version: version.clone(),
                                modpack_title: None,
                                downloaded_bytes: downloaded,
                                total_bytes,
                                speed_bytes_per_sec: speed,
                                status: "downloading".to_string(),
                                error: None,
                            });
                            
                            last_emit = now;
                            last_downloaded = downloaded;
                        }
                    }
                    Some(Err(e)) => {
                        return Err(format!("Stream error: {}", e));
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: None,
        downloaded_bytes: downloaded,
        total_bytes,
        speed_bytes_per_sec: 0.0,
        status: "extracting".to_string(),
        error: None,
    });

    let target_dir = dest_dir.join(&folder_name);
    
    let zip_tmp_path_clone = zip_tmp_path.clone();
    let target_dir_clone = target_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_zip(&zip_tmp_path_clone, &target_dir_clone)
    }).await
      .map_err(|e| format!("Extraction task joined with error: {}", e))??;

    let _ = tokio::fs::remove_file(&zip_tmp_path).await;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: None,
        downloaded_bytes: total_bytes,
        total_bytes,
        speed_bytes_per_sec: 0.0,
        status: "completed".to_string(),
        error: None,
    });

    Ok(())
}

async fn perform_modpack_download(
    app: AppHandle,
    game_id: String,
    game_name: String,
    version: String,
    modpack_title: String,
    uploaded_at: String,
    url: String,
    game_folder: String,
    size_bytes: u64,
    api_key: String,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    let folder_name = format!("{} ({})", game_name, version);
    let dest_dir = PathBuf::from(&game_folder);
    let target_dir = dest_dir.join(&folder_name);
    
    tokio::fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| format!("Failed to create game folder: {}", e))?;

    let zip_tmp_path = dest_dir.join(format!("{}_modpack_{}.zip.tmp", folder_name, modpack_title));

    if url.starts_with("mock://") {
        perform_mock_modpack_download(
            app,
            game_id,
            version,
            modpack_title,
            uploaded_at,
            zip_tmp_path,
            target_dir,
            size_bytes,
            cancel_rx,
        ).await?;
        return Ok(());
    }

    let client = reqwest::Client::new();
    let mut downloaded = 0u64;
    if zip_tmp_path.exists() {
        if let Ok(metadata) = tokio::fs::metadata(&zip_tmp_path).await {
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

    let response = request.send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status();
    let mut file = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        OpenOptions::new()
            .write(true)
            .append(true)
            .open(&zip_tmp_path)
            .await
            .map_err(|e| format!("Failed to open temp file in append mode: {}", e))?
    } else if status.is_success() {
        downloaded = 0;
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&zip_tmp_path)
            .await
            .map_err(|e| format!("Failed to create temp file: {}", e))?
    } else {
        return Err(format!("Server returned HTTP status {}", status));
    };

    let total_bytes = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        if let Some(range_header) = response.headers().get(reqwest::header::CONTENT_RANGE) {
            if let Ok(range_str) = range_header.to_str() {
                if let Some(slash_idx) = range_str.rfind('/') {
                    if let Ok(total) = range_str[slash_idx + 1..].trim().parse::<u64>() {
                        total
                    } else {
                        size_bytes
                    }
                } else {
                    size_bytes
                }
            } else {
                size_bytes
            }
        } else {
            size_bytes
        }
    } else if let Some(content_length) = response.content_length() {
        content_length
    } else {
        size_bytes
    };

    let mut stream = response.bytes_stream();
    let mut last_emit = Instant::now();
    let mut last_downloaded = downloaded;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: downloaded,
        total_bytes,
        speed_bytes_per_sec: 0.0,
        status: "downloading".to_string(),
        error: None,
    });

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(),
                    version: version.clone(),
                    modpack_title: Some(modpack_title.clone()),
                    downloaded_bytes: downloaded,
                    total_bytes,
                    speed_bytes_per_sec: 0.0,
                    status: "paused".to_string(),
                    error: None,
                });
                return Ok(());
            }
            chunk_result = stream.next() => {
                match chunk_result {
                    Some(Ok(chunk)) => {
                        file.write_all(&chunk)
                             .await
                             .map_err(|e| format!("Failed to write chunk: {}", e))?;
                        
                        downloaded += chunk.len() as u64;

                        let now = Instant::now();
                        let elapsed = now.duration_since(last_emit);
                        if elapsed >= Duration::from_millis(300) {
                            let bytes_since_last = downloaded - last_downloaded;
                            let speed = (bytes_since_last as f64) / elapsed.as_secs_f64();
                            
                            let _ = app.emit("download-progress", DownloadProgressPayload {
                                game_id: game_id.clone(),
                                version: version.clone(),
                                modpack_title: Some(modpack_title.clone()),
                                downloaded_bytes: downloaded,
                                total_bytes,
                                speed_bytes_per_sec: speed,
                                status: "downloading".to_string(),
                                error: None,
                            });
                            
                            last_emit = now;
                            last_downloaded = downloaded;
                        }
                    }
                    Some(Err(e)) => {
                        return Err(format!("Stream error: {}", e));
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: downloaded,
        total_bytes: size_bytes,
        speed_bytes_per_sec: 0.0,
        status: "extracting".to_string(),
        error: None,
    });

    let zip_tmp_path_clone = zip_tmp_path.clone();
    let target_dir_clone = target_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_zip(&zip_tmp_path_clone, &target_dir_clone)
    }).await
      .map_err(|e| format!("Extraction task joined with error: {}", e))??;

    let _ = tokio::fs::remove_file(&zip_tmp_path).await;

    // Create marker file
    let marker_path = target_dir.join(format!(".modpack_{}", modpack_title));
    if let Err(e) = tokio::fs::write(&marker_path, &uploaded_at).await {
        println!("Warning: Failed to write modpack marker file: {}", e);
    }

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: total_bytes,
        total_bytes,
        speed_bytes_per_sec: 0.0,
        status: "completed".to_string(),
        error: None,
    });

    Ok(())
}

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

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(true)
        .open(&zip_tmp_path)
        .await
        .map_err(|e| format!("Failed to open mock temp file: {}", e))?;

    let mut last_emit = Instant::now();
    let mut last_downloaded = downloaded;
    let chunk_size = 1_500_000u64;
    
    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: None,
        downloaded_bytes: downloaded,
        total_bytes: size_bytes,
        speed_bytes_per_sec: 0.0,
        status: "downloading".to_string(),
        error: None,
    });

    while downloaded < size_bytes {
        tokio::select! {
            _ = &mut cancel_rx => {
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(),
                    version: version.clone(),
                    modpack_title: None,
                    downloaded_bytes: downloaded,
                    total_bytes: size_bytes,
                    speed_bytes_per_sec: 0.0,
                    status: "paused".to_string(),
                    error: None,
                });
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                let bytes_to_write = std::cmp::min(chunk_size, size_bytes - downloaded);
                let buffer = vec![0u8; bytes_to_write as usize];
                file.write_all(&buffer)
                    .await
                    .map_err(|e| format!("Failed to write mock data: {}", e))?;
                
                downloaded += bytes_to_write;

                let now = Instant::now();
                let elapsed = now.duration_since(last_emit);
                let bytes_since_last = downloaded - last_downloaded;
                let speed = (bytes_since_last as f64) / elapsed.as_secs_f64();

                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(),
                    version: version.clone(),
                    modpack_title: None,
                    downloaded_bytes: downloaded,
                    total_bytes: size_bytes,
                    speed_bytes_per_sec: speed,
                    status: "downloading".to_string(),
                    error: None,
                });

                last_emit = now;
                last_downloaded = downloaded;
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush mock file: {}", e))?;
    drop(file);

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: None,
        downloaded_bytes: downloaded,
        total_bytes: size_bytes,
        speed_bytes_per_sec: 0.0,
        status: "extracting".to_string(),
        error: None,
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    tokio::fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| format!("Failed to create mock game folder: {}", e))?;

    let batch_path = target_dir.join(format!("{}.bat", game_name.replace(" ", "")));
    let bat_content = format!(
        r#"@echo off
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
echo [System]: Game is running successfully!
echo.
echo Press any key to exit {}...
pause > nul
"#,
        game_name, version, game_name, version, game_name
    );

    tokio::fs::write(&batch_path, bat_content)
        .await
        .map_err(|e| format!("Failed to write mock launch bat: {}", e))?;

    let _ = tokio::fs::remove_file(&zip_tmp_path).await;

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: None,
        downloaded_bytes: size_bytes,
        total_bytes: size_bytes,
        speed_bytes_per_sec: 0.0,
        status: "completed".to_string(),
        error: None,
    });

    Ok(())
}

async fn perform_mock_modpack_download(
    app: AppHandle,
    game_id: String,
    version: String,
    modpack_title: String,
    uploaded_at: String,
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

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(true)
        .open(&zip_tmp_path)
        .await
        .map_err(|e| format!("Failed to open mock temp file: {}", e))?;

    let mut last_emit = Instant::now();
    let mut last_downloaded = downloaded;
    let chunk_size = 1_000_000u64; // 1 MB per tick for modpack
    
    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: downloaded,
        total_bytes: size_bytes,
        speed_bytes_per_sec: 0.0,
        status: "downloading".to_string(),
        error: None,
    });

    while downloaded < size_bytes {
        tokio::select! {
            _ = &mut cancel_rx => {
                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(),
                    version: version.clone(),
                    modpack_title: Some(modpack_title.clone()),
                    downloaded_bytes: downloaded,
                    total_bytes: size_bytes,
                    speed_bytes_per_sec: 0.0,
                    status: "paused".to_string(),
                    error: None,
                });
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                let bytes_to_write = std::cmp::min(chunk_size, size_bytes - downloaded);
                let buffer = vec![0u8; bytes_to_write as usize];
                file.write_all(&buffer)
                    .await
                    .map_err(|e| format!("Failed to write mock data: {}", e))?;
                
                downloaded += bytes_to_write;

                let now = Instant::now();
                let elapsed = now.duration_since(last_emit);
                let bytes_since_last = downloaded - last_downloaded;
                let speed = (bytes_since_last as f64) / elapsed.as_secs_f64();

                let _ = app.emit("download-progress", DownloadProgressPayload {
                    game_id: game_id.clone(),
                    version: version.clone(),
                    modpack_title: Some(modpack_title.clone()),
                    downloaded_bytes: downloaded,
                    total_bytes: size_bytes,
                    speed_bytes_per_sec: speed,
                    status: "downloading".to_string(),
                    error: None,
                });

                last_emit = now;
                last_downloaded = downloaded;
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush mock file: {}", e))?;
    drop(file);

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: downloaded,
        total_bytes: size_bytes,
        speed_bytes_per_sec: 0.0,
        status: "extracting".to_string(),
        error: None,
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    let _ = tokio::fs::remove_file(&zip_tmp_path).await;

    // Create marker file
    let marker_path = target_dir.join(format!(".modpack_{}", modpack_title));
    if let Err(e) = tokio::fs::write(&marker_path, &uploaded_at).await {
        println!("Warning: Failed to write mock modpack marker file: {}", e);
    }

    let _ = app.emit("download-progress", DownloadProgressPayload {
        game_id: game_id.clone(),
        version: version.clone(),
        modpack_title: Some(modpack_title.clone()),
        downloaded_bytes: size_bytes,
        total_bytes: size_bytes,
        speed_bytes_per_sec: 0.0,
        status: "completed".to_string(),
        error: None,
    });

    Ok(())
}

fn extract_zip(zip_path: &Path, target_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("Failed to open downloaded archive: {}", e))?;
    
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Failed to parse zip archive: {}", e))?;

    std::fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create game directory: {}", e))?;

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

#[tauri::command]
async fn launch_game(
    game_folder: String,
    game_name: String,
    version: String,
    launch_path: String,
) -> Result<(), String> {
    let folder_name = format!("{} ({})", game_name, version);
    let target_dir = Path::new(&game_folder).join(&folder_name);
    let exec_path = target_dir.join(&launch_path);

    if !exec_path.exists() {
        return Err(format!(
            "Launch executable not found at: {:?}",
            exec_path
        ));
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
        
        cmd.current_dir(&target_dir);
        cmd.spawn()
            .map_err(|e| format!("Failed to launch game executable: {}", e))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new(&exec_path)
            .current_dir(&target_dir)
            .spawn()
            .map_err(|e| format!("Failed to launch game: {}", e))?;
    }

    Ok(())
}

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(DownloadManager::default())
        .invoke_handler(tauri::generate_handler![
            get_games_list,
            get_download_status,
            get_modpack_status,
            start_download,
            start_download_modpack,
            pause_download,
            pause_download_modpack,
            launch_game,
            select_directory
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
