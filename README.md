# les – Linux Everything-style Instant Search

`les` (Linux Everything Search) is a fast, lightweight file name search engine for Linux,
inspired by the famous **Everything** app on Windows.

It provides:

- A background **daemon** (`lesd`) that maintains a persistent index.
- A **client CLI** (`les`) to query the index instantly.
- Optional **interactive mode** for quick, repeated searches.
- Optional **content search** for simple string matches inside files.

> ⚠️ This is a work in progress / prototype. Use at your own risk.

---

## Features

1. **Persistent on-disk index**  
   - Uses an embedded key-value store (`sled`) to store file metadata.
   - On startup, the daemon loads the index into memory for fast queries.

2. **Background daemon (`lesd`)**  
   - Keeps the index in memory.
   - Watches the filesystem for changes and updates entries.
   - Exposes a simple JSON-over-Unix-socket API.

3. **Client CLI (`les`)**  
   - Connects to `lesd` over a Unix domain socket.
   - Sends search queries and prints results in a tabular format.

4. **Interactive mode**  
   - Run `les -i` to get a simple prompt:
     - Type a pattern, press Enter, see results.
     - Empty line exits.

5. **Advanced filters**  
   - Filter by:
     - type: `--files-only`, `--dirs-only`
     - size: `--min-size`, `--max-size` (bytes)
     - time: `--min-mtime`, `--max-mtime` (UNIX seconds)
     - roots: `--roots /home /mnt/data`
     - excludes: `--exclude ".git" --exclude "node_modules"`

6. **Optional content search**  
   - `--content "some string"` will perform a slower search that also checks file content.
   - Intended only for small/medium code bases; for big content search, use ripgrep/rg.

7. **Packaging-friendly**  
   - Single static-ish binaries (`lesd`, `les`).
   - Easy to package as `.deb`/`.rpm` or AppImage (see notes below).

8. **Systemd integration**  
   - Example unit file included to run `lesd` as a system service:
     - Starts on boot.
     - Keeps Unix socket at `/run/lesd.sock`.

---

## Architecture

### Components

- **`les_core` (library crate)**  
  - Common data structures and logic:
    - `FileEntry` – path, type, size, mtime.
    - `Query` – pattern, filters, content search.
    - `Index` – in-memory entries + sled-backed DB.
  - Responsibilities:
    - Initial full scan.
    - Incremental updates of paths.
    - Matching queries with glob / regex / substring.

- **`lesd` (daemon)**  
  - Opens/creates the index DB at `/var/lib/les/index.db` by default.
  - On first run or on `--rebuild`:
    - Recursively scans configured roots.
    - Writes file metadata into the DB.
  - After startup:
    - Watches filesystem roots using `notify`.
    - Listens on Unix socket `/run/lesd.sock`.
    - Accepts JSON requests and returns JSON responses.

- **`les` (client)**  
  - Lightweight CLI that:
    - Constructs a `Query` from CLI arguments.
    - Sends `Request::Query` to `lesd`.
    - Prints `QueryResult` as lines:
      - `<type>\t<size>\t<mtime>\t<path>`
      - `type` is `d` for directories and `-` for files.

---

## Building

Requirements:

- Rust toolchain (stable) with `cargo`
- Linux host with inotify (the watcher backend)

```bash
git clone https://github.com/<your-username>/lesearch.git
cd lesearch

# Fast dev build
cargo build

# Optimized binaries
cargo build --release
```

Artifacts land in `target/{debug,release}/les` and `target/{debug,release}/lesd`.

---

## Running

1. **Start the daemon**

   ```bash
   target/release/lesd \
     --roots /home/you \
     --db-path /tmp/les-index.db \
     --socket /tmp/lesd.sock \
     --rebuild   # only on first start or when forcing a rescan
   ```

   - `--roots` lists directories to index (defaults to `/`).
   - `--exclude` accepts substrings to skip (repeat the flag).
   - The daemon keeps the index in memory, watches the filesystem, and listens on the supplied Unix socket.

2. **Run the client**

   ```bash
   target/release/les --socket /tmp/lesd.sock PATTERN
   ```

   - `PATTERN` is mandatory in non-interactive mode.
   - Options:
     - `--mode substr|glob|regex`
     - `--files-only` / `--dirs-only`
     - `--min-size 1024` / `--max-size 1048576`
     - `--min-mtime 1690000000`
     - `--roots /home/you` (each value requires its own argument)
     - `--exclude ".git"`
     - `--content "needle"`
   - To supply a pattern after `--roots`, use `--` to end option parsing:
     ```bash
     target/release/les --socket /tmp/lesd.sock --roots /home/you -- documents
     ```

3. **Interactive mode**

   ```bash
   target/release/les --socket /tmp/lesd.sock --interactive
   ```

   This opens a simple prompt (`les>`) that keeps issuing queries until you enter a blank line.

4. **Shutdown**

   Press `Ctrl+C` in the daemon process. It removes the socket before exiting.

---

## Packaging

Two helper scripts live under `packaging/` and produce distributable artifacts from the release binaries.

### Debian package

Requirements: `dpkg-deb`, `python3`, and a Linux host that matches your target architecture.

```bash
./packaging/deb/build_deb.sh
```

The script:

- Builds release binaries.
- Creates `/usr/bin/les` and `/usr/bin/lesd` entries.
- Installs a systemd unit at `/usr/lib/systemd/system/lesd.service`.
- Emits `target/package/lesearch_<version>_<arch>.deb`.

Install with `sudo dpkg -i target/package/lesearch_<version>_<arch>.deb`.
`dpkg-deb` may warn about rootless builds; pass `--root-owner-group` if you need strict ownership metadata.

### AppImage

Requirements: `appimagetool` in `PATH` (download from <https://github.com/AppImage/AppImageKit/releases>), `python3`, Linux host.

```bash
./packaging/appimage/build_appimage.sh
```

- Bundles both `les` and `lesd`.
- `AppRun` dispatches to `les` by default; call with `lesd` as the first argument to start the daemon inside the AppImage.
- Produces `target/package/appimage/Lesearch-<version>-<arch>.AppImage`.

---

## Development & Testing

- `cargo fmt` / `cargo clippy -- -D warnings`
- `cargo test` (runs the small integration tests and builds everything)
- Log output is written to stderr; run binaries with `RUST_LOG=debug` to surface verbose sled/notify info.

---

## Roadmap / Ideas

- Richer query language (AND/OR, glob+regex combos).
- Proper TUI front-end.
- Windows/OSX support (needs cross-platform watcher backend).
- Smarter content search (ripgrep/rayon integration).
