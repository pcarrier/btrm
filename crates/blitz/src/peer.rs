use crate::signaling;
use ed25519_dalek::SigningKey;
use std::net::UdpSocket;
use std::time::Instant;
use str0m::change::SdpOffer;
use str0m::channel::ChannelId;
use str0m::net::Receive;
use str0m::{Candidate, Event, Input, Output, Rtc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

pub async fn handle_peer(
    peer_session_id: String,
    sock_path: String,
    signing_key: SigningKey,
    signal_url_base: String,
    public_key_hex: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let signal_url = format!(
        "{}/channel/{}/producer",
        signal_url_base.trim_end_matches('/'),
        public_key_hex,
    );

    let (ws, _) = tokio_tungstenite::connect_async(&signal_url).await?;
    let (mut ws_write, mut ws_read) = futures_util::StreamExt::split(ws);

    let _reg: serde_json::Value = loop {
        use futures_util::StreamExt;
        if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = ws_read.next().await
        {
            let msg: serde_json::Value = serde_json::from_str(&text)?;
            if msg.get("type").and_then(|t| t.as_str()) == Some("registered") {
                break msg;
            }
        }
    };

    let udp = UdpSocket::bind("0.0.0.0:0")?;
    udp.set_nonblocking(true)?;
    let local_addr = udp.local_addr()?;

    let mut rtc = Rtc::new(Instant::now());

    rtc.add_local_candidate(Candidate::host(local_addr, "udp").expect("valid candidate"));

    let offer: SdpOffer = loop {
        use futures_util::StreamExt;
        if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = ws_read.next().await
        {
            let msg: serde_json::Value = serde_json::from_str(&text)?;
            if msg.get("type").and_then(|t| t.as_str()) == Some("signal") {
                if let Some(data) = msg.get("data") {
                    if let Some(sdp) = data.get("sdp") {
                        let offer: SdpOffer = serde_json::from_value(sdp.clone())?;
                        break offer;
                    }
                }
            }
        }
    };

    let answer = rtc.sdp_api().accept_offer(offer)?;
    let answer_json = serde_json::to_value(&answer)?;
    let signal_data = serde_json::json!({ "sdp": answer_json });
    let msg = signaling::build_signed_message(&signing_key, &peer_session_id, &signal_data);
    use futures_util::SinkExt;
    ws_write
        .send(tokio_tungstenite::tungstenite::Message::Text(msg.into()))
        .await?;

    let tokio_udp = tokio::net::UdpSocket::from_std(udp)?;

    let mut blit_conn: Option<UnixStream> = None;
    let mut blit_channel: Option<ChannelId> = None;
    let mut buf = vec![0u8; 2000];
    let mut sock_read_buf = vec![0u8; 4 + MAX_FRAME_SIZE];

    loop {
        let timeout = loop {
            match rtc.poll_output()? {
                Output::Timeout(v) => break v,
                Output::Transmit(t) => {
                    tokio_udp.send_to(&t.contents, t.destination).await?;
                    continue;
                }
                Output::Event(ev) => {
                    match ev {
                        Event::ChannelOpen(cid, label) => {
                            eprintln!("data channel opened: {label}");
                            if label == "blit" {
                                blit_channel = Some(cid);
                                let stream = UnixStream::connect(&sock_path).await?;
                                blit_conn = Some(stream);
                            }
                        }
                        Event::ChannelData(cd) => {
                            if Some(cd.id) == blit_channel {
                                if let Some(conn) = &mut blit_conn {
                                    let data = &cd.data;
                                    if data.len() < 4 {
                                        continue;
                                    }
                                    let len = u32::from_le_bytes([
                                        data[0], data[1], data[2], data[3],
                                    ]) as usize;
                                    if data.len() < 4 + len {
                                        continue;
                                    }
                                    let payload = &data[4..4 + len];
                                    let frame_len = (payload.len() as u32).to_le_bytes();
                                    let _ = conn.write_all(&frame_len).await;
                                    if !payload.is_empty() {
                                        let _ = conn.write_all(payload).await;
                                    }
                                }
                            }
                        }
                        Event::ChannelClose(cid) => {
                            if Some(cid) == blit_channel {
                                eprintln!("blit data channel closed");
                                return Ok(());
                            }
                        }
                        Event::IceConnectionStateChange(state) => {
                            eprintln!("ICE state: {state:?}");
                            if matches!(
                                state,
                                str0m::IceConnectionState::Disconnected
                            ) {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
            }
        };

        let sleep_dur = timeout.saturating_duration_since(Instant::now());

        tokio::select! {
            result = tokio_udp.recv_from(&mut buf) => {
                let (n, source) = result?;
                if let Ok(receive) = Receive::new(
                    str0m::net::Protocol::Udp,
                    source,
                    local_addr,
                    &buf[..n],
                ) {
                    rtc.handle_input(Input::Receive(Instant::now(), receive))?;
                }
            }
            _ = tokio::time::sleep(sleep_dur) => {
                rtc.handle_input(Input::Timeout(Instant::now()))?;
            }
            result = async {
                if let Some(conn) = &mut blit_conn {
                    let mut len_buf = [0u8; 4];
                    conn.read_exact(&mut len_buf).await.map(|_| {
                        u32::from_le_bytes(len_buf) as usize
                    })
                } else {
                    std::future::pending::<Result<usize, std::io::Error>>().await
                }
            } => {
                match result {
                    Ok(len) if len <= MAX_FRAME_SIZE => {
                        if let Some(conn) = &mut blit_conn {
                            let payload_buf = &mut sock_read_buf[4..4 + len];
                            if len > 0 {
                                conn.read_exact(payload_buf).await?;
                            }
                            if let Some(cid) = blit_channel {
                                let frame_len = (len as u32).to_le_bytes();
                                let mut frame = Vec::with_capacity(4 + len);
                                frame.extend_from_slice(&frame_len);
                                if len > 0 {
                                    frame.extend_from_slice(&sock_read_buf[4..4 + len]);
                                }
                                if let Some(mut ch) = rtc.channel(cid) {
                                    let _ = ch.write(true, &frame);
                                }
                            }
                        }
                    }
                    _ => {
                        eprintln!("blit-server connection closed");
                        return Ok(());
                    }
                }
            }
            msg = async {
                use futures_util::StreamExt;
                ws_read.next().await
            } => {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = msg {
                    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    if parsed.get("type").and_then(|t| t.as_str()) == Some("signal") {
                        if let Some(data) = parsed.get("data") {
                            if let Some(candidate) = data.get("candidate") {
                                if let Ok(c) = serde_json::from_value::<Candidate>(candidate.clone()) {
                                    let _ = rtc.add_remote_candidate(c);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
