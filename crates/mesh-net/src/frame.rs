use bytes::{BufMut, BytesMut};
use mesh_core::protocol::{MAX_CONTROL_FRAME_BYTES, proto::ControlEnvelope};
use prost::Message;
use quinn::{RecvStream, SendStream};
use tokio::io::AsyncWriteExt;

use crate::{NetError, NetResult};

pub async fn write_envelope(send: &mut SendStream, envelope: &ControlEnvelope) -> NetResult<()> {
    let payload = envelope.encode_to_vec();
    if payload.is_empty() || payload.len() > MAX_CONTROL_FRAME_BYTES {
        return Err(NetError::Protocol(
            "control envelope size is invalid".to_owned(),
        ));
    }
    let mut frame = BytesMut::with_capacity(4 + payload.len());
    frame.put_u32(payload.len() as u32);
    frame.extend_from_slice(&payload);
    send.write_all(&frame).await?;
    send.flush().await?;
    Ok(())
}

pub async fn read_envelope(recv: &mut RecvStream) -> NetResult<ControlEnvelope> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_CONTROL_FRAME_BYTES {
        return Err(NetError::Protocol(format!(
            "invalid control frame length {len}"
        )));
    }
    let mut payload = vec![0u8; len];
    recv.read_exact(&mut payload).await?;
    ControlEnvelope::decode(payload.as_slice())
        .map_err(|error| NetError::Protocol(format!("invalid control envelope: {error}")))
}
