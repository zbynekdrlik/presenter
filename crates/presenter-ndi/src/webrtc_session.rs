use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

/// A single WebRTC peer connection that sends H.264 video via a static sample track.
pub struct WebRtcSession {
    peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
    video_track: Arc<TrackLocalStaticSample>,
}

impl WebRtcSession {
    /// Create a new session with an H.264 video track ready for negotiation.
    pub async fn new() -> Result<Self> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;

        let api = APIBuilder::new().with_media_engine(media_engine).build();

        let config = RTCConfiguration::default();
        let peer_connection = Arc::new(api.new_peer_connection(config).await?);

        let video_track = Arc::new(TrackLocalStaticSample::new(
            webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability {
                mime_type: "video/h264".to_string(),
                ..Default::default()
            },
            "video".to_string(),
            "presenter-ndi".to_string(),
        ));

        peer_connection
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        Ok(Self {
            peer_connection,
            video_track,
        })
    }

    /// Accept a WHEP SDP offer and return the SDP answer (with ICE candidates).
    pub async fn accept_offer(&self, sdp_offer: String) -> Result<String> {
        let offer = RTCSessionDescription::offer(sdp_offer)?;
        self.peer_connection.set_remote_description(offer).await?;

        let answer = self.peer_connection.create_answer(None).await?;

        let mut gather_complete = self.peer_connection.gathering_complete_promise().await;
        self.peer_connection.set_local_description(answer).await?;
        let _ = gather_complete.recv().await;

        let local_desc = self
            .peer_connection
            .local_description()
            .await
            .ok_or_else(|| anyhow::anyhow!("no local description after setting"))?;
        Ok(local_desc.sdp)
    }

    /// Write an encoded H.264 sample to all connected peers.
    pub async fn write_video(&self, data: Vec<u8>, duration: Duration) -> Result<()> {
        self.video_track
            .write_sample(&Sample {
                data: data.into(),
                duration,
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    /// Close the peer connection.
    pub async fn close(&self) -> Result<()> {
        self.peer_connection.close().await?;
        Ok(())
    }
}
