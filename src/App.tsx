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

interface ServerModpack {
  id: number;
  game_title: string;
  modpack_title: string;
  file_name: string;
  file_size_bytes: number;
  uploaded_at: string;
  url: string;
}

interface ProgressPayload {
  game_id: string;
  version: string;
  modpack_title: string | null;
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
  installed_uploaded_at?: string;
}

function App() {
  // Persistence states
  const [defaultGameFolder, setDefaultGameFolder] = useState<string>(() => {
    return localStorage.getItem("defaultGameFolder") || "C:\\Games";
  });
  const [serverUrl, setServerUrl] = useState<string>(() => {
    return localStorage.getItem("serverUrl") || "mock";
  });
  const [apiKey, setApiKey] = useState<string>(() => {
    return localStorage.getItem("apiKey") || "";
  });

  // UI / Logic states
  const [games, setGames] = useState<Game[]>([]);
  const [modpacks, setModpacks] = useState<ServerModpack[]>([]);
  const [selectedGame, setSelectedGame] = useState<Game | null>(null);
  const [selectedVersion, setSelectedVersion] = useState<GameVersion | null>(null);
  const [selectedModpack, setSelectedModpack] = useState<ServerModpack | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [loadingGames, setLoadingGames] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [settingsModalOpen, setSettingsModalOpen] = useState(false);

  // Settings form states
  const [tempFolder, setTempFolder] = useState(defaultGameFolder);
  const [tempServer, setTempServer] = useState(serverUrl);
  const [tempApiKey, setTempApiKey] = useState(apiKey);

  // Track statuses of game versions: Key is "gameId:version"
  const [gameStatuses, setGameStatuses] = useState<Record<string, VersionStatus>>({});
  // Track statuses of modpacks: Key is "gameId:version:modpackTitle"
  const [modpackStatuses, setModpackStatuses] = useState<Record<string, VersionStatus>>({});

  // 1. Fetch games list
  const fetchGames = async (url: string, key: string) => {
    setLoadingGames(true);
    setErrorMsg(null);
    try {
      const res = await invoke<{ games: Game[]; modpacks: ServerModpack[] }>("get_games_list", {
        serverUrl: url,
        apiKey: key,
      });
      setGames(res.games);
      setModpacks(res.modpacks);
      
      // Auto select first game if none is selected
      if (res.games.length > 0) {
        setSelectedGame((prev) => {
          const stillExists = res.games.find((g) => g.id === prev?.id);
          return stillExists || res.games[0];
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

  // Run on mount or when serverUrl or apiKey changes
  useEffect(() => {
    fetchGames(serverUrl, apiKey);
  }, [serverUrl, apiKey]);

  // 2. Scan download status for all game versions and modpacks
  const scanAllStatuses = async () => {
    if (games.length === 0) return;
    const newStatuses: Record<string, VersionStatus> = {};
    const newModpackStatuses: Record<string, VersionStatus> = {};
    
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

        // Modpack status
        const gameModpacks = modpacks.filter(
          (m) => m.game_title.toLowerCase() === game.name.toLowerCase()
        );
        for (const mp of gameModpacks) {
          try {
            const res = await invoke<{ status: string; downloaded_bytes: number; total_bytes: number; installed_uploaded_at: string | null }>(
              "get_modpack_status",
              {
                gameFolder: defaultGameFolder,
                gameName: game.name,
                version: ver.version,
                gameId: game.id,
                modpackTitle: mp.modpack_title,
              }
            );

            const key = `${game.id}:${ver.version}:${mp.modpack_title}`;
            newModpackStatuses[key] = {
              status: res.status,
              downloadedBytes: res.downloaded_bytes || 0,
              totalBytes: mp.file_size_bytes,
              installed_uploaded_at: res.installed_uploaded_at || undefined,
            };
          } catch (err) {
            console.error("Error scanning modpack status: ", err);
          }
        }
      }
    }
    setGameStatuses((prev) => ({ ...prev, ...newStatuses }));
    setModpackStatuses((prev) => ({ ...prev, ...newModpackStatuses }));
  };

  // Re-scan when games, modpacks, or defaultGameFolder changes
  useEffect(() => {
    scanAllStatuses();
  }, [games, modpacks, defaultGameFolder]);

  // Update selectedVersion and reset modpack when selectedGame changes
  useEffect(() => {
    if (selectedGame && selectedGame.versions.length > 0) {
      // Default to latest version (first in list)
      setSelectedVersion(selectedGame.versions[0]);
    } else {
      setSelectedVersion(null);
    }
    setSelectedModpack(null);
  }, [selectedGame]);

  // Reset modpack when version changes
  useEffect(() => {
    setSelectedModpack(null);
  }, [selectedVersion]);

  // 3. Listen to Rust progress events
  useEffect(() => {
    const setupListener = async () => {
      const unlisten = await listen<ProgressPayload>("download-progress", (event) => {
        const payload = event.payload;
        
        let displayStatus = "Downloading";
        if (payload.status === "paused") displayStatus = "Paused";
        if (payload.status === "extracting") displayStatus = "Extracting";
        if (payload.status === "completed") displayStatus = "Downloaded";
        if (payload.status === "failed") displayStatus = "Failed";

        if (payload.modpack_title) {
          const key = `${payload.game_id}:${payload.version}:${payload.modpack_title}`;
          setModpackStatuses((prev) => ({
            ...prev,
            [key]: {
              status: displayStatus,
              downloadedBytes: payload.downloaded_bytes,
              totalBytes: payload.total_bytes,
              speedBytesPerSec: payload.speed_bytes_per_sec,
              error: payload.error,
              installed_uploaded_at: payload.status === "completed" ? (
                modpacks.find(m => m.game_title.toLowerCase() === payload.game_id.toLowerCase() && m.modpack_title === payload.modpack_title)?.uploaded_at
              ) : prev[key]?.installed_uploaded_at,
            },
          }));
        } else {
          const key = `${payload.game_id}:${payload.version}`;
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
        }
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
  }, [modpacks]);

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
    localStorage.setItem("apiKey", tempApiKey);
    setDefaultGameFolder(tempFolder);
    setServerUrl(tempServer);
    setApiKey(tempApiKey);
    setSettingsModalOpen(false);
  };

  // Action: Start Game Download
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
        apiKey: apiKey,
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

  // Action: Pause Game Download
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

  // Action: Start Modpack Download
  const startDownloadModpack = async (game: Game, ver: GameVersion, mp: ServerModpack) => {
    const key = `${game.id}:${ver.version}:${mp.modpack_title}`;
    setModpackStatuses((prev) => ({
      ...prev,
      [key]: {
        status: "Downloading",
        downloadedBytes: prev[key]?.downloadedBytes || 0,
        totalBytes: mp.file_size_bytes,
        speedBytesPerSec: 0,
      },
    }));

    try {
      await invoke("start_download_modpack", {
        app: null,
        gameId: game.id,
        gameName: game.name,
        version: ver.version,
        modpackTitle: mp.modpack_title,
        uploadedAt: mp.uploaded_at,
        url: mp.url,
        gameFolder: defaultGameFolder,
        sizeBytes: mp.file_size_bytes,
        apiKey: apiKey,
      });
    } catch (err: any) {
      alert("Failed to start modpack download: " + err);
      setModpackStatuses((prev) => ({
        ...prev,
        [key]: {
          status: "NotDownloaded",
          downloadedBytes: 0,
          totalBytes: mp.file_size_bytes,
        },
      }));
    }
  };

  // Action: Pause Modpack Download
  const pauseDownloadModpack = async (game: Game, ver: GameVersion, mp: ServerModpack) => {
    try {
      await invoke("pause_download_modpack", {
        gameId: game.id,
        version: ver.version,
        modpackTitle: mp.modpack_title,
      });
    } catch (err: any) {
      alert("Failed to pause modpack: " + err);
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
    const gameUpdateAvailable = hasSomeVersionDownloaded && !isLatestDownloaded;
    
    // Check if any installed modpack for any version has an update
    let modpackUpdateAvailable = false;
    const gameModpacks = modpacks.filter(
      (m) => m.game_title.toLowerCase() === game.name.toLowerCase()
    );
    
    for (const ver of game.versions) {
      for (const mp of gameModpacks) {
        const key = `${game.id}:${ver.version}:${mp.modpack_title}`;
        const mpStatus = modpackStatuses[key];
        if (mpStatus && mpStatus.status === "Downloaded" && mpStatus.installed_uploaded_at) {
          if (mp.uploaded_at !== mpStatus.installed_uploaded_at) {
            modpackUpdateAvailable = true;
          }
        }
      }
    }

    const updateAvailable = gameUpdateAvailable || modpackUpdateAvailable;
    
    return { hasSomeVersionDownloaded, isLatestDownloaded, updateAvailable, gameUpdateAvailable, modpackUpdateAvailable, latestVersion };
  };

  // Filtered games based on search query
  const filteredGames = games.filter((g) =>
    g.name.toLowerCase().includes(searchQuery.toLowerCase())
  );

  const gameModpacks = selectedGame
    ? modpacks.filter((m) => m.game_title.toLowerCase() === selectedGame.name.toLowerCase())
    : [];

  const selectedModpackStatusKey = selectedGame && selectedVersion && selectedModpack
    ? `${selectedGame.id}:${selectedVersion.version}:${selectedModpack.modpack_title}`
    : "";
  
  const selectedModpackStatus = selectedModpackStatusKey ? modpackStatuses[selectedModpackStatusKey] : null;

  const selectedModpackUpdateAvailable = selectedModpack && selectedModpackStatus && selectedModpackStatus.status === "Downloaded" && selectedModpackStatus.installed_uploaded_at
    ? selectedModpack.uploaded_at !== selectedModpackStatus.installed_uploaded_at
    : false;

  const { gameUpdateAvailable } = selectedGame ? getGameVersionsStatus(selectedGame) : { gameUpdateAvailable: false };

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
              setTempApiKey(apiKey);
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
                    {updateAvailable && (
                      <span className="update-dot" style={{ marginRight: "6px" }} title="Update available!" />
                    )}
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
              <button className="update-btn danger" onClick={() => fetchGames(serverUrl, apiKey)}>
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
                    <label style={{ display: "flex", alignItems: "center", gap: "4px" }}>
                      Version
                      {gameUpdateAvailable && <span className="update-dot" title="Game update available!" />}
                    </label>
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
                    <label style={{ display: "flex", alignItems: "center", gap: "4px" }}>
                      Modpack
                      {selectedModpackUpdateAvailable && <span className="update-dot" title="Modpack update available!" />}
                    </label>
                    <select
                      className="custom-select"
                      value={selectedModpack ? selectedModpack.modpack_title : "NONE"}
                      onChange={(e) => {
                        const mp = gameModpacks.find((m) => m.modpack_title === e.target.value);
                        setSelectedModpack(mp || null);
                      }}
                    >
                      <option value="NONE">NONE</option>
                      {gameModpacks.map((m) => (
                        <option key={m.modpack_title} value={m.modpack_title}>
                          {m.modpack_title}
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

                  {selectedModpack && (
                    <div className="selector-field">
                      <label>Modpack Size</label>
                      <span>
                        {formatBytes(selectedModpack.file_size_bytes)}
                      </span>
                    </div>
                  )}
                </div>

                {/* Update Banner */}
                {(() => {
                  const { gameUpdateAvailable, latestVersion } = getGameVersionsStatus(selectedGame);
                  if (gameUpdateAvailable && selectedVersion?.version !== latestVersion.version) {
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
                  const gameKey = `${selectedGame.id}:${selectedVersion.version}`;
                  const gameStatusInfo = gameStatuses[gameKey] || {
                    status: "NotDownloaded",
                    downloadedBytes: 0,
                    totalBytes: selectedVersion.size_bytes,
                  };

                  const isGameDownloaded = gameStatusInfo.status === "Downloaded";

                  if (!isGameDownloaded) {
                    // Render download controls for the game version
                    const percent = gameStatusInfo.totalBytes > 0
                      ? Math.round((gameStatusInfo.downloadedBytes / gameStatusInfo.totalBytes) * 100)
                      : 0;

                    switch (gameStatusInfo.status) {
                      case "Downloading":
                        return (
                          <div className="action-area">
                            <div className="download-progress-container">
                              <div className="progress-info">
                                <span className="progress-status">Downloading Game</span>
                                <span>{percent}%</span>
                              </div>
                              <div className="progress-bar-wrapper">
                                <div className="progress-bar" style={{ width: `${percent}%` }} />
                              </div>
                              <div className="progress-stats">
                                <span>{formatBytes(gameStatusInfo.downloadedBytes)} / {formatBytes(gameStatusInfo.totalBytes)}</span>
                                <span className="progress-speed">{formatSpeed(gameStatusInfo.speedBytesPerSec)}</span>
                              </div>
                            </div>
                            <button
                              className="btn btn-secondary"
                              onClick={() => pauseDownload(selectedGame, selectedVersion)}
                            >
                              <Pause size={15} /> Pause Game Download
                            </button>
                          </div>
                        );

                      case "Paused":
                        return (
                          <div className="action-area">
                            <div className="download-progress-container">
                              <div className="progress-info">
                                <span className="progress-status">Paused Game Download</span>
                                <span>{percent}%</span>
                              </div>
                              <div className="progress-bar-wrapper">
                                <div className="progress-bar" style={{ width: `${percent}%`, opacity: 0.5 }} />
                              </div>
                              <div className="progress-stats">
                                <span>{formatBytes(gameStatusInfo.downloadedBytes)} / {formatBytes(gameStatusInfo.totalBytes)}</span>
                                <span>Paused</span>
                              </div>
                            </div>
                            <button
                              className="btn btn-primary"
                              onClick={() => startDownload(selectedGame, selectedVersion)}
                            >
                              <Download size={15} /> Resume Game Download
                            </button>
                          </div>
                        );

                      case "Extracting":
                        return (
                          <div className="action-area">
                            <div className="download-progress-container">
                              <div className="progress-info">
                                <span className="progress-status">Extracting Game</span>
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

                      case "Failed":
                        return (
                          <div className="action-area">
                            <div className="status-pill danger">
                              <span>Game download failed</span>
                              <span className="error-detail">
                                {gameStatusInfo.error || "Unknown error"}
                              </span>
                            </div>
                            <button
                              className="btn btn-primary"
                              onClick={() => startDownload(selectedGame, selectedVersion)}
                            >
                              <Download size={15} /> Retry Game Download
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
                              <Download size={15} /> Download Game
                            </button>
                          </div>
                        );
                    }
                  } else {
                    // Game version is downloaded!
                    // Check if a modpack is selected
                    if (selectedModpack) {
                      const modpackKey = `${selectedGame.id}:${selectedVersion.version}:${selectedModpack.modpack_title}`;
                      const modpackStatusInfo = modpackStatuses[modpackKey] || {
                        status: "NotDownloaded",
                        downloadedBytes: 0,
                        totalBytes: selectedModpack.file_size_bytes,
                      };

                      const isModpackDownloaded = modpackStatusInfo.status === "Downloaded";

                      if (!isModpackDownloaded) {
                        // Render download controls for the modpack
                        const percent = modpackStatusInfo.totalBytes > 0
                          ? Math.round((modpackStatusInfo.downloadedBytes / modpackStatusInfo.totalBytes) * 100)
                          : 0;

                        switch (modpackStatusInfo.status) {
                          case "Downloading":
                            return (
                              <div className="action-area">
                                <div className="download-progress-container">
                                  <div className="progress-info">
                                    <span className="progress-status">Downloading Modpack: {selectedModpack.modpack_title}</span>
                                    <span>{percent}%</span>
                                  </div>
                                  <div className="progress-bar-wrapper">
                                    <div className="progress-bar" style={{ width: `${percent}%` }} />
                                  </div>
                                  <div className="progress-stats">
                                    <span>{formatBytes(modpackStatusInfo.downloadedBytes)} / {formatBytes(modpackStatusInfo.totalBytes)}</span>
                                    <span className="progress-speed">{formatSpeed(modpackStatusInfo.speedBytesPerSec)}</span>
                                  </div>
                                </div>
                                <button
                                  className="btn btn-secondary"
                                  onClick={() => pauseDownloadModpack(selectedGame, selectedVersion, selectedModpack)}
                                >
                                  <Pause size={15} /> Pause Modpack Download
                                </button>
                              </div>
                            );

                          case "Paused":
                            return (
                              <div className="action-area">
                                <div className="download-progress-container">
                                  <div className="progress-info">
                                    <span className="progress-status">Paused Modpack Download</span>
                                    <span>{percent}%</span>
                                  </div>
                                  <div className="progress-bar-wrapper">
                                    <div className="progress-bar" style={{ width: `${percent}%`, opacity: 0.5 }} />
                                  </div>
                                  <div className="progress-stats">
                                    <span>{formatBytes(modpackStatusInfo.downloadedBytes)} / {formatBytes(modpackStatusInfo.totalBytes)}</span>
                                    <span>Paused</span>
                                  </div>
                                </div>
                                <button
                                  className="btn btn-primary"
                                  onClick={() => startDownloadModpack(selectedGame, selectedVersion, selectedModpack)}
                                >
                                  <Download size={15} /> Resume Modpack Download
                                </button>
                              </div>
                            );

                          case "Extracting":
                            return (
                              <div className="action-area">
                                <div className="download-progress-container">
                                  <div className="progress-info">
                                    <span className="progress-status">Extracting Modpack</span>
                                    <span>99%</span>
                                  </div>
                                  <div className="progress-bar-wrapper">
                                    <div className="progress-bar extracting" />
                                  </div>
                                  <div className="progress-stats">
                                    <span>Applying modpack files...</span>
                                    <span>Please wait</span>
                                  </div>
                                </div>
                                <button className="btn btn-primary" disabled>
                                  <Loader2 className="animate-spin" size={15} /> Extracting...
                                </button>
                              </div>
                            );

                          case "Failed":
                            return (
                              <div className="action-area">
                                <div className="status-pill danger">
                                  <span>Modpack download failed</span>
                                  <span className="error-detail">
                                    {modpackStatusInfo.error || "Unknown error"}
                                  </span>
                                </div>
                                <button
                                  className="btn btn-primary"
                                  onClick={() => startDownloadModpack(selectedGame, selectedVersion, selectedModpack)}
                                >
                                  <Download size={15} /> Retry Modpack Download
                                </button>
                              </div>
                            );

                          default:
                            return (
                              <div className="action-area">
                                <button
                                  className="btn btn-primary"
                                  onClick={() => startDownloadModpack(selectedGame, selectedVersion, selectedModpack)}
                                >
                                  <Download size={15} /> Download Modpack
                                </button>
                              </div>
                            );
                        }
                      }
                    }

                    // Ready to play!
                    return (
                      <div className="action-area">
                        {selectedModpackUpdateAvailable && (
                          <div className="update-banner">
                            <div className="update-banner-info">
                              <AlertTriangle size={14} />
                              <span>
                                An update is available for modpack <strong>{selectedModpack?.modpack_title}</strong>.
                              </span>
                            </div>
                            <button
                              className="update-btn"
                              onClick={() => selectedModpack && startDownloadModpack(selectedGame, selectedVersion, selectedModpack)}
                            >
                              Update Modpack
                            </button>
                          </div>
                        )}
                        <div className="status-pill success">
                          <CheckCircle2 size={14} />
                          Installed {selectedModpack ? `with modpack "${selectedModpack.modpack_title}"` : ""} and ready to play
                        </div>
                        <button
                          className="btn btn-success"
                          onClick={() => playGame(selectedGame, selectedVersion)}
                        >
                          <Play size={15} /> Play
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
                  placeholder="e.g. http://my-server.com"
                />
                <span className="hint">
                  Use "mock" for the built-in offline demo library.
                </span>
              </div>

              <div className="setting-group">
                <label>API Key / Download Key</label>
                <input
                  type="password"
                  value={tempApiKey}
                  onChange={(e) => setTempApiKey(e.target.value)}
                  placeholder="X-API-Key value"
                />
                <span className="hint">
                  Required for server authentication.
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
