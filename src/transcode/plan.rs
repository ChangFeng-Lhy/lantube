use super::timeline::PlaybackTimeline;
use crate::error::AppError;
use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::LazyLock;

type PlanMac = Hmac<Sha256>;

static SIGNING_KEY: LazyLock<Result<[u8; 32], getrandom::Error>> = LazyLock::new(|| {
    let mut key = [0; 32];
    getrandom::fill(&mut key)?;
    Ok(key)
});

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "720p")]
    #[default]
    P720,
    #[serde(rename = "480p")]
    P480,
}

impl Profile {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "original" => Some(Self::Original),
            "1080p" => Some(Self::P1080),
            "720p" => Some(Self::P720),
            "480p" => Some(Self::P480),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::P1080 => "1080p",
            Self::P720 => "720p",
            Self::P480 => "480p",
        }
    }

    pub fn is_original(self) -> bool {
        self == Self::Original
    }
}

#[derive(Serialize, Deserialize)]
pub struct PlaybackPlan {
    pub revision: u64,
    #[serde(default)]
    pub profile: Profile,
    #[serde(default)]
    pub session: Option<String>,
    pub timeline: PlaybackTimeline,
}

pub fn sanitize_session(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then(|| value.to_owned())
}

impl PlaybackPlan {
    pub fn encode(&self, path: &str) -> Result<String, AppError> {
        let data = serde_json::to_vec(self)?;
        let mac = plan_mac(path, &data)?.finalize().into_bytes();
        Ok(format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(data),
            URL_SAFE_NO_PAD.encode(mac)
        ))
    }

    pub fn decode(path: &str, token: &str) -> Result<Self, AppError> {
        if token.len() > 512 {
            return Err(invalid_plan());
        }
        let (data, mac) = token.split_once('.').ok_or_else(invalid_plan)?;
        let data = URL_SAFE_NO_PAD.decode(data).map_err(|_| invalid_plan())?;
        let mac = URL_SAFE_NO_PAD.decode(mac).map_err(|_| invalid_plan())?;
        plan_mac(path, &data)?
            .verify_slice(&mac)
            .map_err(|_| invalid_plan())?;
        serde_json::from_slice(&data).map_err(|_| invalid_plan())
    }
}

fn plan_mac(path: &str, data: &[u8]) -> Result<PlanMac, AppError> {
    let key = SIGNING_KEY.as_ref().map_err(|_| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to initialize playback signing",
        )
    })?;
    let mut mac = PlanMac::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    // 媒体路径不得包含 NUL，用它分隔路径和计划内容。
    mac.update(path.as_bytes());
    mac.update(&[0]);
    mac.update(data);
    Ok(mac)
}

fn invalid_plan() -> AppError {
    AppError::bad_request("Invalid playback plan")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_tampering_with_the_plan_and_reuse_for_another_path() {
        let path = "share/视频 &.mp4";
        let plan = PlaybackPlan {
            revision: 42,
            profile: Profile::P720,
            session: None,
            timeline: serde_json::from_str(
                r#"{"picture":{"start":0,"end":10},"duration":10,"audio":1}"#,
            )
            .unwrap(),
        };
        let token = plan.encode(path).unwrap();
        let decoded = PlaybackPlan::decode(path, &token).unwrap();
        assert_eq!(decoded.revision, 42);
        assert_eq!(decoded.timeline.segment(2).unwrap().length, 2.0);
        assert!(PlaybackPlan::decode("share/other.mp4", &token).is_err());

        let (_, mac) = token.split_once('.').unwrap();
        let altered = PlaybackPlan {
            revision: 43,
            ..plan
        };
        let data = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&altered).unwrap());
        assert!(PlaybackPlan::decode(path, &format!("{data}.{mac}")).is_err());
    }
}
