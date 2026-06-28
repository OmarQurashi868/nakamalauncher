# NakamaServer API Reference

NakamaServer is an HTTP file-distribution server for games and modpacks.
All endpoints require an API key passed via the `X-API-Key` header.

---

## Authentication

| Key | Header | Who uses it |
|-----|--------|-------------|
| `ADMIN_KEY` | `X-API-Key: <admin-key>` | Admin endpoints (`/admin/...`) and `/query` |
| `DOWNLOAD_KEY` | `X-API-Key: <download-key>` | `/query` and `/download/...` |

Requests with a missing or wrong key receive:
```
HTTP 401 Unauthorized
{"error":"unauthorized"}
```

---

## Rate Limiting

All endpoints share a **token-bucket rate limit of 10 requests per minute per IP** (burst of 10).
When the limit is exceeded:
```
HTTP 429 Too Many Requests
Retry-After: 6
{"error":"rate limit exceeded"}
```

---

## User Endpoints

These endpoints use the **download key**.

---

### `GET /query`

Returns the full catalog of all available games and modpacks.
Accepts **either** the admin key or the download key.

**Request**
```
GET /query
X-API-Key: <admin-key or download-key>
```

**Response `200 OK`**
```json
{
  "games": [
    {
      "id": 1,
      "title": "My Game",
      "version": "1.0",
      "file_name": "My_Game_1.0.zip",
      "file_size_bytes": 1073741824,
      "launch_exe": "game.exe",
      "uploaded_at": "2026-06-28T01:00:00Z"
    }
  ],
  "modpacks": [
    {
      "id": 1,
      "game_title": "My Game",
      "modpack_title": "Cool Mod",
      "file_name": "My_Game_Cool_Mod.zip",
      "file_size_bytes": 52428800,
      "uploaded_at": "2026-06-28T02:00:00Z"
    }
  ]
}
```

---

### `GET /download/game/{title}/{version}`

Streams a game zip file. Only **one active download per IP** is allowed at a time.

**Request**
```
GET /download/game/My%20Game/1.0
X-API-Key: <download-key>
```

| Parameter | Location | Description |
|-----------|----------|-------------|
| `title`   | URL path | Game title (URL-encoded if it contains spaces) |
| `version` | URL path | Game version string |

**Response `200 OK`**
```
Content-Type: application/zip
Content-Disposition: attachment; filename="My_Game_1.0.zip"
Content-Length: 1073741824

<binary zip data>
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing title or version in path |
| `404 Not Found` | Game not found in catalog or file missing on disk |
| `429 Too Many Requests` | Another download is already active from this IP |

> Only one download can be active per IP at a time. Wait for the current download to finish before starting another.

---

### `GET /download/modpack/{gameTitle}/{modpackTitle}`

Streams a modpack zip file. Only **one active download per IP** is allowed at a time.

**Request**
```
GET /download/modpack/My%20Game/Cool%20Mod
X-API-Key: <download-key>
```

| Parameter | Location | Description |
|-----------|----------|-------------|
| `gameTitle`    | URL path | Title of the game this modpack belongs to |
| `modpackTitle` | URL path | Title of the modpack |

**Response `200 OK`**
```
Content-Type: application/zip
Content-Disposition: attachment; filename="My_Game_Cool_Mod.zip"
Content-Length: 52428800

<binary zip data>
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing gameTitle or modpackTitle in path |
| `404 Not Found` | Modpack not found in catalog or file missing on disk |
| `429 Too Many Requests` | Another download is already active from this IP |

---

## Admin Endpoints

These endpoints use the **admin key**.

---

### `POST /admin/upload/game`

Uploads a game zip and registers it in the catalog.

**Request**
```
POST /admin/upload/game
X-API-Key: <admin-key>
Content-Type: multipart/form-data
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `title` | string | Yes | Display name of the game |
| `version` | string | Yes | Version string (e.g. `1.0`, `2.1.3`) |
| `launch_exe` | string | Yes | Relative path to the executable inside the zip (e.g. `game.exe`) |
| `file` | file | Yes | The zip archive to upload |

**Example (curl)**
```bash
curl -X POST http://localhost:8080/admin/upload/game \
  -H "X-API-Key: <admin-key>" \
  -F "title=My Game" \
  -F "version=1.0" \
  -F "launch_exe=game.exe" \
  -F "file=@/path/to/game.zip"
```

**Response `200 OK`**
```json
{"ok": true, "file": "My_Game_1.0.zip", "size_bytes": 1073741824}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing required fields or malformed form data |
| `409 Conflict` | A game with this title + version already exists |
| `500 Internal Server Error` | Disk or database error |

> The stored filename is derived from `title` and `version` with unsafe characters replaced by underscores (e.g. `My Game` + `1.0` → `My_Game_1.0.zip`). The `title` + `version` pair must be unique.

---

### `POST /admin/upload/modpack`

Uploads a modpack zip and registers it in the catalog.

**Request**
```
POST /admin/upload/modpack
X-API-Key: <admin-key>
Content-Type: multipart/form-data
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `game_title` | string | Yes | Title of the game this modpack is for |
| `modpack_title` | string | Yes | Display name of the modpack |
| `file` | file | Yes | The zip archive to upload |

**Example (curl)**
```bash
curl -X POST http://localhost:8080/admin/upload/modpack \
  -H "X-API-Key: <admin-key>" \
  -F "game_title=My Game" \
  -F "modpack_title=Cool Mod" \
  -F "file=@/path/to/modpack.zip"
```

**Response `200 OK`**
```json
{"ok": true, "file": "My_Game_Cool_Mod.zip", "size_bytes": 52428800}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing required fields or malformed form data |
| `409 Conflict` | A modpack with this game_title + modpack_title already exists |
| `500 Internal Server Error` | Disk or database error |

---

### `DELETE /admin/game/{title}/{version}`

Removes a game from the catalog and deletes its file from disk.

**Request**
```
DELETE /admin/game/My%20Game/1.0
X-API-Key: <admin-key>
```

| Parameter | Location | Description |
|-----------|----------|-------------|
| `title`   | URL path | Game title |
| `version` | URL path | Game version |

**Example (curl)**
```bash
curl -X DELETE "http://localhost:8080/admin/game/My%20Game/1.0" \
  -H "X-API-Key: <admin-key>"
```

**Response `200 OK`**
```json
{"ok": true}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing title or version in path |
| `404 Not Found` | Game not found in catalog |
| `500 Internal Server Error` | Database error |

> Deletion is permanent. The zip is removed from disk and the catalog entry is dropped. This cannot be undone.

---

### `DELETE /admin/modpack/{gameTitle}/{modpackTitle}`

Removes a modpack from the catalog and deletes its file from disk.

**Request**
```
DELETE /admin/modpack/My%20Game/Cool%20Mod
X-API-Key: <admin-key>
```

| Parameter | Location | Description |
|-----------|----------|-------------|
| `gameTitle`    | URL path | Game title |
| `modpackTitle` | URL path | Modpack title |

**Example (curl)**
```bash
curl -X DELETE "http://localhost:8080/admin/modpack/My%20Game/Cool%20Mod" \
  -H "X-API-Key: <admin-key>"
```

**Response `200 OK`**
```json
{"ok": true}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing gameTitle or modpackTitle in path |
| `404 Not Found` | Modpack not found in catalog |
| `500 Internal Server Error` | Database error |

> Deletion is permanent. The zip is removed from disk and the catalog entry is dropped. This cannot be undone.

---

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `ADMIN_KEY` | Yes | — | Secret key for admin endpoints |
| `DOWNLOAD_KEY` | Yes | — | Secret key for user endpoints |
| `PORT` | No | `8080` | Port the server listens on |
| `GAMES_DIR` | No | `/data/nakama/games` | Directory for game files and DB |
| `MODPACKS_DIR` | No | `/data/nakama/modpacks` | Directory for modpack files and DB |
| `MAX_UPLOAD_BYTES` | No | `10737418240` (10 GB) | Maximum allowed upload size in bytes |
