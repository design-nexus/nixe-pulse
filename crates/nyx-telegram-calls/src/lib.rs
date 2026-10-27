//! `nyx-telegram-calls`: Voice and video calling media engine for Nyx Pulse.
//!
//! Interfaces with Telegram's WebRTC/tgcalls calling infrastructure, PipeWire audio/video,
//! and Wayland screen casting via XDG Desktop Portal.

pub mod capture;
pub mod devices;
pub mod engine;
pub mod signaling;

pub use devices::{AvailableDevices, DeviceInfo};
pub use signaling::{TELEGRAM_CALL_EMOJIS, derive_call_emojis, format_call_duration};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CallError {
    #[error("Signaling error: {0}")]
    Signaling(String),
    #[error("Device error: {0}")]
    Device(String),
    #[error("Screen sharing portal error: {0}")]
    Portal(String),
    #[error("Call terminated")]
    Terminated,
}

/// Connection parameters extracted from TDLib `CallStateReady`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallConnectionParams {
    pub call_id: i32,
    pub encryption_key: Vec<u8>,
    pub emojis: Vec<String>,
    pub servers: Vec<CallServerEndpoint>,
    pub is_video: bool,
    pub allow_p2p: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallServerEndpoint {
    pub id: i64,
    pub ip_address: String,
    pub port: u16,
    pub is_ipv6: bool,
    pub endpoint_type: String,
}

/// Current status of an active audio/video call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    Initiating,
    Ringing,
    Connecting,
    Connected,
    Reconnecting,
    HangingUp,
    Discarded,
}

/// Manages active call media session, devices, and streaming.
pub struct CallSession {
    params: CallConnectionParams,
    state: CallState,
    is_muted: bool,
    is_camera_on: bool,
    is_screen_sharing: bool,
}

impl CallSession {
    pub fn new(params: CallConnectionParams) -> Self {
        Self { params, state: CallState::Connecting, is_muted: false, is_camera_on: false, is_screen_sharing: false }
    }

    pub fn state(&self) -> CallState {
        self.state
    }

    pub fn emojis(&self) -> &[String] {
        &self.params.emojis
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.is_muted = muted;
    }

    pub fn is_camera_on(&self) -> bool {
        self.is_camera_on
    }

    pub fn set_camera_on(&mut self, on: bool) {
        self.is_camera_on = on;
    }

    pub fn is_screen_sharing(&self) -> bool {
        self.is_screen_sharing
    }

    /// Initiates a screen capture request via the XDG Desktop Portal on Wayland/Hyprland.
    pub async fn request_screen_cast(&mut self) -> Result<u32, CallError> {
        use ashpd::desktop::CreateSessionOptions;
        use ashpd::desktop::screencast::{Screencast, SelectSourcesOptions, StartCastOptions};

        if let Ok(proxy) = Screencast::new().await {
            if let Ok(session) = proxy.create_session(CreateSessionOptions::default()).await {
                if proxy.select_sources(&session, SelectSourcesOptions::default()).await.is_ok() {
                    if let Ok(start) = proxy.start(&session, None, StartCastOptions::default()).await {
                        if let Ok(response) = start.response() {
                            if let Some(stream) = response.streams().first() {
                                self.is_screen_sharing = true;
                                return Ok(stream.pipe_wire_node_id());
                            }
                        }
                    }
                }
            }
        }

        Err(CallError::Portal("Screen capture was cancelled or unavailable".into()))
    }

    pub fn hang_up(&mut self) {
        self.state = CallState::Discarded;
        self.is_screen_sharing = false;
        self.is_camera_on = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_session_lifecycle() {
        let params = CallConnectionParams {
            call_id: 101,
            encryption_key: vec![1, 2, 3, 4],
            emojis: vec!["🍎".into(), "🚀".into(), "🌟".into(), "🐱".into()],
            servers: Vec::new(),
            is_video: false,
            allow_p2p: true,
        };

        let mut session = CallSession::new(params);
        assert_eq!(session.state(), CallState::Connecting);
        assert_eq!(session.emojis().len(), 4);

        session.set_muted(true);
        assert!(session.is_muted());

        session.hang_up();
        assert_eq!(session.state(), CallState::Discarded);
    }

    #[test]
    fn emoji_derivation_and_formatting() {
        let key = b"nyx_pulse_test_voip_encryption_key_2026";
        let emojis = derive_call_emojis(key);
        assert_eq!(emojis.len(), 4);

        assert_eq!(format_call_duration(45), "00:45");
        assert_eq!(format_call_duration(125), "02:05");
        assert_eq!(format_call_duration(3665), "01:01:05");
    }
}
