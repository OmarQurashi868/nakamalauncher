import { useState, useEffect, useCallback } from "react";
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
  Folder,
  ExternalLink,
  Trash2,
  RefreshCw,
} from "lucide-react";
import "./App.css";

// ── Types ──────────────────────────────────────────────────────────────

interface GameVersion {
  uuid: string;
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
  notes: string | null;
  title_notes: string | null;
  app_id: string | null;
}

interface ServerModpack {
  uuid: string;
  id: number;
  game_title: string;
  modpack_title: string;
  file_name: string;
  file_size_bytes: number;
  uploaded_at: string;
  url: string;
  notes: string | null;
}

interface ProgressPayload {
  game_id: string;
  version: string;
  modpack_title: string | null;
  downloaded_bytes: number;
  total_bytes: number;
  speed_bytes_per_sec: number;
  status: string;
  error: string | null;
}

interface VersionStatus {
  status: string;
  downloadedBytes: number;
  totalBytes: number;
  speedBytesPerSec?: number;
  error?: string | null;
}

interface StagedState {
  game_id: string;
  version: string;
  modpack: string | null;
  swap_phase: string | null;
}

interface StorageSizes {
  game_id: string;
  game_name: string;
  total_bytes: number;
  versions: VersionSize[];
  modpacks: ModpackSize[];
}

interface VersionSize {
  version: string;
  size_bytes: number;
  staged: boolean;
}

interface ModpackSize {
  modpack_title: string;
  size_bytes: number;
  staged: boolean;
}

// ── Helpers ────────────────────────────────────────────────────────────

const formatBytes = (bytes: number) => {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + " " + sizes[i];
};

const formatSpeed = (bytesPerSec?: number) => {
  if (!bytesPerSec || bytesPerSec === 0) return "0 B/s";
  return `${formatBytes(bytesPerSec)}/s`;
};

// ── App ────────────────────────────────────────────────────────────────

function App() {
  // Settings
  const [defaultGameFolder, setDefaultGameFolder] = useState<string>(() =>
    localStorage.getItem("defaultGameFolder") || "C:\\Games"
  );
  const [serverUrl, setServerUrl] = useState<string>(() =>
    localStorage.getItem("serverUrl") || "mock"
  );
  const [apiKey, setApiKey] = useState<string>(() =>
    localStorage.getItem("apiKey") || ""
  );

  // Catalog
  const [games, setGames] = useState<Game[]>([]);
  const [modpacks, setModpacks] = useState<ServerModpack[]>([]);
  const [selectedGame, setSelectedGame] = useState<Game | null>(null);
  const [selectedVersion, setSelectedVersion] = useState<GameVersion | null>(null);
  const [selectedModpack, setSelectedModpack] = useState<ServerModpack | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [loadingGames, setLoadingGames] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [settingsModalOpen, setSettingsModalOpen] = useState(false);

  // Settings form
  const [tempFolder, setTempFolder] = useState(defaultGameFolder);
  const [tempServer, setTempServer] = useState(serverUrl);
  const [tempApiKey, setTempApiKey] = useState(apiKey);

  // Status tracking
  const [gameStatuses, setGameStatuses] = useState<Record<string, VersionStatus>>({});
  const [modpackStatuses, setModpackStatuses] = useState<Record<string, VersionStatus>>({});

  // New: Staged state per game (game_name → StagedState)
  const [stagedStates, setStagedStates] = useState<Record<string, StagedState>>({});

  // New: Storage sizes
  const [storageSizes, setStorageSizes] = useState<Record<string, StorageSizes>>({});

  // New: Active operation tracking per game
  const [activeOps, setActiveOps] = useState<Set<string>>(new Set());
  const [queuedKeys, setQueuedKeys] = useState<Set<string>>(new Set());

  // Delete confirmation modal
  const [deleteModal, setDeleteModal] = useState<{
    type: "version" | "modpack" | "game";
    gameName: string;
    label: string;
    sizeBytes: number;
  } | null>(null);

  // Move folder prompt
  const [movePrompt, setMovePrompt] = useState<{
    oldFolder: string;
    newFolder: string;
    totalBytes: number;
  } | null>(null);

  // ── Fetch catalog ────────────────────────────────────────────────────

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

  useEffect(() => { fetchGames(serverUrl, apiKey); }, [serverUrl, apiKey]);

  // ── Scan statuses ────────────────────────────────────────────────────

  const scanAllStatuses = useCallback(async () => {
    if (games.length === 0) return;
    const newStatuses: Record<string, VersionStatus> = {};
    const newModpackStatuses: Record<string, VersionStatus> = {};
    const newStaged: Record<string, StagedState> = {};

    for (const game of games) {
      // Get staged state
      try {
        const staged = await invoke<StagedState | null>("get_staged_state", {
          gameFolder: defaultGameFolder,
          gameName: game.name,
        });
        if (staged) newStaged[game.name] = staged;
      } catch (_) {}

      for (const ver of game.versions) {
        try {
          const res = await invoke<{ status: string; downloaded_bytes: number; total_bytes: number }>(
            "get_download_status",
            { gameFolder: defaultGameFolder, gameName: game.name, version: ver.version, gameId: game.id }
          );
          newStatuses[`${game.id}:${ver.version}`] = {
            status: res.status,
            downloadedBytes: res.downloaded_bytes || 0,
            totalBytes: ver.size_bytes,
          };
        } catch (_) {}

        const gameModpacks = modpacks.filter(
          (m) => m.game_title.toLowerCase() === game.name.toLowerCase()
        );
        for (const mp of gameModpacks) {
          try {
            const res = await invoke<{ status: string; downloaded_bytes: number; total_bytes: number; installed_uploaded_at: string | null }>(
              "get_modpack_status",
              { gameFolder: defaultGameFolder, gameName: game.name, version: ver.version, gameId: game.id, modpackTitle: mp.modpack_title }
            );
            newModpackStatuses[`${game.id}:${ver.version}:${mp.modpack_title}`] = {
              status: res.status,
              downloadedBytes: res.downloaded_bytes || 0,
              totalBytes: mp.file_size_bytes,
            };
          } catch (_) {}
        }
      }
    }

    setGameStatuses((prev) => ({ ...prev, ...newStatuses }));
    setModpackStatuses((prev) => ({ ...prev, ...newModpackStatuses }));
    setStagedStates(newStaged);
  }, [games, modpacks, defaultGameFolder]);

  useEffect(() => { scanAllStatuses(); }, [games, modpacks, defaultGameFolder]);

  // ── Scan storage sizes ───────────────────────────────────────────────

  const scanStorageSizes = useCallback(async () => {
    if (games.length === 0) return;
    try {
      const sizes = await invoke<StorageSizes[]>("get_storage_sizes", {
        gameFolder: defaultGameFolder,
        gamesJson: JSON.stringify(games),
      });
      const map: Record<string, StorageSizes> = {};
      for (const s of sizes) map[s.game_id] = s;
      setStorageSizes(map);
    } catch (_) {}
  }, [games, defaultGameFolder]);

  useEffect(() => { scanStorageSizes(); }, [games, defaultGameFolder, gameStatuses, modpackStatuses]);

  // ── Selection sync ───────────────────────────────────────────────────

  useEffect(() => {
    if (selectedGame && selectedGame.versions.length > 0) {
      const staged = stagedStates[selectedGame.name];
      if (staged) {
        const stagedVer = selectedGame.versions.find(v => v.version === staged.version);
        setSelectedVersion(stagedVer || selectedGame.versions[0]);
      } else {
        setSelectedVersion(selectedGame.versions[0]);
      }
      if (staged?.modpack) {
        const gameMps = modpacks.filter(
          (m) => m.game_title.toLowerCase() === selectedGame.name.toLowerCase()
        );
        const stagedMp = gameMps.find(m => m.modpack_title === staged.modpack);
        setSelectedModpack(stagedMp || null);
      } else {
        setSelectedModpack(null);
      }
    } else {
      setSelectedVersion(null);
      setSelectedModpack(null);
    }
  }, [selectedGame]);

  // ── Progress listener ────────────────────────────────────────────────

  useEffect(() => {
    const setup = async () => {
      const unlisten = await listen<ProgressPayload>("download-progress", (event) => {
        const p = event.payload;

        // Clear queued state when real progress arrives
        if (p.status === "downloading") {
          const qKey = p.modpack_title
            ? `${p.game_id}:${p.version}:${p.modpack_title}`
            : `${p.game_id}:${p.version}`;
          setQueuedKeys((prev) => { const next = new Set(prev); next.delete(qKey); return next; });
        }

        let displayStatus = "Downloading";
        if (p.status === "paused") displayStatus = "Paused";
        if (p.status === "extracting") displayStatus = "Extracting";
        if (p.status === "completed") displayStatus = "Downloaded";
        if (p.status === "failed") displayStatus = "Failed";
        if (p.status === "applied") displayStatus = "Applied";

        if (p.status === "applied") {
          // Re-scan staged state
          scanAllStatuses();
          scanStorageSizes();
          return;
        }

        if (p.modpack_title) {
          const key = `${p.game_id}:${p.version}:${p.modpack_title}`;
          setModpackStatuses((prev) => ({
            ...prev,
            [key]: {
              status: displayStatus,
              downloadedBytes: p.downloaded_bytes,
              totalBytes: p.total_bytes || prev[key]?.totalBytes || 0,
              speedBytesPerSec: p.speed_bytes_per_sec,
              error: p.error,
            },
          }));

          if (p.status === "completed") {
            // Auto-apply if this matches current selection
            if (
              selectedGame &&
              selectedVersion &&
              selectedModpack &&
              selectedGame.id === p.game_id &&
              selectedVersion.version === p.version &&
              selectedModpack.modpack_title === p.modpack_title
            ) {
              applyPermutation(selectedGame, selectedVersion, selectedModpack);
            }
          }
        } else {
          const key = `${p.game_id}:${p.version}`;
          setGameStatuses((prev) => ({
            ...prev,
            [key]: {
              status: displayStatus,
              downloadedBytes: p.downloaded_bytes,
              totalBytes: p.total_bytes || prev[key]?.totalBytes || 0,
              speedBytesPerSec: p.speed_bytes_per_sec,
              error: p.error,
            },
          }));

          if (p.status === "completed") {
            // Auto-apply if this matches current selection
            if (
              selectedGame &&
              selectedVersion &&
              selectedGame.id === p.game_id &&
              selectedVersion.version === p.version
            ) {
              applyPermutation(selectedGame, selectedVersion, selectedModpack || null);
            }
          }
        }

        // Track active operations
        if (p.status === "downloading" || p.status === "extracting") {
          setActiveOps((prev) => new Set(prev).add(p.game_id));
        } else if (p.status === "completed" || p.status === "failed" || p.status === "paused") {
          setActiveOps((prev) => {
            const next = new Set(prev);
            next.delete(p.game_id);
            return next;
          });
        }
      });
      return unlisten;
    };

    let unlistenFn: (() => void) | undefined;
    setup().then((fn) => { unlistenFn = fn; });
    return () => { if (unlistenFn) unlistenFn(); };
  }, [selectedGame, selectedVersion, selectedModpack]);

  // ── Actions ──────────────────────────────────────────────────────────

  const browseFolder = async () => {
    try {
      const selected = await invoke<string>("select_directory");
      setTempFolder(selected);
    } catch (err: any) {
      if (err !== "Cancelled") alert("Failed to pick folder: " + err);
    }
  };

  const saveSettings = async () => {
    // Check if folder changed and there's content to move
    if (tempFolder !== defaultGameFolder && defaultGameFolder !== "C:\\Games") {
      const totalBytes = Object.values(storageSizes).reduce((sum, s) => sum + s.total_bytes, 0);
      if (totalBytes > 0) {
        setMovePrompt({ oldFolder: defaultGameFolder, newFolder: tempFolder, totalBytes });
        return; // Will call saveSettings again after move prompt resolved
      }
    }
    localStorage.setItem("defaultGameFolder", tempFolder);
    localStorage.setItem("serverUrl", tempServer);
    localStorage.setItem("apiKey", tempApiKey);
    setDefaultGameFolder(tempFolder);
    setServerUrl(tempServer);
    setApiKey(tempApiKey);
    setSettingsModalOpen(false);
  };

  const handleMoveConfirm = async (move: boolean) => {
    if (move) {
      try {
        await invoke("move_game_folder", {
          oldFolder: movePrompt!.oldFolder,
          newFolder: movePrompt!.newFolder,
        });
      } catch (err: any) {
        alert("Failed to move game folder: " + err);
      }
    }
    setMovePrompt(null);
    // Continue with save
    localStorage.setItem("defaultGameFolder", tempFolder);
    localStorage.setItem("serverUrl", tempServer);
    localStorage.setItem("apiKey", tempApiKey);
    setDefaultGameFolder(tempFolder);
    setServerUrl(tempServer);
    setApiKey(tempApiKey);
    setSettingsModalOpen(false);
  };

  const startDownload = async (game: Game, ver: GameVersion) => {
    const key = `${game.id}:${ver.version}`;
    setQueuedKeys((prev) => new Set(prev).add(key));
    setGameStatuses((prev) => ({
      ...prev,
      [key]: { status: "Downloading", downloadedBytes: prev[key]?.downloadedBytes || 0, totalBytes: ver.size_bytes, speedBytesPerSec: 0 },
    }));
    try {
      await invoke("start_download", {
        gameId: game.id, gameName: game.name, version: ver.version,
        url: ver.url, gameFolder: defaultGameFolder, sizeBytes: ver.size_bytes,
        apiKey: apiKey, uuid: ver.uuid,
      });
    } catch (err: any) {
      alert("Failed to start download: " + err);
      setGameStatuses((prev) => ({
        ...prev,
        [key]: { status: "NotDownloaded", downloadedBytes: 0, totalBytes: ver.size_bytes },
      }));
    }
  };

  const pauseDownload = async (game: Game, ver: GameVersion) => {
    try { await invoke("pause_download", { gameId: game.id, version: ver.version }); }
    catch (err: any) { alert("Failed to pause: " + err); }
  };

  const startDownloadModpack = async (game: Game, ver: GameVersion, mp: ServerModpack) => {
    const key = `${game.id}:${ver.version}:${mp.modpack_title}`;
    setQueuedKeys((prev) => new Set(prev).add(key));
    setModpackStatuses((prev) => ({
      ...prev,
      [key]: { status: "Downloading", downloadedBytes: prev[key]?.downloadedBytes || 0, totalBytes: mp.file_size_bytes, speedBytesPerSec: 0 },
    }));
    try {
      await invoke("start_download_modpack", {
        gameId: game.id, gameName: game.name, version: ver.version,
        modpackTitle: mp.modpack_title, url: mp.url, gameFolder: defaultGameFolder,
        sizeBytes: mp.file_size_bytes, apiKey: apiKey, uuid: mp.uuid,
      });
    } catch (err: any) {
      alert("Failed to start modpack download: " + err);
      setModpackStatuses((prev) => ({
        ...prev,
        [key]: { status: "NotDownloaded", downloadedBytes: 0, totalBytes: mp.file_size_bytes },
      }));
    }
  };

  const pauseDownloadModpack = async (game: Game, ver: GameVersion, mp: ServerModpack) => {
    try { await invoke("pause_download_modpack", { gameId: game.id, version: ver.version, modpackTitle: mp.modpack_title }); }
    catch (err: any) { alert("Failed to pause modpack: " + err); }
  };

  const cancelDownload = async (game: Game, ver: GameVersion) => {
    const key = `${game.id}:${ver.version}`;
    setQueuedKeys((prev) => { const next = new Set(prev); next.delete(key); return next; });
    try {
      await invoke("cancel_download", { gameFolder: defaultGameFolder, gameName: game.name, version: ver.version, gameId: game.id });
      await scanAllStatuses();
    } catch (err: any) { alert("Failed to cancel: " + err); }
  };

  const cancelDownloadModpack = async (game: Game, ver: GameVersion, mp: ServerModpack) => {
    const key = `${game.id}:${ver.version}:${mp.modpack_title}`;
    setQueuedKeys((prev) => { const next = new Set(prev); next.delete(key); return next; });
    try {
      await invoke("cancel_download_modpack", { gameFolder: defaultGameFolder, gameName: game.name, version: ver.version, gameId: game.id, modpackTitle: mp.modpack_title });
      await scanAllStatuses();
    } catch (err: any) { alert("Failed to cancel modpack: " + err); }
  };

  const applyPermutation = async (game: Game, ver: GameVersion, mp: ServerModpack | null) => {
    try {
      await invoke("apply_permutation", {
        gameFolder: defaultGameFolder, gameId: game.id, gameName: game.name,
        version: ver.version, modpack: mp ? mp.modpack_title : null,
      });
      await scanAllStatuses();
      await scanStorageSizes();
    } catch (err: any) {
      alert("Failed to apply: " + err);
    }
  };

  const playGame = async (game: Game, ver: GameVersion) => {
    try {
      await invoke("launch_game", {
        gameFolder: defaultGameFolder, gameName: game.name,
        version: ver.version, launchPath: ver.launch_path,
      });
    } catch (err: any) { alert("Failed to launch game: " + err); }
  };

  const deleteVersion = async (gameName: string, version: string) => {
    try {
      await invoke("delete_version", { gameFolder: defaultGameFolder, gameName, version });
      await scanAllStatuses();
      await scanStorageSizes();
    } catch (err: any) { alert("Failed to delete version: " + err); }
    setDeleteModal(null);
  };

  const deleteModpack = async (gameName: string, modpackTitle: string) => {
    try {
      await invoke("delete_modpack", { gameFolder: defaultGameFolder, gameName, modpackTitle });
      await scanAllStatuses();
      await scanStorageSizes();
    } catch (err: any) { alert("Failed to delete modpack: " + err); }
    setDeleteModal(null);
  };

  const deleteGame = async (gameName: string) => {
    try {
      await invoke("delete_game", { gameFolder: defaultGameFolder, gameName });
      setSelectedGame(null);
      await scanAllStatuses();
      await scanStorageSizes();
    } catch (err: any) { alert("Failed to delete game: " + err); }
    setDeleteModal(null);
  };

  // ── Button state ─────────────────────────────────────────────────────

  const getButtonState = () => {
    if (!selectedGame || !selectedVersion) return { label: "Select a game", disabled: true, action: null };

    const staged = stagedStates[selectedGame.name];
    const isStaged = staged &&
      staged.version === selectedVersion.version &&
      (staged.modpack || null) === (selectedModpack?.modpack_title || null);

    if (isStaged) {
      const hasOp = activeOps.has(selectedGame.id);
      return {
        label: hasOp ? "Operation in progress..." : "Play",
        disabled: hasOp,
        action: "play" as const,
        variant: "success" as const,
      };
    }

    const verKey = `${selectedGame.id}:${selectedVersion.version}`;
    const verCached = gameStatuses[verKey]?.status === "Downloaded";
    // Version is also "available" if it's the one currently staged (moved out of cache)
    const verStaged = staged?.version === selectedVersion.version;
    const verAvailable = verCached || verStaged;

    let mpAvailable = false;
    if (selectedModpack) {
      const mpKey = `${selectedGame.id}:${selectedVersion.version}:${selectedModpack.modpack_title}`;
      const mpCached = modpackStatuses[mpKey]?.status === "Downloaded";
      const mpStaged = staged?.modpack === selectedModpack.modpack_title;
      mpAvailable = mpCached || mpStaged;
    } else {
      mpAvailable = true; // "none" doesn't need cache
    }

    const verStatus = gameStatuses[verKey]?.status;
    const mpStatus = selectedModpack
      ? modpackStatuses[`${selectedGame.id}:${selectedVersion.version}:${selectedModpack.modpack_title}`]?.status
      : null;

    // In-progress states
    if (verStatus === "Downloading" || verStatus === "Extracting") {
      return { label: "Downloading...", disabled: true, action: "progress" as const, variant: "primary" as const };
    }
    if (verStatus === "Paused") {
      return { label: "Resume Download", disabled: false, action: "resume" as const, variant: "primary" as const };
    }
    if (verStatus === "Failed") {
      return { label: "Retry Download", disabled: false, action: "retry" as const, variant: "primary" as const };
    }
    if (mpStatus === "Downloading" || mpStatus === "Extracting") {
      return { label: "Downloading Modpack...", disabled: true, action: "progress" as const, variant: "primary" as const };
    }
    if (mpStatus === "Paused") {
      return { label: "Resume Modpack", disabled: false, action: "resumeMp" as const, variant: "primary" as const };
    }
    if (mpStatus === "Failed") {
      return { label: "Retry Modpack", disabled: false, action: "retryMp" as const, variant: "primary" as const };
    }

    // Cache/Staged states
    if (verAvailable && mpAvailable) {
      return { label: "Apply", disabled: false, action: "apply" as const, variant: "primary" as const };
    }
    if (!verAvailable && !mpAvailable) {
      return { label: "Download", disabled: false, action: "download" as const, variant: "primary" as const };
    }
    if (!verAvailable && mpAvailable) {
      return { label: "Download (version only)", disabled: false, action: "downloadVer" as const, variant: "primary" as const };
    }
    // verAvailable && !mpAvailable
    return { label: "Download (modpack only)", disabled: false, action: "downloadMp" as const, variant: "primary" as const };
  };

  // ── Derived data ─────────────────────────────────────────────────────

  const filteredGames = games
    .filter((g) => g.name.toLowerCase().includes(searchQuery.toLowerCase()))
    .sort((a, b) => a.name.localeCompare(b.name));

  const gameModpacks = selectedGame
    ? modpacks.filter((m) => m.game_title.toLowerCase() === selectedGame.name.toLowerCase())
    : [];

  const buttonState = getButtonState();

  const selectedGameSizes = selectedGame ? storageSizes[selectedGame.id] : null;

  const handleMainButton = () => {
    if (!selectedGame || !selectedVersion) return;
    switch (buttonState.action) {
      case "play": playGame(selectedGame, selectedVersion); break;
      case "apply": applyPermutation(selectedGame, selectedVersion, selectedModpack || null); break;
      case "download": startDownload(selectedGame, selectedVersion); break;
      case "downloadVer": startDownload(selectedGame, selectedVersion); break;
      case "downloadMp": selectedModpack && startDownloadModpack(selectedGame, selectedVersion, selectedModpack); break;
      case "resume": startDownload(selectedGame, selectedVersion); break;
      case "retry": startDownload(selectedGame, selectedVersion); break;
      case "resumeMp": selectedModpack && startDownloadModpack(selectedGame, selectedVersion, selectedModpack); break;
      case "retryMp": selectedModpack && startDownloadModpack(selectedGame, selectedVersion, selectedModpack); break;
    }
  };

  // ── Render ───────────────────────────────────────────────────────────

  return (
    <div className="app-container">
      {/* Header */}
      <header className="app-header">
        <div className="logo-section">
          <img src="/nakama_logo.png" className="logo-icon" alt="Nakama" />
          <h1>Nakama Launcher</h1>
        </div>
        <div className="header-actions">
          <div className="folder-info">
            <Folder size={12} />
            <span>{defaultGameFolder}</span>
          </div>
          <button
            className="settings-trigger"
            onClick={() => fetchGames(serverUrl, apiKey)}
            title="Refresh catalog"
          >
            <RefreshCw size={16} />
          </button>
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

      {/* Body */}
      <div className="app-body">
        {/* Sidebar */}
        <aside className="sidebar">
          <div className="search-section">
            <div className="search-input-wrapper">
              <Search className="search-icon" />
              <input type="text" placeholder="Search games..." value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} />
            </div>
          </div>

          <div className="game-list">
            {loadingGames ? (
              <div className="list-state"><Loader2 className="animate-spin" size={20} /><span>Loading catalog...</span></div>
            ) : filteredGames.length === 0 ? (
              <div className="list-state">No games found. Check server settings.</div>
            ) : (
              filteredGames.map((game) => {
                const isActive = selectedGame?.id === game.id;
                const staged = stagedStates[game.name];
                const sizes = storageSizes[game.id];

                return (
                  <div key={game.id} className={`game-card ${isActive ? "active" : ""}`} onClick={() => setSelectedGame(game)}>
                    {game.icon_url ? (
                      <img src={game.icon_url} alt={game.name} className="game-card-icon" onError={(e) => { (e.currentTarget as any).style.display = "none"; }} />
                    ) : (
                      <div className="game-card-icon"><Gamepad2 size={18} /></div>
                    )}
                    <div className="game-card-info">
                      <div className="game-card-name">{game.name}</div>
                      <div className="game-card-status-row">
                        {staged && <span className="badge badge-installed">Ready</span>}
                        {sizes && sizes.total_bytes > 0 && (
                          <span className="badge" style={{ background: "var(--bg-tertiary)", color: "var(--text-secondary)", fontSize: "10px" }}>
                            {formatBytes(sizes.total_bytes)}
                          </span>
                        )}
                      </div>
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </aside>

        {/* Main Panel */}
        <main className="main-content">
          {errorMsg && (
            <div className="update-banner error-banner">
              <div className="update-banner-info"><AlertTriangle size={14} /><span>Error: {errorMsg}</span></div>
              <button className="update-btn danger" onClick={() => fetchGames(serverUrl, apiKey)}>Retry</button>
            </div>
          )}

          {selectedGame ? (
            <div className="game-details">
              {/* Game Header */}
              <div className="details-header">
                {selectedGame.icon_url ? (
                  <img src={selectedGame.icon_url} alt={selectedGame.name} className="details-icon" onError={(e) => { (e.currentTarget as any).style.display = "none"; }} />
                ) : (
                  <div className="details-icon"><Gamepad2 size={36} /></div>
                )}
                <div className="details-title-desc">
                  <h2>{selectedGame.name}</h2>
                  {selectedGame.app_id && (
                    <a
                      href={`https://store.steampowered.com/app/${selectedGame.app_id}/`}
                      target="_blank"
                      rel="noreferrer"
                      className="steam-link"
                      style={{ display: "inline-flex", alignItems: "center", gap: "4px", fontSize: "13px", color: "var(--accent-primary)", textDecoration: "none", marginTop: "2px" }}
                    >
                      <ExternalLink size={12} /> Steam Store
                    </a>
                  )}
                  <p className="details-desc">{selectedGame.description}</p>
                </div>
              </div>

              {/* Notes */}
              {(selectedGame.notes || selectedGame.title_notes) && (
                <div className="notes-box" style={{
                  background: "var(--bg-tertiary)", border: "1px solid var(--border-color)",
                  borderRadius: "8px", padding: "12px 16px", marginBottom: "16px", fontSize: "13px",
                  color: "var(--text-secondary)", lineHeight: "1.5",
                }}>
                  {selectedGame.title_notes && <p style={{ margin: "0 0 4px 0" }}>{selectedGame.title_notes}</p>}
                  {selectedGame.notes && <p style={{ margin: 0, fontStyle: "italic" }}>Version note: {selectedGame.notes}</p>}
                </div>
              )}

              {/* Version & Action Panel */}
              <div className="version-card">
                {/* Selectors */}
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
                        <option key={v.version} value={v.version}>{v.version}</option>
                      ))}
                    </select>
                  </div>

                  <div className="selector-field">
                    <label>Modpack</label>
                    <select
                      className="custom-select"
                      value={selectedModpack ? selectedModpack.modpack_title : "NONE"}
                      onChange={(e) => {
                        const mp = gameModpacks.find((m) => m.modpack_title === e.target.value);
                        setSelectedModpack(mp || null);
                      }}
                    >
                      <option value="NONE">None</option>
                      {gameModpacks.map((m) => (
                        <option key={m.modpack_title} value={m.modpack_title}>{m.modpack_title}</option>
                      ))}
                    </select>
                  </div>

                  <div className="selector-field">
                    <label>Size</label>
                    <span>{selectedVersion ? formatBytes(selectedVersion.size_bytes) : "—"}</span>
                  </div>

                  {selectedModpack && (
                    <div className="selector-field">
                      <label>Modpack Size</label>
                      <span>{formatBytes(selectedModpack.file_size_bytes)}</span>
                    </div>
                  )}
                </div>

                {/* Staged status indicator */}
                {selectedVersion && stagedStates[selectedGame.name] && (
                  (() => {
                    const staged = stagedStates[selectedGame.name];
                    const isMatch = staged.version === selectedVersion.version &&
                      (staged.modpack || null) === (selectedModpack?.modpack_title || null);
                    if (isMatch) {
                      return (
                        <div className="status-pill success" style={{ marginBottom: "12px" }}>
                          <CheckCircle2 size={14} />
                          Staged: {staged.version}{staged.modpack ? ` + ${staged.modpack}` : ""}
                        </div>
                      );
                    } else {
                      return (
                        <div className="status-pill" style={{ marginBottom: "12px", background: "var(--bg-tertiary)", color: "var(--text-secondary)" }}>
                          <Folder size={14} />
                          Staged: {staged.version}{staged.modpack ? ` + ${staged.modpack}` : " (vanilla)"}
                        </div>
                      );
                    }
                  })()
                )}

                {/* Action area */}
                {selectedVersion && buttonState.action === "progress" && (() => {
                  const verKey = `${selectedGame.id}:${selectedVersion.version}`;
                  const verStatus = gameStatuses[verKey];
                  const mpStatus = selectedModpack ? modpackStatuses[`${selectedGame.id}:${selectedVersion.version}:${selectedModpack.modpack_title}`] : null;

                  const activeDownload = mpStatus && (mpStatus.status === "Downloading" || mpStatus.status === "Paused" || mpStatus.status === "Extracting" || mpStatus.status === "Failed")
                    ? { ...mpStatus, isModpack: true, title: selectedModpack!.modpack_title }
                    : (verStatus && (verStatus.status === "Downloading" || verStatus.status === "Paused" || verStatus.status === "Extracting" || verStatus.status === "Failed")
                      ? { ...verStatus, isModpack: false, title: selectedVersion.version }
                      : null);

                  if (!activeDownload) return null;

                  const activeKey = activeDownload.isModpack
                    ? `${selectedGame!.id}:${selectedVersion!.version}:${(activeDownload as any).title}`
                    : `${selectedGame!.id}:${selectedVersion!.version}`;
                  const isQueued = queuedKeys.has(activeKey);

                  const percent = activeDownload.totalBytes > 0
                    ? Math.round((activeDownload.downloadedBytes / activeDownload.totalBytes) * 100) : 0;

                  if (activeDownload.status === "Downloading") {
                    if (isQueued) {
                      return (
                        <div className="action-area">
                          <div className="download-progress-container">
                            <div className="progress-info">
                              <span className="progress-status">Queued — {activeDownload.isModpack ? "Modpack" : "Game"}: {activeDownload.title}</span>
                              <span>—</span>
                            </div>
                            <div className="progress-stats" style={{ justifyContent: "center" }}>
                              <span>Waiting for download slot...</span>
                            </div>
                          </div>
                          <button className="btn btn-secondary" style={{ color: "var(--accent-danger)", borderColor: "var(--accent-danger)" }} onClick={() =>
                            activeDownload.isModpack
                              ? cancelDownloadModpack(selectedGame!, selectedVersion!, selectedModpack!)
                              : cancelDownload(selectedGame!, selectedVersion!)
                          }><Trash2 size={15} /> Cancel</button>
                        </div>
                      );
                    }
                    return (
                      <div className="action-area">
                        <div className="download-progress-container">
                          <div className="progress-info">
                            <span className="progress-status">Downloading {activeDownload.isModpack ? "Modpack" : "Game"}: {activeDownload.title}</span>
                            <span>{percent}%</span>
                          </div>
                          <div className="progress-bar-wrapper"><div className="progress-bar" style={{ width: `${percent}%` }} /></div>
                          <div className="progress-stats">
                            <span>{formatBytes(activeDownload.downloadedBytes)} / {formatBytes(activeDownload.totalBytes)}</span>
                            <span className="progress-speed">{formatSpeed(activeDownload.speedBytesPerSec)}</span>
                          </div>
                        </div>
                        <div style={{ display: "flex", gap: "8px" }}>
                          <button className="btn btn-secondary" onClick={() =>
                            activeDownload.isModpack
                              ? pauseDownloadModpack(selectedGame!, selectedVersion!, selectedModpack!)
                              : pauseDownload(selectedGame!, selectedVersion!)
                          }><Pause size={15} /> Pause</button>
                          <button className="btn btn-secondary" style={{ color: "var(--accent-danger)", borderColor: "var(--accent-danger)" }} onClick={() =>
                            activeDownload.isModpack
                              ? cancelDownloadModpack(selectedGame!, selectedVersion!, selectedModpack!)
                              : cancelDownload(selectedGame!, selectedVersion!)
                          }><Trash2 size={15} /> Cancel</button>
                        </div>
                      </div>
                    );
                  }
                  if (activeDownload.status === "Paused") {
                    return (
                      <div className="action-area">
                        <div className="download-progress-container">
                          <div className="progress-info"><span className="progress-status">Paused</span><span>{percent}%</span></div>
                          <div className="progress-bar-wrapper"><div className="progress-bar" style={{ width: `${percent}%`, opacity: 0.5 }} /></div>
                          <div className="progress-stats"><span>{formatBytes(activeDownload.downloadedBytes)} / {formatBytes(activeDownload.totalBytes)}</span><span>Paused</span></div>
                        </div>
                        <div style={{ display: "flex", gap: "8px" }}>
                          <button className="btn btn-primary" onClick={() =>
                            activeDownload.isModpack
                              ? startDownloadModpack(selectedGame!, selectedVersion!, selectedModpack!)
                              : startDownload(selectedGame!, selectedVersion!)
                          }><Download size={15} /> Resume</button>
                          <button className="btn btn-secondary" style={{ color: "var(--accent-danger)", borderColor: "var(--accent-danger)" }} onClick={() =>
                            activeDownload.isModpack
                              ? cancelDownloadModpack(selectedGame!, selectedVersion!, selectedModpack!)
                              : cancelDownload(selectedGame!, selectedVersion!)
                          }><Trash2 size={15} /> Cancel</button>
                        </div>
                      </div>
                    );
                  }
                  if (activeDownload.status === "Extracting") {
                    return (
                      <div className="action-area">
                        <div className="download-progress-container">
                          <div className="progress-info"><span className="progress-status">Extracting...</span><span>99%</span></div>
                          <div className="progress-bar-wrapper"><div className="progress-bar extracting" /></div>
                          <div className="progress-stats"><span>Unpacking files...</span><span>Please wait</span></div>
                        </div>
                        <button className="btn btn-primary" disabled><Loader2 className="animate-spin" size={15} /> Extracting...</button>
                      </div>
                    );
                  }
                  if (activeDownload.status === "Failed") {
                    return (
                      <div className="action-area">
                        <div className="status-pill danger"><span>Download failed</span><span className="error-detail">{activeDownload.error || "Unknown error"}</span></div>
                        <button className="btn btn-primary" onClick={() =>
                          activeDownload.isModpack
                            ? startDownloadModpack(selectedGame, selectedVersion, selectedModpack!)
                            : startDownload(selectedGame, selectedVersion)
                        }><Download size={15} /> Retry</button>
                      </div>
                    );
                  }
                  return null;
                })()}

                {/* Main button */}
                {selectedVersion && buttonState.action !== "progress" && (
                  <div className="action-area">
                    <button
                      className={`btn btn-${buttonState.variant || "primary"}`}
                      disabled={buttonState.disabled}
                      onClick={handleMainButton}
                    >
                      {buttonState.action === "play" ? <Play size={15} /> :
                       buttonState.action === "apply" ? <RefreshCw size={15} /> :
                       <Download size={15} />}
                      {buttonState.label}
                    </button>
                  </div>
                )}
              </div>

              {/* Storage & Delete Section */}
              {selectedGameSizes && (selectedGameSizes.total_bytes > 0) && (
                <div className="version-card" style={{ marginTop: "16px" }}>
                  <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "12px" }}>Storage — {formatBytes(selectedGameSizes.total_bytes)} total</h3>

                  {/* Versions */}
                  {selectedGameSizes.versions.length > 0 && (
                    <div style={{ marginBottom: "12px" }}>
                      <div style={{ fontSize: "12px", color: "var(--text-secondary)", marginBottom: "6px" }}>Versions</div>
                      {selectedGameSizes.versions.map((v) => (
                        <div key={v.version} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "6px 8px", borderRadius: "4px", fontSize: "12px", background: "var(--bg-secondary)" }}>
                          <span>
                            {v.version}
                            {v.staged && <span style={{ marginLeft: "6px", color: "var(--accent-success)", fontSize: "11px" }}>(staged)</span>}
                          </span>
                          <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                            <span style={{ color: "var(--text-secondary)" }}>{formatBytes(v.size_bytes)}</span>
                            <span title={v.staged ? "This version is staged" : "Delete version"}>
                              <button
                                className="btn-delete-icon"
                                disabled={v.staged}
                                onClick={() => setDeleteModal({
                                  type: "version", gameName: selectedGame.name, label: v.version, sizeBytes: v.size_bytes
                                })}
                                style={{ background: "none", border: "none", color: v.staged ? "var(--text-tertiary)" : "var(--text-secondary)", cursor: v.staged ? "not-allowed" : "pointer", padding: "2px" }}
                              >
                                <Trash2 size={14} />
                              </button>
                            </span>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}

                  {/* Modpacks */}
                  {selectedGameSizes.modpacks.length > 0 && (
                    <div style={{ marginBottom: "12px" }}>
                      <div style={{ fontSize: "12px", color: "var(--text-secondary)", marginBottom: "6px" }}>Modpacks</div>
                      {selectedGameSizes.modpacks.map((m) => (
                        <div key={m.modpack_title} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "6px 8px", borderRadius: "4px", fontSize: "12px", background: "var(--bg-secondary)" }}>
                          <span>
                            {m.modpack_title}
                            {m.staged && <span style={{ marginLeft: "6px", color: "var(--accent-success)", fontSize: "11px" }}>(applied)</span>}
                          </span>
                          <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                            <span style={{ color: "var(--text-secondary)" }}>{formatBytes(m.size_bytes)}</span>
                            <span title={m.staged ? "This modpack is applied" : "Delete modpack"}>
                              <button
                                className="btn-delete-icon"
                                disabled={m.staged}
                                onClick={() => setDeleteModal({
                                  type: "modpack", gameName: selectedGame.name, label: m.modpack_title, sizeBytes: m.size_bytes
                                })}
                                style={{ background: "none", border: "none", color: m.staged ? "var(--text-tertiary)" : "var(--text-secondary)", cursor: m.staged ? "not-allowed" : "pointer", padding: "2px" }}
                              >
                                <Trash2 size={14} />
                              </button>
                            </span>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}

                  {/* Delete entire game */}
                  <button
                    className="btn btn-secondary"
                    style={{ color: "var(--accent-danger)", borderColor: "var(--accent-danger)", width: "100%", marginTop: "8px" }}
                    onClick={() => setDeleteModal({
                      type: "game", gameName: selectedGame.name, label: selectedGame.name, sizeBytes: selectedGameSizes.total_bytes
                    })}
                  >
                    <Trash2 size={14} /> Delete Game (all versions & modpacks)
                  </button>
                </div>
              )}

              {/* Modpack notes */}
              {selectedModpack?.notes && (
                <div className="notes-box" style={{
                  background: "var(--bg-tertiary)", border: "1px solid var(--border-color)",
                  borderRadius: "8px", padding: "12px 16px", marginTop: "16px", fontSize: "13px",
                  color: "var(--text-secondary)", lineHeight: "1.5",
                }}>
                  <strong>{selectedModpack.modpack_title}:</strong> {selectedModpack.notes}
                </div>
              )}
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

      {/* Settings Modal */}
      {settingsModalOpen && (
        <div className="modal-overlay">
          <div className="modal-content">
            <header className="modal-header">
              <h3><Settings size={15} /> Settings</h3>
              <button className="modal-close" onClick={() => setSettingsModalOpen(false)}>✕</button>
            </header>
            <div className="settings-content">
              <div className="setting-group">
                <label>Game Folder</label>
                <div className="input-browse-wrapper">
                  <input type="text" value={tempFolder} onChange={(e) => setTempFolder(e.target.value)} placeholder="e.g. C:\Games" />
                  <button className="browse-btn" onClick={browseFolder}><FolderOpen size={14} /> Browse</button>
                </div>
                <span className="hint">
                  Cache: [Folder]\.nakama\{'<'}Game{'>'}\versions\{'<'}version{'>'} | Active: [Folder]\{'<'}Game{'>'}
                </span>
              </div>
              <div className="setting-group">
                <label>Metadata Server URL</label>
                <input type="text" value={tempServer} onChange={(e) => setTempServer(e.target.value)} placeholder="e.g. http://my-server.com" />
                <span className="hint">Use "mock" for the built-in offline demo library.</span>
              </div>
              <div className="setting-group">
                <label>API Key / Download Key</label>
                <input type="password" value={tempApiKey} onChange={(e) => setTempApiKey(e.target.value)} placeholder="X-API-Key value" />
                <span className="hint">Required for server authentication.</span>
              </div>
            </div>
            <footer className="settings-actions">
              <button className="modal-btn modal-btn-cancel" onClick={() => setSettingsModalOpen(false)}>Cancel</button>
              <button className="modal-btn modal-btn-save" onClick={saveSettings}>Save</button>
            </footer>
          </div>
        </div>
      )}

      {/* Delete Confirmation Modal */}
      {deleteModal && (
        <div className="modal-overlay">
          <div className="modal-content" style={{ maxWidth: "420px" }}>
            <header className="modal-header">
              <h3><Trash2 size={15} /> Confirm Delete</h3>
              <button className="modal-close" onClick={() => setDeleteModal(null)}>✕</button>
            </header>
            <div style={{ padding: "16px", fontSize: "14px", lineHeight: "1.5" }}>
              {deleteModal.type === "game" ? (
                <p>This will delete <strong>everything</strong> in the game folder for <strong>{deleteModal.label}</strong> — all cached versions, modpacks, and the staged copy.</p>
              ) : deleteModal.type === "version" ? (
                <p>Delete version <strong>{deleteModal.label}</strong> from cache ({formatBytes(deleteModal.sizeBytes)})?</p>
              ) : (
                <p>Delete modpack <strong>{deleteModal.label}</strong> from cache ({formatBytes(deleteModal.sizeBytes)})?</p>
              )}
              {deleteModal.type === "game" && (
                <p style={{ color: "var(--accent-danger)", fontWeight: 600, marginTop: "8px" }}>
                  Total size: {formatBytes(deleteModal.sizeBytes)}. This cannot be undone.
                </p>
              )}
            </div>
            <footer className="settings-actions">
              <button className="modal-btn modal-btn-cancel" onClick={() => setDeleteModal(null)}>Cancel</button>
              <button
                className="modal-btn modal-btn-save"
                style={{ background: "var(--accent-danger)", borderColor: "var(--accent-danger)" }}
                onClick={() => {
                  if (deleteModal.type === "version") deleteVersion(deleteModal.gameName, deleteModal.label);
                  else if (deleteModal.type === "modpack") deleteModpack(deleteModal.gameName, deleteModal.label);
                  else deleteGame(deleteModal.gameName);
                }}
              >
                Delete
              </button>
            </footer>
          </div>
        </div>
      )}

      {/* Move Folder Prompt Modal */}
      {movePrompt && (
        <div className="modal-overlay">
          <div className="modal-content" style={{ maxWidth: "440px" }}>
            <header className="modal-header">
              <h3><FolderOpen size={15} /> Move Game Files?</h3>
              <button className="modal-close" onClick={() => { setMovePrompt(null); }}>✕</button>
            </header>
            <div style={{ padding: "16px", fontSize: "14px", lineHeight: "1.5" }}>
              <p>Move all games and cache to the new location?</p>
              <p style={{ color: "var(--text-secondary)", fontSize: "13px", marginTop: "4px" }}>
                From: {movePrompt.oldFolder}<br />
                To: {movePrompt.newFolder}
              </p>
              <p style={{ color: "var(--text-secondary)", fontSize: "13px", marginTop: "8px" }}>
                Total size: {formatBytes(movePrompt.totalBytes)}
              </p>
            </div>
            <footer className="settings-actions">
              <button className="modal-btn modal-btn-cancel" onClick={() => handleMoveConfirm(false)}>Skip (start fresh)</button>
              <button className="modal-btn modal-btn-save" onClick={() => handleMoveConfirm(true)}>Move Everything</button>
            </footer>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
