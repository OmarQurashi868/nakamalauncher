# NakamaLauncher — Domain Glossary

## Core Concepts

- **Game** — a playable title with multiple versions. Identified by `id` (sanitized name slug) and `name` (display title). Grouped from flat `/query` response by matching `title` strings.
- **Version** — a specific release of a Game. Has `uuid`, `version` string, `launch_path`, `size_bytes`, download URL derived from UUID (`/download/game/{uuid}`).
- **Modpack** — a set of mod files for a Game (game-wide, not version-specific). Has `uuid`, `modpack_title`, `size_bytes`, download URL (`/download/modpack/{uuid}`).
- **Permutation** — a (version, modpack) pair the user selects. The launcher makes this permutation live in the staging area.

## Storage Locations

- **Game Folder** — user-configured root directory (e.g. `C:/games`). Contains staging area and `.nakama` cache.
- **Cache** (`{gameFolder}/.nakama/`) — internal storage for all downloaded content, organized as:
  ```
  .nakama/
  ├── {sanitized-game-name}/
  │   ├── versions/
  │   │   └── {version}/
  │   │       ├── (game files, unzipped)
  │   │       └── .vanilla/          (originals replaced by any modpack)
  │   └── modpacks/
  │       └── {modpack_title}/
  │           ├── (mod files, unzipped)
  │           └── .manifest.json     (list of file paths this modpack deploys)
  └── _downloads/                    (partial .zip.tmp files)
  ```
- **Staging Area** (`{gameFolder}/{GameName}/`) — the single playable deployment. Contains the active version's files with the active modpack applied on top. Contains `.nakama-state` file.
- **State File** (`.nakama-state`) — JSON in staging folder recording what's deployed:
  ```json
  {"game_id": "...", "version": "...", "modpack": "..." | null, "swap_phase": "..." | null}
  ```
  `swap_phase` tracks in-progress swaps for crash recovery. `null` means consistent.
- **Vanilla Backup** (`.vanilla/` inside a version folder in cache) — copies of original game files that a modpack replaced. Used to restore the game to unmodded state when user selects `<none>` modpack.
- **Modpack Manifest** (`.manifest.json` inside a modpack cache folder) — list of file paths the modpack deploys. Used for eviction. Format: `{"modpack_title": "...", "files": ["path1", "path2"]}`.

## Operations

- **Download** — fetch zip from server to `_downloads/`, extract to cache. Always targets cache, never directly stages. Supports resume via `Range` header and partial `.zip.tmp` files.
- **Auto-Apply** — after download completes, if the downloaded permutation matches the user's currently selected permutation, automatically swap to make it live.
- **Apply (Swap)** — make a cached permutation the staged one. Steps in order:
  1. Evict current modpack from staging (restore vanilla, move modpack files to cache)
  2. Swap version (rename staging old version → cache, rename cache new version → staging)
  3. Apply new modpack to staging (backup overwritten files to `.vanilla/`, move modpack files to staging)
  4. Update `.nakama-state`
- **Restore Vanilla** — on modpack eviction, for each file path in the modpack manifest: delete from staging, then copy the backup from `.vanilla/` back to staging if one exists.
- **Delete Version** — remove version folder from cache. Blocked if version is currently staged.
- **Delete Modpack** — remove modpack folder from cache. Blocked if modpack is currently applied.
- **Delete Game** — remove entire game from cache and staging. Confirmation dialog warns about all files in staging folder.
- **Repair** — when `.nakama-state` has a non-null `swap_phase` on startup, offer a Repair button that re-applies the recorded permutation from cache (or re-downloads if cache missing).

## Button State Machine

Button label depends on what's selected vs cached vs staged:

| Version cached | Modpack cached | Modpack | Currently staged? | Button |
|---|---|---|---|---|
| Yes | Yes | any | No | **Apply** |
| Yes | Yes | same permutation | Yes | **Play** |
| No | No | any | — | **Download** |
| Yes | No | "qol mods" | — | **Download (modpack only)** |
| No | Yes | "qol mods" | — | **Download (version only)** |
| Yes | — | `<none>` | No | **Apply** |
| No | — | `<none>` | — | **Download** |

When any operation (download/swap) is active or queued for a game, Play is blocked for that game only.

## Concurrency

- Global download queue (FIFO). One active download at a time, 1s gap between completions.
- One operation per game at a time (download or swap). Queue per-game.
- Server enforces one download per IP; queue design matches this constraint.
- All moves are on the same filesystem volume (atomic `rename()`). Cross-volume moves only happen during "change game folder" migration (prompted).
- Crash recovery: best-effort. State file records phase. On interrupted swap, next launch shows Repair.

## UI

- Version dropdown + modpack dropdown (includes `<none>`). Changing selection does nothing until button clicked.
- Steam link under game name when `app_id` is present: `https://store.steampowered.com/app/{app_id}/`
- Notes box shown for entries that have `notes` or `title_notes`; hidden otherwise.
- Storage sizes computed once on startup via filesystem walk, updated on mutations (download/delete).
- Delete buttons: per-version, per-modpack, per-game (with size display and confirmation).
- Settings: change game folder prompts to move all content to new location.
