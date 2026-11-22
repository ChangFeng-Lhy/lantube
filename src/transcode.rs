mod access;
mod audio;
mod cache;
mod plan;
mod timeline;
mod tools;

pub use access::{MediaAccess, ReadLease};
pub use cache::TranscodeCache;
pub use tools::MediaTools;

use crate::{
    AppState,
    error::AppError,
    library::{check_revision, file_revision, require_revision, resolve_file},
};
use audio::{add_audio_input, audio_slice};
use axum::{
    body::Body,
    extract::{Path as RoutePath, Query, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::stream;
use plan::{PlaybackPlan, Profile, sanitize_session};
use serde::Deserialize;
use std::{
    collections::HashMap,
    convert::Infallible,
    fmt::Write,
    future::Future,
    path::Path,
    sync::{Arc, LazyLock, Mutex, Weak},
    time::Duration,
};
use timeline::{Segment, chapter_duration, playback_timeline, thumbnail_timing};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use tools::{H264Encoder, ffmpeg, ffmpeg_command, ffmpeg_copy, run, run_with_audio};

const FRAME_RATE: u32 = 30;
const SEGMENT_SECONDS: u32 = 4;
const MIN_TAIL_SECONDS: f64 = 0.5;
const MAX_SEGMENTS: u32 = 100_000;
const AUDIO_FILTER: &str = "aresample=async=1:first_pts=0,apad";

struct PlaybackSlots {
    global: Semaphore,
    sessions: Mutex<HashMap<String, Weak<Semaphore>>>,
}

impl PlaybackSlots {
    fn new() -> Self {
        Self {
            global: Semaphore::const_new(2),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn session(&self, id: &str) -> Arc<Semaphore> {
        let mut sessions = self
            .sessions
            .lock()
            .expect("playback sessions are not poisoned");
        sessions.retain(|_, session| session.strong_count() > 0);
        if let Some(session) = sessions.get(id).and_then(Weak::upgrade) {
            session
        } else {
            let session = Arc::new(Semaphore::const_new(1));
            sessions.insert(id.to_owned(), Arc::downgrade(&session));
            session
        }
    }
}

static PLAYBACK_SLOTS: LazyLock<PlaybackSlots> = LazyLock::new(PlaybackSlots::new);
static THUMBNAIL_SLOTS: Semaphore = Semaphore::const_new(1);
static CHAPTER_PROBE_SLOTS: Semaphore = Semaphore::const_new(1);

#[derive(Deserialize)]
pub struct PlaybackQuery {
    segment: Option<u32>,
    plan: Option<String>,
    revision: Option<u64>,
    profile: Option<String>,
    session: Option<String>,
}

pub async fn playback(
    State(state): State<AppState>,
    RoutePath(path): RoutePath<String>,
    Query(query): Query<PlaybackQuery>,
    method: Method,
) -> Result<Response, AppError> {
    let input = resolve_file(&state, &path).await?;
    let read = state.media.read(&input)?;
    if let Some(index) = query.segment {
        let token = query
            .plan
            .as_deref()
            .ok_or_else(|| AppError::bad_request("Missing playback plan"))?;
        let plan = PlaybackPlan::decode(&path, token)?;
        let segment = plan.timeline.segment(index)?;
        let (bytes, cached) = with_playback_slot(
            plan.session.as_deref(),
            encode_segment(&state, &input, &path, &plan, index, segment, &read),
        )
        .await?;
        let mut response = (
            [
                (header::CONTENT_TYPE, "video/mp2t"),
                (header::CACHE_CONTROL, "private, max-age=3600"),
            ],
            bytes,
        )
            .into_response();
        if cached {
            response
                .headers_mut()
                .insert("x-lantube-cache", HeaderValue::from_static("hit"));
        }
        return Ok(response);
    }

    let profile = match query.profile.as_deref() {
        None => Profile::P720,
        Some(value) => Profile::parse(value)
            .ok_or_else(|| AppError::bad_request("Invalid playback profile"))?,
    };
    let session = sanitize_session(query.session.as_deref());
    let revision = file_revision(&input).await?;
    if let Some(expected) = query.revision {
        require_revision(revision, expected)?;
    }
    let timeline = with_playback_slot(
        session.as_deref(),
        playback_timeline(&state.tools, &input, method, &read, profile),
    )
    .await?;
    check_revision(&input, revision).await?;
    let headers = [
        (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
        (header::CACHE_CONTROL, "no-store"),
    ];
    let Some(timeline) = timeline else {
        // 原生 HLS 的 HEAD 只确认有视频轨，逐包探测留给实际请求。
        return Ok((headers, ()).into_response());
    };
    let plan = PlaybackPlan {
        revision,
        profile,
        session,
        timeline,
    };
    let token = plan.encode(&path)?;
    let duration = plan.timeline.duration.to_string();
    let chunks = plan.timeline.playlist_chunks(token)?;
    Ok((
        headers,
        [(
            header::HeaderName::from_static("x-media-duration"),
            duration,
        )],
        Body::from_stream(stream::iter(chunks.map(Ok::<_, Infallible>))),
    )
        .into_response())
}

async fn encode_segment(
    state: &AppState,
    input: &Path,
    path: &str,
    plan: &PlaybackPlan,
    index: u32,
    segment: Segment,
    read: &ReadLease,
) -> Result<(Vec<u8>, bool), AppError> {
    check_revision(input, plan.revision).await?;
    let cacheable = !plan.profile.is_original() || state.cache_original_remux;
    let cache_key = state
        .cache
        .as_ref()
        .filter(|cache| cache.available() && cacheable)
        .map(|_| TranscodeCache::key(path, plan.revision, plan.profile.as_str(), index));
    if let (Some(cache), Some(key)) = (&state.cache, &cache_key) {
        // 同一个分片只允许一个 ffmpeg 任务，其他请求等待后直接读缓存。
        let _flight = cache.lock(key).await;
        if let Some(bytes) = cache.get(key).await {
            return Ok((bytes, true));
        }
        let bytes = encode_uncached(state, input, plan, segment, read).await?;
        cache.put(key, &bytes).await;
        return Ok((bytes, false));
    }
    Ok((
        encode_uncached(state, input, plan, segment, read).await?,
        false,
    ))
}

async fn encode_uncached(
    state: &AppState,
    input: &Path,
    plan: &PlaybackPlan,
    segment: Segment,
    read: &ReadLease,
) -> Result<Vec<u8>, AppError> {
    if plan.profile.is_original() {
        encode_original_segment(state, input, plan.revision, segment, read).await
    } else {
        encode_transcoded_segment(
            state,
            input,
            plan.revision,
            plan.profile,
            plan.timeline.fps(),
            segment,
            read,
        )
        .await
    }
}

struct ProfileSettings {
    max_width: u32,
    max_height: u32,
    bitrate: &'static str,
    maxrate: &'static str,
    bufsize: &'static str,
    crf: u32,
}

impl ProfileSettings {
    fn for_profile(profile: Profile) -> Option<Self> {
        match profile {
            Profile::Original => None,
            Profile::P1080 => Some(Self {
                max_width: 1920,
                max_height: 1080,
                bitrate: "3M",
                maxrate: "3M",
                bufsize: "6M",
                crf: 28,
            }),
            Profile::P720 => Some(Self {
                max_width: 1280,
                max_height: 720,
                bitrate: "2M",
                maxrate: "2M",
                bufsize: "4M",
                crf: 28,
            }),
            Profile::P480 => Some(Self {
                max_width: 854,
                max_height: 480,
                bitrate: "1M",
                maxrate: "1M",
                bufsize: "2M",
                crf: 30,
            }),
        }
    }
}

async fn encode_original_segment(
    state: &AppState,
    input: &Path,
    revision: u64,
    segment: Segment,
    read: &ReadLease,
) -> Result<Vec<u8>, AppError> {
    check_revision(input, revision).await?;
    let mut command = ffmpeg_copy(
        &state.tools,
        input,
        (segment.offset > 0.0).then_some(segment.offset),
    )?;
    command.args([
        "-t",
        &segment.length.to_string(),
        "-map",
        "0:V:0",
        "-c:v",
        "copy",
    ]);
    if let Some(audio) = segment.audio {
        command.args([
            "-map",
            &format!("0:a:{audio}"),
            "-c:a",
            "aac",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-b:a",
            "128k",
            "-af",
            AUDIO_FILTER,
        ]);
    } else {
        command.arg("-an");
    }
    command.args([
        "-muxdelay",
        "0",
        "-muxpreload",
        "0",
        "-output_ts_offset",
        &segment.offset.to_string(),
        "-f",
        "mpegts",
        "pipe:1",
    ]);
    let bytes = run(&mut command, "ffmpeg", read).await?;
    check_revision(input, revision).await?;
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
async fn encode_transcoded_segment(
    state: &AppState,
    input: &Path,
    revision: u64,
    profile: Profile,
    fps: u32,
    segment: Segment,
    read: &ReadLease,
) -> Result<Vec<u8>, AppError> {
    check_revision(input, revision).await?;
    let settings = ProfileSettings::for_profile(profile)
        .ok_or_else(|| AppError::bad_request("Invalid playback profile"))?;
    let scale = format!(
        "scale=w='min({},iw)':h='min({},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2:flags=bilinear",
        settings.max_width, settings.max_height
    );
    let derived = if let (Some(track), Some(window)) = (segment.audio, segment.audio_window) {
        Some((
            window,
            state
                .media
                .audio
                .track(&state.tools, input, revision, track, window, read)
                .await?,
        ))
    } else {
        None
    };
    let trim = segment.offset - segment.seek;
    let needs_original = derived.is_none() || segment.picture.is_some();
    // 音频已单独定位时，从实际首帧开始找画面，避免部分容器在首帧之前定位失败。
    let picture_delay = segment
        .picture
        .as_ref()
        .filter(|_| derived.is_some())
        .map_or(0.0, |picture| picture.start);
    let video_seek = segment.seek + picture_delay;
    let mut command = if needs_original {
        ffmpeg(
            &state.tools,
            input,
            (video_seek > 0.0).then_some(video_seek),
        )?
    } else {
        ffmpeg_command(&state.tools)?
    };

    let video = if segment.picture.is_some() {
        "0:V:0"
    } else {
        command.args([
            "-itsoffset",
            &trim.to_string(),
            "-f",
            "lavfi",
            "-i",
            "color=s=640x360:r=30",
        ]);
        if needs_original { "1:v:0" } else { "0:v:0" }
    };
    let pcm = if let Some((window, track)) = &derived {
        add_audio_input(&mut command);
        Some(audio_slice(track, *window, segment.offset, segment.length)?)
    } else {
        None
    };
    if let Some(picture) = &segment.picture {
        let mut filter = if picture_delay > 0.0 {
            hold_frames(&format!("{scale},setpts=PTS+{picture_delay}/TB"), trim, fps)
        } else {
            hold_frames(&scale, trim, fps)
        };
        if picture.start > 0.0 || picture.end < segment.length {
            write!(
                filter,
                ",drawbox=c=black:t=fill:enable='lt(t,{})+gte(t,{})'",
                trim + picture.start,
                trim + picture.end,
            )
            .expect("writing to a String cannot fail");
        }
        command.args(["-vf", &filter]);
    }
    if trim > 0.0 {
        command.args(["-ss", &trim.to_string()]);
    }
    command.args(["-map", video]);
    if derived.is_some() {
        command.args(["-map", "1:a:0"]);
    } else if let Some(audio) = segment.audio {
        command.args(["-map", &format!("0:a:{audio}")]);
    }
    let length = segment.length.to_string();
    let crf = settings.crf.to_string();
    let gop = (fps.saturating_mul(2)).max(1).to_string();
    command.args(["-t", &length, "-af", AUDIO_FILTER]);
    match state.tools.h264_encoder().await {
        H264Encoder::X264 => command.args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            &crf,
            "-maxrate",
            settings.maxrate,
            "-bufsize",
            settings.bufsize,
            "-pix_fmt",
            "yuv420p",
            "-g",
            &gop,
            "-bf",
            "0",
            "-threads",
            "2",
        ]),
        encoder => command.args([
            "-c:v",
            encoder.name(),
            "-b:v",
            settings.bitrate,
            "-maxrate",
            settings.maxrate,
            "-bufsize",
            settings.bufsize,
            "-pix_fmt",
            "yuv420p",
            "-g",
            &gop,
        ]),
    };
    command.args([
        "-c:a",
        "aac",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-b:a",
        "128k",
        "-muxdelay",
        "0",
        "-muxpreload",
        "0",
        "-output_ts_offset",
        &segment.offset.to_string(),
        "-f",
        "mpegts",
        "pipe:1",
    ]);
    let bytes = match pcm {
        Some(audio) => run_with_audio(&mut command, audio, read).await?,
        None => run(&mut command, "ffmpeg", read).await?,
    };
    check_revision(input, revision).await?;
    Ok(bytes)
}

pub async fn thumbnail(
    State(state): State<AppState>,
    RoutePath(path): RoutePath<String>,
) -> Result<Response, AppError> {
    let input = resolve_file(&state, &path).await?;
    let read = state.media.read(&input)?;
    let revision = file_revision(&input).await?;
    let (bytes, duration) = with_media_slot(&THUMBNAIL_SLOTS, async {
        let timing = thumbnail_timing(&state.tools, &input, &read).await?;
        let filter = hold_frames(
            "scale=w='min(480,iw)':h='min(360,ih)':force_original_aspect_ratio=decrease",
            0.0,
            FRAME_RATE,
        );
        let bytes = run(
            ffmpeg(&state.tools, &input, timing.seek)?.args([
                "-map",
                "0:V:0",
                "-an",
                "-frames:v",
                "1",
                "-vf",
                &filter,
                "-c:v",
                "mjpeg",
                "-pix_fmt",
                "yuvj420p",
                "-threads",
                "1",
                "-q:v",
                "4",
                "-f",
                "image2pipe",
                "pipe:1",
            ]),
            "ffmpeg",
            &read,
        )
        .await?;
        Ok((bytes, timing.duration))
    })
    .await?;
    check_revision(&input, revision).await?;
    let mut response = (
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response();
    if let Some(duration) = duration {
        response.headers_mut().insert(
            "x-media-duration",
            HeaderValue::from_str(&duration.to_string()).expect("duration is a valid header value"),
        );
    }
    Ok(response)
}

pub async fn probe_chapter_duration(
    tools: &MediaTools,
    input: &Path,
    read: &ReadLease,
) -> Result<f64, AppError> {
    with_media_slot(&CHAPTER_PROBE_SLOTS, chapter_duration(tools, input, read)).await
}

fn hold_frames(scale: &str, start: f64, fps: u32) -> String {
    format!("{scale},tpad=stop_mode=clone:stop=-1,fps={fps}:start_time={start}")
}

async fn with_playback_slot<T>(
    session: Option<&str>,
    work: impl Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    timeout(Duration::from_secs(45), async {
        let _session_permit: Option<OwnedSemaphorePermit> = match session {
            Some(id) => Some(
                PLAYBACK_SLOTS
                    .session(id)
                    .acquire_owned()
                    .await
                    .expect("session semaphore must stay open"),
            ),
            None => None,
        };
        let _global = PLAYBACK_SLOTS
            .global
            .acquire()
            .await
            .expect("global playback semaphore must stay open");
        work.await
    })
    .await
    .map_err(|_| AppError::new(StatusCode::GATEWAY_TIMEOUT, "Media conversion timed out"))?
}

async fn with_media_slot<T>(
    slots: &Semaphore,
    work: impl Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    // 探测和转码共用一次排队与处理时限。
    timeout(Duration::from_secs(45), async {
        let _permit = slots
            .acquire()
            .await
            .expect("media semaphore must stay open");
        work.await
    })
    .await
    .map_err(|_| AppError::new(StatusCode::GATEWAY_TIMEOUT, "Media conversion timed out"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    #[tokio::test]
    async fn rejects_a_segment_after_the_source_is_replaced() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("video.mp4");
        fs::write(&file, b"old").await.unwrap();
        let revision = file_revision(&file).await.unwrap();
        fs::write(&file, b"replacement").await.unwrap();
        let state = AppState {
            data: std::collections::HashMap::from([("media".into(), folder.path().to_owned())]),
            ..Default::default()
        };
        let path = "media/video.mp4";
        let plan = PlaybackPlan {
            revision,
            profile: Profile::P720,
            session: None,
            timeline: serde_json::from_str(r#"{"picture":{"start":0,"end":4},"duration":4}"#)
                .unwrap(),
        };
        let result = playback(
            State(state),
            RoutePath(path.into()),
            Query(PlaybackQuery {
                segment: Some(0),
                plan: Some(plan.encode(path).unwrap()),
                revision: None,
                profile: None,
                session: None,
            }),
            Method::GET,
        )
        .await;
        assert_eq!(
            result.unwrap_err().into_response().status(),
            StatusCode::CONFLICT
        );
    }
}
