# Install pfx

Every tag's [GitHub Release](https://github.com/gmrdad82/pfx/releases) carries the `pfx` tool for three platforms, built from the signed tag with `bin/build-release` (Linux and Windows on Linux x86_64, macOS on a Mac with Apple Silicon) and checksummed with `bin/checksums`:

| Asset | Holds |
|---|---|
| `pfx-vX.Y.Z-linux-x86_64.tar.gz` | `pfx`, and `share/` with the desktop file, the MIME type and the icons |
| `pfx-vX.Y.Z-windows-x86_64.zip` | `pfx.com`, the console tool; `pfx.exe`, the window launcher; and `associate.ps1` for the file type and the Start menu |
| `pfx-vX.Y.Z-macos-arm64.tar.gz` | `pfx` for Apple Silicon |
| `pfx-vX.Y.Z-macos-arm64.app.zip` | `pfx.app`, the same binary in an app bundle that declares the file type and carries the icon |
| `docs-vX.Y.Z.tar.gz` | these docs |
| `SHA256SUMS` | the sha256 of every asset above, one `<hash>  <file>` line each |

Each archive holds its binaries at its root with `LICENSE`, `README.md`, `NOTICE.md`, `LICENSE-MT19937.md` and `LICENSE-ebur128.md`, the licence texts that ship with binary copies. A tag with a suffix (`v3.0.0-alpha.1`) is published as a pre-release. Check a download before you unpack it:

```
sha256sum --check --ignore-missing SHA256SUMS
```

On macOS the command is `shasum -a 256 --check --ignore-missing SHA256SUMS`; on Windows, `Get-FileHash <file>` prints the hash to compare.

The tool needs a GPU wgpu can open: Vulkan on Linux, Vulkan or DX12 on Windows, Metal on macOS. `pfx` builds from source on all three with Rust 1.97 or newer, `cargo install --locked --git https://github.com/gmrdad82/pfx --tag vX.Y.Z pfx`; on Debian or Ubuntu install `libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev libx11-dev` first.

## How pfx starts

On every system `pfx` with no arguments opens the window, `pfx <file.pitofx>` opens that file in it, and any subcommand or flag is the command line and never opens a window. `.pitofx` is pfx's file on the desktop, the `PFXFILE` container ([pfx-file.md](pfx-file.md)). Where the window runs:

- **Linux:** in the process that was asked.
- **macOS:** a bare `pfx` from a terminal opens `pfx.app` through Launch Services (`open -b com.pitomd.pfx`) and returns; inside the bundle, and with a file, the window runs in the process.
- **Windows:** `pfx.com` hands a bare call or a lone `.pitofx` to `pfx.exe` beside it, detached, and returns at once, so the terminal stays free. `pfx.exe` is a window-subsystem program: it starts `pfx.com gui <arguments>` with no console window and exits, which is what a double-click, the Start menu and the file type run.

## Linux

```
mkdir pfx && tar -xzf pfx-vX.Y.Z-linux-x86_64.tar.gz -C pfx
install -Dm755 pfx/pfx ~/.local/bin/pfx
cp -r pfx/share/. ~/.local/share/
update-mime-database ~/.local/share/mime
update-desktop-database ~/.local/share/applications
```

`share/` follows the freedesktop layout, so copying it into `~/.local/share` (or `/usr/local/share` for every user) installs:

- `applications/pfx.desktop`: pfx as an application that opens `application/x-pfx` with `pfx gui %f`;
- `mime/packages/pfx.xml`: the `application/x-pfx` type, matched by the `*.pitofx` glob and by the file's magic, `PFXFILE\0` at offset 0, so a pfx file is recognised whatever its name;
- `icons/hicolor/`: pfx's mark as the app icon `pfx` and the file icon `application-x-pfx`, scalable and 512 px.

`pfx` must be on `PATH` for the desktop file's `Exec`; `~/.local/bin` is on it in most desktops.

## Windows

In PowerShell, from the folder holding the zip:

```
Expand-Archive pfx-vX.Y.Z-windows-x86_64.zip -DestinationPath $env:LOCALAPPDATA\Programs\pfx
$user = [Environment]::GetEnvironmentVariable('Path', 'User')
[Environment]::SetEnvironmentVariable('Path', "$user;$env:LOCALAPPDATA\Programs\pfx", 'User')
powershell -ExecutionPolicy Bypass -File $env:LOCALAPPDATA\Programs\pfx\associate.ps1
```

Typing `pfx` in a terminal runs `pfx.com`, because Windows tries `.COM` before `.EXE`: the command line with its full output and exit codes. Both binaries must stay in the same folder.

`associate.ps1` registers pfx for the current user only, under `HKCU\Software\Classes`, and needs no administrator:

- the ProgID `pfx.document`, whose `shell\open\command` is `"<folder>\pfx.exe" "%1"` and whose `DefaultIcon` is `pfx.exe`'s own icon;
- `.pitofx`, whose default is `pfx.document`, with `Content Type` `application/x-pfx` and `pfx.document` under `OpenWithProgids`;
- `Applications\pfx.exe`, with `.pitofx` in its `SupportedTypes`;
- a Start menu entry, `pfx.lnk` in the user's Programs folder, pointing at `pfx.exe`.

`associate.ps1 -Remove` takes all of it out again. Run it again after moving the folder.

## macOS

The command-line tool:

```
tar -xzf pfx-vX.Y.Z-macos-arm64.tar.gz
sudo install -m755 pfx /usr/local/bin/pfx
```

The app: unzip `pfx-vX.Y.Z-macos-arm64.app.zip` and move `pfx.app` to Applications. Its `Info.plist` exports `com.pitomd.pfx.document`, pfx's own document type (the `pitofx` extension and `application/x-pfx`, conforming to `public.data` and `public.content`, with `pfx.icns` as its icon), and lists it as a document type pfx edits, with pfx as its owner; `CFBundleIconFile` is `pfx`, the bundle's `pfx.icns`. The release builds are not signed or notarized, so Gatekeeper asks before the first launch; `xattr -dr com.apple.quarantine /Applications/pfx.app` clears that for a download you have checked.

Finder hands a document to a running app as an Apple Event, not as an argument, and the window does not read that event: a double-clicked `.pitofx` starts the window on its start scene, and the file is opened from its File menu. `pfx <file.pitofx>` from a terminal opens it directly.

## What each platform runs

`bin/gate` builds the whole workspace and runs its unit tests on each system (phases `tests` on Linux, `windows` and `macos`), with clippy on each (`lint`, `windows-lint`, `macos-lint`). GPU tests are ignored in those phases, so the Windows and macOS GPU paths are built but untested on hardware. pgpu, the GPU queue, is Linux only: when `pgpu` is not on `PATH`, which is always the case on Windows and macOS, every render, bake and bench runs directly. Ending a render's children on a stop signal uses SIGTERM on Linux and macOS and ends the child process on Windows; finding the processes below a queue wrapper reads `/proc` and is Linux only, as the queue is.
