# rusty-sts

A lightweight desktop app for syncing Slay the Spire 2 run data to the [STS2 Stats Tracker](https://github.com/JiriPlasek/sts).

## What it does

- Auto-detects your STS2 save folder
- Uploads `.run` files to the stats tracker with one click
- Skips already-uploaded runs automatically
- Remembers your settings between launches

## Setup

### Windows

1. Download `rusty-sts.exe` from [Releases](https://github.com/JiriPlasek/rusty_sts/releases)
2. Run it — no installation needed

### Linux

Build from source (requires Rust and the GTK3 + appindicator libraries, e.g.
`gtk3` and `libayatana-appindicator` on Arch, `libgtk-3-dev` and
`libayatana-appindicator3-dev` on Debian/Ubuntu):

```sh
STS_API_URL="https://ststracker.app/" cargo build --release
./target/release/rusty-sts
```

Both the native Linux version of the game and the Windows version running
under Proton are detected automatically.

### First run

1. Go to the stats tracker website → Settings → Generate a Sync Token
2. Paste the token into the app
3. Confirm the auto-detected save folder (or browse to select it manually)
4. Click **Save & Continue**

## Usage

Open the app and click **Sync**. That's it. New runs are uploaded, duplicates are skipped.
