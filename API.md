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

Rate limits use per-IP token buckets. Separate limits apply depending on the key used:

| Endpoints | Rate | Burst | Retry-After |
|-----------|------|-------|--------------|
| `/query`, `/download/...` (download key) | 10 req/min | 10 | 6s |
| `/admin/...` (admin key) | 120 req/min | 60 | 1s |

When the limit is exceeded:
```
HTTP 429 Too Many Requests
Retry-After: <seconds>
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
      "uuid": "a1b2c3d4-e5f6-4789-abcd-ef0123456789",
      "title": "My Game",
      "version": "1.0",
      "file_name": "My_Game_1.0.zip",
      "file_size_bytes": 1073741824,
      "launch_exe": "game.exe",
      "app_id": "",
      "notes": "",
      "title_notes": "",
      "uploaded_at": "2026-06-28T01:00:00Z",
      "downloads": 42
    }
  ],
  "modpacks": [
    {
      "id": 1,
      "uuid": "b2c3d4e5-f6a7-4890-bcde-f01234567890",
      "game_title": "My Game",
      "modpack_title": "Cool Mod",
      "file_name": "My_Game_Cool_Mod.zip",
      "file_size_bytes": 52428800,
      "notes": "",
      "uploaded_at": "2026-06-28T02:00:00Z",
      "downloads": 7
    }
  ]
}
```

---

### `GET /download/game/{uuid}`

Streams a game zip file identified by its UUID. Only **one active download per IP** is allowed at a time.

**Request**
```
GET /download/game/a1b2c3d4-e5f6-4789-abcd-ef0123456789
X-API-Key: <download-key>
```

| Parameter | Location | Description |
|-----------|----------|-------------|
| `uuid`    | URL path | Game UUID (returned at upload, listed in `/query`) |

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
| `400 Bad Request` | Missing or malformed UUID in path |
| `404 Not Found` | Game not found or file missing on disk |
| `429 Too Many Requests` | Another download is already active from this IP |

> Only one download can be active per IP at a time. Wait for the current download to finish before starting another.

---

### `GET /download/modpack/{uuid}`

Streams a modpack zip file identified by its UUID. Only **one active download per IP** is allowed at a time.

**Request**
```
GET /download/modpack/b2c3d4e5-f6a7-4890-bcde-f01234567890
X-API-Key: <download-key>
```

| Parameter | Location | Description |
|-----------|----------|-------------|
| `uuid`    | URL path | Modpack UUID (returned at upload, listed in `/query`) |

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
| `400 Bad Request` | Missing or malformed UUID in path |
| `404 Not Found` | Modpack not found or file missing on disk |
| `429 Too Many Requests` | Another download is already active from this IP |

---

## Admin Endpoints

These endpoints use the **admin key**.

---

### `POST /admin/upload/game`

Uploads a game zip and registers it in the catalog. Returns the assigned UUID.

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
| `app_id` | string | No | External app/store ID (per-title — syncs across all versions) |
| `notes` | string | No | Free-form notes for this specific version |
| `title_notes` | string | No | Game-wide notes (per-title — syncs across all versions) |
| `file` | file | Yes | The zip archive to upload |

**Example (curl)**
```bash
curl -X POST http://localhost:8080/admin/upload/game \
  -H "X-API-Key: <admin-key>" \
  -F "title=My Game" \
  -F "version=1.0" \
  -F "launch_exe=game.exe" \
  -F "app_id=123456" \
  -F "file=@/path/to/game.zip"
```

**Response `200 OK`**
```json
{"ok": true, "uuid": "a1b2c3d4-e5f6-4789-abcd-ef0123456789", "file": "My_Game_1.0.zip", "size_bytes": 1073741824}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing required fields or malformed form data |
| `409 Conflict` | A game with this title + version already exists |
| `500 Internal Server Error` | Disk or database error |

> The stored filename is derived from `title` and `version` with unsafe characters replaced by underscores (e.g. `My Game` + `1.0` → `My_Game_1.0.zip`). The `title` + `version` pair must be unique. A UUID is assigned at upload and returned — save this UUID for future download, delete, and patch operations. **app_id and title_notes are per-title:** setting either on any version automatically syncs it to all other versions of the same game. **notes is per-version** — each version keeps its own notes.

---

### `POST /admin/upload/modpack`

Uploads a modpack zip and registers it in the catalog. Returns the assigned UUID.

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
| `notes` | string | No | Free-form notes about the modpack |
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
{"ok": true, "uuid": "b2c3d4e5-f6a7-4890-bcde-f01234567890", "file": "My_Game_Cool_Mod.zip", "size_bytes": 52428800}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing required fields or malformed form data |
| `409 Conflict` | A modpack with this game_title + modpack_title already exists |
| `500 Internal Server Error` | Disk or database error |

---

### `DELETE /admin/game/{uuid}`

Removes a game from the catalog and deletes its file from disk.

**Request**
```
DELETE /admin/game/a1b2c3d4-e5f6-4789-abcd-ef0123456789
X-API-Key: <admin-key>
```

| Parameter | Location | Description |
|-----------|----------|-------------|
| `uuid`    | URL path | Game UUID |

**Example (curl)**
```bash
curl -X DELETE "http://localhost:8080/admin/game/a1b2c3d4-e5f6-4789-abcd-ef0123456789" \
  -H "X-API-Key: <admin-key>"
```

**Response `200 OK`**
```json
{"ok": true}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing or malformed UUID in path |
| `404 Not Found` | Game not found |
| `500 Internal Server Error` | Database error |

> Deletion is permanent. The zip is removed from disk and the catalog entry is dropped. This cannot be undone.

---

### `DELETE /admin/modpack/{uuid}`

Removes a modpack from the catalog and deletes its file from disk.

**Request**
```
DELETE /admin/modpack/b2c3d4e5-f6a7-4890-bcde-f01234567890
X-API-Key: <admin-key>
```

| Parameter | Location | Description |
|-----------|----------|-------------|
| `uuid`    | URL path | Modpack UUID |

**Example (curl)**
```bash
curl -X DELETE "http://localhost:8080/admin/modpack/b2c3d4e5-f6a7-4890-bcde-f01234567890" \
  -H "X-API-Key: <admin-key>"
```

**Response `200 OK`**
```json
{"ok": true}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Missing or malformed UUID in path |
| `404 Not Found` | Modpack not found |
| `500 Internal Server Error` | Database error |

> Deletion is permanent. The zip is removed from disk and the catalog entry is dropped. This cannot be undone.

---

### `PATCH /admin/game/{uuid}`

Updates properties of an existing game. All fields are optional — only the provided fields are modified.

**Request**
```
PATCH /admin/game/a1b2c3d4-e5f6-4789-abcd-ef0123456789
X-API-Key: <admin-key>
Content-Type: application/json
```

```json
{"title": "New Title", "version": "2.0", "app_id": "999999", "notes": "Updated notes", "title_notes": "Game-wide note", "launch_exe": "new_launcher.exe"}
```

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | New display name for the game |
| `version` | string | New version string |
| `app_id` | string | External app/store ID (per-title — syncs across all versions) |
| `notes` | string | Free-form notes for this specific version |
| `title_notes` | string | Game-wide notes (per-title — syncs across all versions) |
| `launch_exe` | string | Relative path to the executable inside the zip |

All fields are optional. Only include the fields you want to change.

**Example (curl)**
```bash
curl -X PATCH "http://localhost:8080/admin/game/a1b2c3d4-e5f6-4789-abcd-ef0123456789" \
  -H "X-API-Key: <admin-key>" \
  -H "Content-Type: application/json" \
  -d '{"app_id": "999999"}'
```

**Response `200 OK`**
```json
{"ok": true}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Invalid JSON body, no fields provided, or unknown field name |
| `404 Not Found` | Game not found |
| `500 Internal Server Error` | Database error |

> Changing `title` or `version` does **not** rename the file on disk. The `file_name` field reflects the original upload name.

---

### `PATCH /admin/modpack/{uuid}`

Updates properties of an existing modpack. All fields are optional — only the provided fields are modified.

**Request**
```
PATCH /admin/modpack/b2c3d4e5-f6a7-4890-bcde-f01234567890
X-API-Key: <admin-key>
Content-Type: application/json
```

```json
{"game_title": "New Game", "modpack_title": "Updated Mod Name", "notes": "Updated notes"}
```

| Field | Type | Description |
|-------|------|-------------|
| `game_title` | string | New game title this modpack belongs to |
| `modpack_title` | string | New display name for the modpack |
| `notes` | string | Free-form notes about the modpack |

All fields are optional. Only include the fields you want to change.

**Example (curl)**
```bash
curl -X PATCH "http://localhost:8080/admin/modpack/b2c3d4e5-f6a7-4890-bcde-f01234567890" \
  -H "X-API-Key: <admin-key>" \
  -H "Content-Type: application/json" \
  -d '{"modpack_title": "New Mod Name"}'
```

**Response `200 OK`**
```json
{"ok": true}
```

**Error responses**

| Status | Condition |
|--------|-----------|
| `400 Bad Request` | Invalid JSON body, no fields provided, or unknown field name |
| `404 Not Found` | Modpack not found |
| `500 Internal Server Error` | Database error |

> Changing `game_title` or `modpack_title` does **not** rename the file on disk.

---

### `GET /admin/disk-quota`

Returns the total and used bytes of the filesystem that contains the games directory.

**Request**
```
GET /admin/disk-quota
X-API-Key: <admin-key>
```

**Example (curl)**
```bash
curl -X GET http://localhost:8080/admin/disk-quota \
  -H "X-API-Key: <admin-key>"
```

**Response `200 OK`**
```json
{"total_bytes": 107374182400, "used_bytes": 52648001536}
```

| Field | Type | Description |
|-------|------|-------------|
| `total_bytes` | int64 | Total size of the server disk in bytes |
| `used_bytes` | int64 | Used space across the entire server disk in bytes |

> `free_bytes` can be derived as `total_bytes - used_bytes`.

**Error responses**

| Status | Condition |
|--------|-----------|
| `500 Internal Server Error` | Unable to query filesystem stats |

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
