<p align="center">
  <img src="docs/images/icon.png" width="128" alt="Colima GUI icon">
</p>

<h1 align="center">Colima GUI</h1>

<p align="center">
  A small, fast, native-feeling macOS window for <a href="https://github.com/abiosoft/colima">Colima</a> —
  see and manage your containers without bringing Docker Desktop back.
</p>

![Containers](docs/images/containers.png)

## Features

- **Colima status at a glance** — running/stopped, CPU, RAM, disk; start and stop the VM.
- **Containers** — grouped by Compose project, live CPU / memory, published ports that open in your browser,
  start / stop / restart / delete, and a one-click shell in Terminal.
- **Container detail** — streaming logs with filter, follow and copy; CPU and memory sparklines.
- **Images & volumes** — sizes, age, "in use" markers, delete, and one-click clean-up of dangling images / unused volumes.
- **Menu bar icon** — the app keeps running in the menu bar when you close the window.
- **Guided setup** — if Colima isn't installed yet, the app shows what's missing and installs it with Homebrew.
- **Light** — a single Rust binary (egui), no WebView, no background daemon of its own.

![Container detail](docs/images/detail.png)

## Install

1. Download `Colima-GUI-<version>-macos-universal.zip` from the
   [latest release](https://github.com/cuong95nddev/colima-gui/releases/latest) (Apple Silicon and Intel).
2. Unzip it and move **Colima GUI.app** to `/Applications`.
3. The app is not notarized by Apple, so the first launch is blocked by Gatekeeper. Either
   right-click the app → **Open**, or run:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Colima GUI.app"
   ```

Requires macOS 11 or later. Colima itself is installed with `brew install colima docker`
(the app offers to do this for you on first launch).

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| <kbd>⌘</kbd> <kbd>1</kbd> / <kbd>2</kbd> / <kbd>3</kbd> | Containers / Images / Volumes |
| <kbd>⌘</kbd> <kbd>F</kbd> | Search |
| <kbd>⌘</kbd> <kbd>R</kbd> | Refresh |
| <kbd>⌘</kbd> <kbd>W</kbd> | Hide window (keeps running in the menu bar) |
| <kbd>Esc</kbd> | Back / clear search |

## Build from source

```sh
cargo run --release          # run it
scripts/install.sh           # build "Colima GUI.app" and copy it to /Applications
UNIVERSAL=1 scripts/bundle.sh  # universal (arm64 + x86_64) bundle in target/bundle/
```

The app talks to Docker through `~/.colima/default/docker.sock` (or `DOCKER_HOST`) and to Colima through its CLI.

## Releasing

Push a tag like `v0.1.0`; the [Release workflow](.github/workflows/release.yml) builds the universal app,
zips it and publishes a GitHub release.
