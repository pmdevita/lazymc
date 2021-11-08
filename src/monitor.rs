// TODO: remove all unwraps/expects here!

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use minecraft_protocol::data::server_status::ServerStatus;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::status::StatusResponse;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::proto::{self, ClientState, RawPacket};
use crate::server::ServerState;

/// Minecraft protocol version used when polling server status.
const PROTOCOL_VERSION: i32 = 754;

/// Monitor ping inverval in seconds.
const MONITOR_PING_INTERVAL: u64 = 2;

/// Status request timeout in seconds.
const STATUS_TIMEOUT: u64 = 8;

/// Monitor server.
pub async fn monitor_server(addr: SocketAddr, state: Arc<ServerState>) {
    loop {
        trace!("Fetching status for {} ... ", addr);
        let status = poll_server(addr).await;

        // Update server state
        state.set_online(status.is_some());
        if let Some(status) = status {
            state.set_status(status);
        }

        tokio::time::sleep(Duration::from_secs(MONITOR_PING_INTERVAL)).await;
    }
}

/// Poll server state.
///
/// Returns server status if connection succeeded.
pub async fn poll_server(addr: SocketAddr) -> Option<ServerStatus> {
    fetch_status(addr).await.ok()
}

/// Attemp to fetch status from server.
async fn fetch_status(addr: SocketAddr) -> Result<ServerStatus, ()> {
    let mut stream = TcpStream::connect(addr).await.map_err(|_| ())?;

    send_handshake(&mut stream, addr).await?;
    request_status(&mut stream).await?;
    wait_for_status_timeout(&mut stream).await
}

/// Send handshake.
async fn send_handshake(stream: &mut TcpStream, addr: SocketAddr) -> Result<(), ()> {
    let handshake = Handshake {
        protocol_version: PROTOCOL_VERSION,
        server_addr: addr.ip().to_string(),
        server_port: addr.port(),
        next_state: ClientState::Status.to_id(),
    };

    let mut packet = Vec::new();
    handshake.encode(&mut packet).map_err(|_| ())?;

    let raw = RawPacket::new(proto::HANDSHAKE_PACKET_ID_HANDSHAKE, packet)
        .encode()
        .map_err(|_| ())?;

    stream.write_all(&raw).await.map_err(|_| ())?;

    Ok(())
}

/// Send status request.
async fn request_status(stream: &mut TcpStream) -> Result<(), ()> {
    let raw = RawPacket::new(proto::STATUS_PACKET_ID_STATUS, vec![])
        .encode()
        .map_err(|_| ())?;
    stream.write_all(&raw).await.map_err(|_| ())?;
    Ok(())
}

/// Wait for a status response.
async fn wait_for_status(stream: &mut TcpStream) -> Result<ServerStatus, ()> {
    // Get stream reader, set up buffer
    let (mut reader, mut _writer) = stream.split();
    let mut buf = BytesMut::new();

    loop {
        // Read packet from stream
        let (packet, _raw) = match proto::read_packet(&mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => continue,
        };

        // Catch status response
        if packet.id == proto::STATUS_PACKET_ID_STATUS {
            let status = StatusResponse::decode(&mut packet.data.as_slice()).map_err(|_| ())?;
            return Ok(status.server_status);
        }
    }

    // Some error occurred
    Err(())
}

/// Wait for a status response.
async fn wait_for_status_timeout(stream: &mut TcpStream) -> Result<ServerStatus, ()> {
    let status = wait_for_status(stream);
    tokio::time::timeout(Duration::from_secs(STATUS_TIMEOUT), status)
        .await
        .map_err(|_| ())?
}
