# LanTube

[简体中文](README.zh.md)

I have always had such troubles：

> Videos downloaded on the Internet（cannot be posted to public platforms due to copyright issues）,
> need to be played on computer and mobile phone (tablet),
> But don’t want to copy the video to each device, so there is this project.

---

This is a LAN video server, which can be deployed on any device (with operating system) in the LAN (even Raspberry Pi)

* Computer（Windows, Linux, Mac）
* MobilePhone or tablet
* RaspberryPie

---

# Features

* Navigate shared folders with a directory tree, breadcrumbs and search within the current folder.
* Browse video frames in the library and the play queue.
* Play H.264 videos without re-encoding, or transcode to compatible 1080p, 720p, or 480p.
* Configure a player cache budget in MB. The grey-white seek-bar range shows media buffered on the playback device, and quality changes preload before switching.
* Optionally cache compatible segments in a bounded LRU directory on disk.
* Upload files, create folders and switch between English and Simplified Chinese.
* Play the queue automatically or manually, with a sleep timer.

![LanTube demo showing folder navigation, video thumbnails, the play queue, and the sleep timer](images/screenshot.gif)

# Build

Install a current stable Rust toolchain, a linker for your target platform, and Node.js 24.11 or newer with npm. Run this command from the repository root. [Rust](https://www.rust-lang.org/) · [Node.js](https://nodejs.org/)

```sh
cargo build --release
```

Cargo installs the locked frontend dependencies, builds the UI and embeds it in `target/release/lantube` (`target/release/lantube.exe` on Windows). Copy that executable to a machine with a compatible OS and architecture. Node.js, npm and the frontend files are only needed when building from source.

## Run

The Windows executable requires Windows 10 or later on desktop systems, or Windows Server 2016 or later on servers.

Start `lantube` (or `lantube.exe`) and open [http://127.0.0.1:7777](http://127.0.0.1:7777). With the default settings, LanTube listens on localhost and uses a `share` directory next to the executable if one exists. Create the directory and restart LanTube to use it.

To enable access from other devices on a trusted LAN or choose other media directories, use [config.example.toml](config.example.toml) as a template for `config.toml`. LanTube looks in the current working directory first, then beside the executable. Relative media paths are resolved from the configuration file's directory. The example listens on `0.0.0.0:7777`.

Compatible playback and server-generated thumbnails require `ffmpeg` and `ffprobe` on the server's `PATH`, from FFmpeg 5.1 or later. Put the tools in an absolute `PATH` directory outside your media folders before starting LanTube. Hardware H.264 encoders (`h264_v4l2m2m` or `h264_omx`) are detected and test-encoded at startup with a timeout; LanTube falls back to `libx264` when neither works. Set `LANTUBE_H264_ENCODER` to `libx264`, `v4l2`, or `omx` to override detection and force a specific encoder. The server reads the source files as-is.

Use the quality menu below the player to choose the original stream or a compatible 1080p, 720p, or 480p stream. Original playback remuxes H.264 video without re-encoding and falls back to compatible 720p when a file cannot be remuxed. Open **Settings** to choose the default quality and a player cache budget in MB. Buffering is handled by hls.js on the playback device; the server only keeps the segments currently being generated or read. The grey-white seek-bar range shows what is already buffered. Changes apply when playback next starts or the quality changes.

Use **Show other files** in a folder to open media with an unrecognized or missing extension. Those entries are opened individually and stay out of automatic playback.

Compatible segments can optionally be cached on disk. Add a `[cache]` section to `config.toml`:

```toml
[cache]
path = "/mnt/usb/lantube-cache"
max_size_mb = 2048
cache_original_remux = false
```

The cache uses an LRU size limit, may be placed on a removable USB disk, and must not overlap a media directory. On startup LanTube removes incomplete temporary files and evicts the oldest segments to fit the limit. Original remux segments are streamed without writing to disk unless `cache_original_remux = true`. If the cache directory cannot be created or resolved, caching is disabled and playback continues normally.

Choose **Split here** or press `C` while the player has focus to add a chapter at the current position. Open **Chapters** to jump to a chapter, or use the pencil button to rename it, adjust its start time, or merge it with the previous chapter. The server stores chapters in `data/chapters/` under its working directory. All devices share the same chapters, and the media files stay intact. With authentication enabled, editors can change chapters and viewers can open and navigate them.

Saving chapters with cuts requires `ffprobe` on the server to verify the duration of the playback tracks. Changing the media content invalidates its previous chapters. Media folders must be separate from chapter storage.

---
如果您对股票，期货，外汇等金融交易感兴趣

欢迎加入`交易奇才 Slack 交流群`，让我们一起成为更好的交易者！

![](images/qrcode.jpeg)

> 可扫码关注公众号，并回复：**加群**
