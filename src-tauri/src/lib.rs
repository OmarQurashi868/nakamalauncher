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
pub struct DownloadStatus {
    pub status: String, // "Downloaded", "Downloading", "Paused", "NotDownloaded"
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

#[derive(serde::Serialize, Clone)]
struct DownloadProgressPayload {
    game_id: String,
    version: String,
    downloaded_bytes: u64,
    total_bytes: u64,
    speed_bytes_per_sec: f64,
    status: String, // "downloading", "paused", "extracting", "completed", "failed"
    error: Option<String>,
}

pub struct DownloadManager {
    // Key: "game_id:version" -> cancel sender
    jobs: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[tauri::command]
async fn get_games_list(server_url: String) -> Result<Vec<Game>, String> {
    if server_url.trim().is_empty() || server_url == "mock" {
        // Return mock list
        let mock_json = include_str!("mock_games.json");
        let games: Vec<Game> = serde_json::from_str(mock_json)
            .map_err(|e| format!("Failed to parse mock games: {}", e))?;
        return Ok(games);
    }

    // Fetch from server_url
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client.get(&server_url)
        .send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    let games = response.json::<Vec<Game>>()
        .await
        .map_err(|e| format!("Failed to parse server response: {}", e))?;

    Ok(games)
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
async fn start_download(
    app: AppHandle,
    game_manager: tauri::State<'_, DownloadManager>,
    game_id: String,
    game_name: String,
    version: String,
    url: String,
    game_folder: String,
    size_bytes: u64,
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
            cancel_rx,
        ).await;

        // Remove from jobs map
        let mut jobs = jobs_clone.lock().unwrap();
        jobs.remove(&job_key_clone);

        if let Err(e) = result {
            let _ = app.emit("download-progress", DownloadProgressPayload {
                game_id: game_id.clone(),
                version: version.clone(),
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
        downloaded_bytes: total_bytes,
        total_bytes,
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
        downloaded_bytes: size_bytes,
        total_bytes: size_bytes,
        speed_bytes_per_sec: 0.0,
        status: "completed".to_string(),
        error: None,
    });

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
            start_download,
            pause_download,
            launch_game,
            select_directory
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
