import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  FolderOpen,
  Settings,
  Search,
  Gamepad2,
  Download,
  Pause,
  Play,
  CheckCircle2,
  AlertTriangle,
  Loader2,
  Folder
} from "lucide-react";
import "./App.css";

// Interface definitions matches Rust structs
interface GameVersion {
  version: string;
  url: string;
  launch_path: string;
  size_bytes: number;
}

interface Game {
  id: string;
  name: string;
  icon_url: string | null;
  description: string;
  versions: GameVersion[];
}

interface ProgressPayload {
  game_id: string;
  version: string;
  downloaded_bytes: number;
  total_bytes: number;
  speed_bytes_per_sec: number;
  status: string; // "downloading", "paused", "extracting", "completed", "failed"
  error: string | null;
}

interface VersionStatus {
  status: string; // "Downloaded", "Downloading", "Paused", "NotDownloaded", "Extracting", "Failed"
  downloadedBytes: number;
  totalBytes: number;
  speedBytesPerSec?: number;
  error?: string | null;
}

function App() {
  // Persistence states
  const [defaultGameFolder, setDefaultGameFolder] = useState<string>(() => {
    return localStorage.getItem("defaultGameFolder") || "C:\\Games";
  });
  const [serverUrl, setServerUrl] = useState<string>(() => {
    return localStorage.getItem("serverUrl") || "mock";
  });

  // UI / Logic states
  const [games, setGames] = useState<Game[]>([]);
  const [selectedGame, setSelectedGame] = useState<Game | null>(null);
  const [selectedVersion, setSelectedVersion] = useState<GameVersion | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [loadingGames, setLoadingGames] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [settingsModalOpen, setSettingsModalOpen] = useState(false);

  // Settings form states
  const [tempFolder, setTempFolder] = useState(defaultGameFolder);
  const [tempServer, setTempServer] = useState(serverUrl);

  // Track statuses of game versions: Key is "gameId:version"
  const [gameStatuses, setGameStatuses] = useState<Record<string, VersionStatus>>({});

  // 1. Fetch games list
  const fetchGames = async (url: string) => {
    setLoadingGames(true);
    setErrorMsg(null);
    try {
      const list = await invoke<Game[]>("get_games_list", { serverUrl: url });
      setGames(list);
      
      // Auto select first game if none is selected
      if (list.length > 0) {
        setSelectedGame((prev) => {
          const stillExists = list.find((g) => g.id === prev?.id);
          return stillExists || list[0];
        });
      } else {
        setSelectedGame(null);
      }
    } catch (err: any) {
      console.error(err);
      setErrorMsg(typeof err === "string" ? err : "Failed to fetch games list.");
    } finally {
      setLoadingGames(false);
    }
  };

  // Run on mount or when serverUrl changes
  useEffect(() => {
    fetchGames(serverUrl);
  }, [serverUrl]);

  // 2. Scan download status for all game versions
  const scanAllStatuses = async () => {
    if (games.length === 0) return;
    const newStatuses: Record<string, VersionStatus> = {};
    
    for (const game of games) {
      for (const ver of game.versions) {
        try {
          const res = await invoke<{ status: string; downloaded_bytes: number; total_bytes: number }>(
            "get_download_status",
            {
              gameFolder: defaultGameFolder,
              gameName: game.name,
              version: ver.version,
              gameId: game.id,
            }
          );
          
          const key = `${game.id}:${ver.version}`;
          newStatuses[key] = {
            status: res.status, // "Downloaded", "Downloading", "Paused", "NotDownloaded"
            downloadedBytes: res.downloaded_bytes || 0,
            totalBytes: ver.size_bytes,
          };
        } catch (err) {
          console.error("Error scanning status: ", err);
        }
      }
    }
    setGameStatuses((prev) => ({ ...prev, ...newStatuses }));
  };

  // Re-scan when games or defaultGameFolder changes
  useEffect(() => {
    scanAllStatuses();
  }, [games, defaultGameFolder]);

  // Update selectedVersion when selectedGame changes
  useEffect(() => {
    if (selectedGame && selectedGame.versions.length > 0) {
      // Default to latest version (first in list)
      setSelectedVersion(selectedGame.versions[0]);
    } else {
      setSelectedVersion(null);
    }
  }, [selectedGame]);

  // 3. Listen to Rust progress events
  useEffect(() => {
    const setupListener = async () => {
      const unlisten = await listen<ProgressPayload>("download-progress", (event) => {
        const payload = event.payload;
        const key = `${payload.game_id}:${payload.version}`;
        
        let displayStatus = "Downloading";
        if (payload.status === "paused") displayStatus = "Paused";
        if (payload.status === "extracting") displayStatus = "Extracting";
        if (payload.status === "completed") displayStatus = "Downloaded";
        if (payload.status === "failed") displayStatus = "Failed";

        setGameStatuses((prev) => ({
          ...prev,
          [key]: {
            status: displayStatus,
            downloadedBytes: payload.downloaded_bytes,
            totalBytes: payload.total_bytes,
            speedBytesPerSec: payload.speed_bytes_per_sec,
            error: payload.error,
          },
        }));
      });
      return unlisten;
    };

    let unlistenFn: (() => void) | undefined;
    setupListener().then((fn) => {
      unlistenFn = fn;
    });

    return () => {
      if (unlistenFn) unlistenFn();
    };
  }, []);

  // Action: Select folder
  const browseFolder = async () => {
    try {
      const selected = await invoke<string>("select_directory");
      setTempFolder(selected);
    } catch (err: any) {
      if (err !== "Cancelled") {
        alert("Failed to pick folder: " + err);
      }
    }
  };

  // Action: Save Settings
  const saveSettings = () => {
    localStorage.setItem("defaultGameFolder", tempFolder);
    localStorage.setItem("serverUrl", tempServer);
    setDefaultGameFolder(tempFolder);
    setServerUrl(tempServer);
    setSettingsModalOpen(false);
  };

  // Action: Start Download
  const startDownload = async (game: Game, ver: GameVersion) => {
    const key = `${game.id}:${ver.version}`;
    setGameStatuses((prev) => ({
      ...prev,
      [key]: {
        status: "Downloading",
        downloadedBytes: prev[key]?.downloadedBytes || 0,
        totalBytes: ver.size_bytes,
        speedBytesPerSec: 0,
      },
    }));

    try {
      await invoke("start_download", {
        app: null,
        gameId: game.id,
        gameName: game.name,
        version: ver.version,
        url: ver.url,
        gameFolder: defaultGameFolder,
        sizeBytes: ver.size_bytes,
      });
    } catch (err: any) {
      alert("Failed to start download: " + err);
      setGameStatuses((prev) => ({
        ...prev,
        [key]: {
          status: "NotDownloaded",
          downloadedBytes: 0,
          totalBytes: ver.size_bytes,
        },
      }));
    }
  };

  // Action: Pause Download
  const pauseDownload = async (game: Game, ver: GameVersion) => {
    try {
      await invoke("pause_download", {
        gameId: game.id,
        version: ver.version,
      });
    } catch (err: any) {
      alert("Failed to pause: " + err);
    }
  };

  // Action: Play Game
  const playGame = async (game: Game, ver: GameVersion) => {
    try {
      await invoke("launch_game", {
        gameFolder: defaultGameFolder,
        gameName: game.name,
        version: ver.version,
        launchPath: ver.launch_path,
      });
    } catch (err: any) {
      alert("Failed to launch game: " + err);
    }
  };

  // Helper: Format bytes
  const formatBytes = (bytes: number) => {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + " " + sizes[i];
  };

  // Helper: Format Speed
  const formatSpeed = (bytesPerSec?: number) => {
    if (!bytesPerSec || bytesPerSec === 0) return "0 B/s";
    return `${formatBytes(bytesPerSec)}/s`;
  };

  // Helper: Get Game Update status
  const getGameVersionsStatus = (game: Game) => {
    const downloadedVersions = game.versions.filter(
      (v) => gameStatuses[`${game.id}:${v.version}`]?.status === "Downloaded"
    );
    const hasSomeVersionDownloaded = downloadedVersions.length > 0;
    
    const latestVersion = game.versions[0];
    const isLatestDownloaded = gameStatuses[`${game.id}:${latestVersion.version}`]?.status === "Downloaded";
    const updateAvailable = hasSomeVersionDownloaded && !isLatestDownloaded;
    
    return { hasSomeVersionDownloaded, isLatestDownloaded, updateAvailable, latestVersion };
  };

  // Filtered games based on search query
  const filteredGames = games.filter((g) =>
    g.name.toLowerCase().includes(searchQuery.toLowerCase())
  );

  return (
    <div className="app-container">
      {/* 1. App Header */}
      <header className="app-header">
        <div className="logo-section">
          <Gamepad2 className="logo-icon" size={22} />
          <h1>Nakama Launcher</h1>
        </div>
        
        <div className="header-actions">
          <div className="folder-info">
            <Folder size={12} />
            <span>{defaultGameFolder}</span>
          </div>
          <button
            className="settings-trigger"
            onClick={() => {
              setTempFolder(defaultGameFolder);
              setTempServer(serverUrl);
              setSettingsModalOpen(true);
            }}
            title="Settings"
          >
            <Settings size={16} />
          </button>
        </div>
      </header>

      {/* 2. App Main Body */}
      <div className="app-body">
        {/* Left Side: Game List Sidebar */}
        <aside className="sidebar">
          <div className="search-section">
            <div className="search-input-wrapper">
              <Search className="search-icon" />
              <input
                type="text"
                placeholder="Search games..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
            </div>
          </div>
          
          <div className="game-list">
            {loadingGames ? (
              <div className="list-state">
                <Loader2 className="animate-spin" size={20} />
                <span>Loading catalog...</span>
              </div>
            ) : filteredGames.length === 0 ? (
              <div className="list-state">
                No games found. Check server settings.
              </div>
            ) : (
              filteredGames.map((game) => {
                const { updateAvailable } = getGameVersionsStatus(game);
                const isActive = selectedGame?.id === game.id;
                const latestVer = game.versions[0];
                const latestStatus = gameStatuses[`${game.id}:${latestVer.version}`]?.status;

                return (
                  <div
                    key={game.id}
                    className={`game-card ${isActive ? "active" : ""}`}
                    onClick={() => setSelectedGame(game)}
                  >
                    {game.icon_url ? (
                      <img
                        src={game.icon_url}
                        alt={game.name}
                        className="game-card-icon"
                        onError={(e) => {
                          (e.currentTarget as any).style.display = "none";
                        }}
                      />
                    ) : (
                      <div className="game-card-icon">
                        <Gamepad2 size={18} />
                      </div>
                    )}
                    
                    <div className="game-card-info">
                      <div className="game-card-name">{game.name}</div>
                      <div className="game-card-status-row">
                        {updateAvailable && (
                          <span className="badge badge-update">Update</span>
                        )}
                        {latestStatus === "Downloaded" && (
                          <span className="badge badge-installed">Installed</span>
                        )}
                        {latestStatus === "Downloading" && (
                          <span className="badge badge-downloading">Downloading</span>
                        )}
                        {latestStatus === "Paused" && (
                          <span className="badge badge-paused">Paused</span>
                        )}
                      </div>
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </aside>

        {/* Right Side: Game Details Panel */}
        <main className="main-content">
          {errorMsg && (
            <div className="update-banner error-banner">
              <div className="update-banner-info">
                <AlertTriangle size={14} />
                <span>Error: {errorMsg}</span>
              </div>
              <button className="update-btn danger" onClick={() => fetchGames(serverUrl)}>
                Retry
              </button>
            </div>
          )}

          {selectedGame ? (
            <div className="game-details">
              {/* Game Header */}
              <div className="details-header">
                {selectedGame.icon_url ? (
                  <img
                    src={selectedGame.icon_url}
                    alt={selectedGame.name}
                    className="details-icon"
                    onError={(e) => {
                      (e.currentTarget as any).style.display = "none";
                    }}
                  />
                ) : (
                  <div className="details-icon">
                    <Gamepad2 size={36} />
                  </div>
                )}
                
                <div className="details-title-desc">
                  <h2>{selectedGame.name}</h2>
                  <p className="details-desc">{selectedGame.description}</p>
                </div>
              </div>

              {/* Version & Action Panel */}
              <div className="version-card">
                {/* Selector Row */}
                <div className="version-selector-row">
                  <div className="selector-field">
                    <label>Version</label>
                    <select
                      className="custom-select"
                      value={selectedVersion?.version || ""}
                      onChange={(e) => {
                        const ver = selectedGame.versions.find((v) => v.version === e.target.value);
                        if (ver) setSelectedVersion(ver);
                      }}
                    >
                      {selectedGame.versions.map((v) => (
                        <option key={v.version} value={v.version}>
                          {v.version}
                        </option>
                      ))}
                    </select>
                  </div>
                  
                  <div className="selector-field">
                    <label>Size</label>
                    <span>
                      {selectedVersion ? formatBytes(selectedVersion.size_bytes) : "—"}
                    </span>
                  </div>
                </div>

                {/* Update Banner */}
                {(() => {
                  const { updateAvailable, latestVersion } = getGameVersionsStatus(selectedGame);
                  if (updateAvailable && selectedVersion?.version !== latestVersion.version) {
                    return (
                      <div className="update-banner">
                        <div className="update-banner-info">
                          <AlertTriangle size={14} />
                          <span>
                            Version <strong>{latestVersion.version}</strong> is available
                          </span>
                        </div>
                        <button
                          className="update-btn"
                          onClick={() => setSelectedVersion(latestVersion)}
                        >
                          Switch
                        </button>
                      </div>
                    );
                  }
                  return null;
                })()}

                {/* Action Area */}
                {selectedVersion && (() => {
                  const key = `${selectedGame.id}:${selectedVersion.version}`;
                  const statusInfo = gameStatuses[key] || {
                    status: "NotDownloaded",
                    downloadedBytes: 0,
                    totalBytes: selectedVersion.size_bytes,
                  };

                  const percent = statusInfo.totalBytes > 0
                    ? Math.round((statusInfo.downloadedBytes / statusInfo.totalBytes) * 100)
                    : 0;

                  switch (statusInfo.status) {
                    case "Downloading":
                      return (
                        <div className="action-area">
                          <div className="download-progress-container">
                            <div className="progress-info">
                              <span className="progress-status">Downloading</span>
                              <span>{percent}%</span>
                            </div>
                            <div className="progress-bar-wrapper">
                              <div className="progress-bar" style={{ width: `${percent}%` }} />
                            </div>
                            <div className="progress-stats">
                              <span>{formatBytes(statusInfo.downloadedBytes)} / {formatBytes(statusInfo.totalBytes)}</span>
                              <span className="progress-speed">{formatSpeed(statusInfo.speedBytesPerSec)}</span>
                            </div>
                          </div>
                          <button
                            className="btn btn-secondary"
                            onClick={() => pauseDownload(selectedGame, selectedVersion)}
                          >
                            <Pause size={15} /> Pause
                          </button>
                        </div>
                      );

                    case "Paused":
                      return (
                        <div className="action-area">
                          <div className="download-progress-container">
                            <div className="progress-info">
                              <span className="progress-status">Paused</span>
                              <span>{percent}%</span>
                            </div>
                            <div className="progress-bar-wrapper">
                              <div className="progress-bar" style={{ width: `${percent}%`, opacity: 0.5 }} />
                            </div>
                            <div className="progress-stats">
                              <span>{formatBytes(statusInfo.downloadedBytes)} / {formatBytes(statusInfo.totalBytes)}</span>
                              <span>Paused</span>
                            </div>
                          </div>
                          <button
                            className="btn btn-primary"
                            onClick={() => startDownload(selectedGame, selectedVersion)}
                          >
                            <Download size={15} /> Resume
                          </button>
                        </div>
                      );

                    case "Extracting":
                      return (
                        <div className="action-area">
                          <div className="download-progress-container">
                            <div className="progress-info">
                              <span className="progress-status">Extracting</span>
                              <span>99%</span>
                            </div>
                            <div className="progress-bar-wrapper">
                              <div className="progress-bar extracting" />
                            </div>
                            <div className="progress-stats">
                              <span>Unpacking files...</span>
                              <span>Please wait</span>
                            </div>
                          </div>
                          <button className="btn btn-primary" disabled>
                            <Loader2 className="animate-spin" size={15} /> Extracting...
                          </button>
                        </div>
                      );

                    case "Downloaded":
                      return (
                        <div className="action-area">
                          <div className="status-pill success">
                            <CheckCircle2 size={14} />
                            Installed and ready to play
                          </div>
                          <button
                            className="btn btn-success"
                            onClick={() => playGame(selectedGame, selectedVersion)}
                          >
                            <Play size={15} /> Play
                          </button>
                        </div>
                      );

                    case "Failed":
                      return (
                        <div className="action-area">
                          <div className="status-pill danger">
                            <span>Download failed</span>
                            <span className="error-detail">
                              {statusInfo.error || "Unknown error"}
                            </span>
                          </div>
                          <button
                            className="btn btn-primary"
                            onClick={() => startDownload(selectedGame, selectedVersion)}
                          >
                            <Download size={15} /> Retry
                          </button>
                        </div>
                      );

                    default:
                      return (
                        <div className="action-area">
                          <button
                            className="btn btn-primary"
                            onClick={() => startDownload(selectedGame, selectedVersion)}
                          >
                            <Download size={15} /> Download
                          </button>
                        </div>
                      );
                  }
                })()}
              </div>
            </div>
          ) : (
            <div className="welcome-screen">
              <Gamepad2 className="welcome-icon" size={64} />
              <h2>Nakama Launcher</h2>
              <p>Select a game from the list to get started.</p>
            </div>
          )}
        </main>
      </div>

      {/* 3. Settings Modal */}
      {settingsModalOpen && (
        <div className="modal-overlay">
          <div className="modal-content">
            <header className="modal-header">
              <h3>
                <Settings size={15} /> Settings
              </h3>
              <button className="modal-close" onClick={() => setSettingsModalOpen(false)}>
                ✕
              </button>
            </header>
            
            <div className="settings-content">
              <div className="setting-group">
                <label>Game Folder</label>
                <div className="input-browse-wrapper">
                  <input
                    type="text"
                    value={tempFolder}
                    onChange={(e) => setTempFolder(e.target.value)}
                    placeholder="e.g. C:\Games"
                  />
                  <button className="browse-btn" onClick={browseFolder}>
                    <FolderOpen size={14} /> Browse
                  </button>
                </div>
                <span className="hint">
                  Stored as: [Folder] \ [Game Name] ([Version])
                </span>
              </div>
              
              <div className="setting-group">
                <label>Metadata Server URL</label>
                <input
                  type="text"
                  value={tempServer}
                  onChange={(e) => setTempServer(e.target.value)}
                  placeholder="e.g. http://my-server.com/games.json"
                />
                <span className="hint">
                  Use "mock" for the built-in offline demo library.
                </span>
              </div>
            </div>
            
            <footer className="settings-actions">
              <button
                className="modal-btn modal-btn-cancel"
                onClick={() => setSettingsModalOpen(false)}
              >
                Cancel
              </button>
              <button className="modal-btn modal-btn-save" onClick={saveSettings}>
                Save
              </button>
            </footer>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
