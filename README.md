# NakamaLauncher

NakamaLauncher is a lightweight desktop client built on Tauri, Rust, and Vanilla HTML/CSS/JS. It provides a simple frontend for downloading game versions and modpack archives from [NakamaServer](https://github.com/OmarQurashi868/nakamaserver), styled with an elegant dark theme based on the Obsidian design language.

## Architecture

- **Frontend**: Single-page application using vanilla Javascript and custom CSS matching the Obsidian design scheme (`src/index.html`, `src/main.js`, `src/styles.css`).
- **Backend (Tauri/Rust)**: Bypasses standard browser security (such as CORS policies) by routing server requests and large file uploads directly through Rust commands (`reqwest` client).

## Getting Started

### Prerequisites

Ensure you have [Rust](https://www.rust-lang.org/) and [Node.js](https://nodejs.org/) installed.

### Installing Dependencies

```bash
npm install
```

### Running Locally (Development)

To run the application in development mode with hot-reloading:

```bash
npm run tauri dev
```

### Compiling to Binary (Production Build)

To build a standalone desktop application executable:

```bash
npm run tauri build
```

## Features

- **Obsidian Theme**: Polished dark dashboard interface, responsive split-pane layout, custom scrollbars, and toast notifications.
- **Connection Configuration**: Configure server IP/URL and the `DOWNLOAD_KEY` key via the top-right settings gear modal. Settings are persisted in local storage.
- **Automatic Catalog Sync**: Performs `/query` on load to construct a complete map of games, versions, and modpacks.
- **Dynamic Sidebar**: Renders games alphabetically with quick stats. Includes a filter search bar.
- **Fuzzy Search Dropdowns**: Autocompletes game titles when uploading new game versions or modpacks, searching dynamically from existing catalog games.