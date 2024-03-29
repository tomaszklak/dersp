use anyhow::Context;
use codec::{Decode, Encode, SizeWrapper};

use crypto_box::{
    aead::{Aead, AeadCore},
    PublicKey as BoxPublicKey, SalsaBox,
};
use serde::{Deserialize, Serialize};

use crate::crypto::{PublicKey, SecretKey};

/// 8 bytes of magic message prefix: `DERP🔑`
const MAGIC: [u8; 8] = [0x44, 0x45, 0x52, 0x50, 0xF0, 0x9F, 0x94, 0x91];

#[derive(Debug, Decode, Encode, PartialEq)]
pub enum FrameType {
    /// 8B magic + 32B public key + (0+ bytes future use)
    #[tag(0x01u8)]
    ServerKey,
    /// 32B pub key + 24B nonce + naclbox(json)
    #[tag(0x02)]
    ClientInfo,
    /// 24B nonce + naclbox(json)
    #[tag(0x03)]
    ServerInfo,
    /// 32B dest pub key + packet bytes
    #[tag(0x04)]
    SendPacket,
    /// v2: 32B src pub key + packet bytes
    #[tag(0x05)]
    RecvPacket,
    /// no payload, no-op (to be replaced with ping/pong)
    #[tag(0x06)]
    KeepAlive,
    /// 1 byte payload: 0x01 or 0x00 for whether this is client's home node
    #[tag(0x07)]
    NotePreferred,
    /// PeerGone is sent from server to client to signal that
    /// a previous sender is no longer connected. That is, if A
    /// sent to B, and then if A disconnects, the server sends
    /// PeerGone to B so B can forget that a reverse path
    /// exists on that connection to get back to A.
    /// 32B pub key of peer that's gone
    #[tag(0x08)]
    PeerGone,
    /// PeerPresent is like PeerGone, but for other
    /// members of the DERP region when they're meshed up together.
    /// 32B pub key of peer that's connected
    #[tag(0x09)]
    PeerPresent,
    /// 32B src pub key + 32B dst pub key + packet bytes
    #[tag(0x0A)]
    ForwardPacket,
    /// WatchConns is how one DERP node in a regional mesh
    /// subscribes to the others in the region.
    /// There's no payload. If the sender doesn't have permission, the connection
    /// is closed. Otherwise, the client is initially flooded with
    /// PeerPresent for all connected nodes, and then a stream of
    /// PeerPresent & PeerGone has peers connect and disconnect.
    #[tag(0x10)]
    WatchConns,
    /// ClosePeer is a privileged frame type (requires the
    /// mesh key for now) that closes the provided peer's
    /// connection. (To be used for cluster load balancing
    /// purposes, when clients end up on a non-ideal node)
    /// 32B pub key of peer to close.
    #[tag(0x11)]
    ClosePeer,
    /// 8 byte ping payload, to be echoed back in Pong
    #[tag(0x12)]
    Ping,
    /// 8 byte payload, the contents of the ping being replied to
    #[tag(0x13)]
    Pong,
    /// control message for communication with derp. Since these messages are not
    /// for communication with other peers through derp, they don't contain public_key
    #[tag(0x14)]
    ControlMessage,

    #[unknown]
    Unkonow(#[unknown] u8),
}

impl FrameType {
    pub fn get_frame_type(buf: &[u8]) -> Self {
        if let Some(first_byte) = buf.get(0).copied() {
            FrameType::decode(&mut vec![first_byte].as_slice()).unwrap_or(FrameType::Unkonow(0))
        } else {
            FrameType::Unkonow(0)
        }
    }
}

#[derive(Decode, Encode)]
pub struct Frame<T> {
    pub frame_type: FrameType,
    pub inner: SizeWrapper<u32, T>,
}

#[derive(Clone, Default, Decode, Encode)]
pub struct ServerKey {
    pub magic: [u8; 8],
    pub public_key: PublicKey,
}

impl ServerKey {
    pub fn new(public_key: PublicKey) -> Self {
        ServerKey {
            magic: MAGIC,
            public_key,
        }
    }

    /// This consume self
    pub fn frame(self) -> Frame<ServerKey> {
        Frame {
            frame_type: FrameType::ServerKey,
            inner: SizeWrapper::new(self),
        }
    }

    pub fn validate_magic(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.magic == MAGIC, "Invalid magic {:?}", self.magic);
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfoPayload {
    pub version: u32,
    #[serde(rename = "meshKey")]
    pub meshkey: String,
}

#[derive(Clone, Decode, Encode)]
pub struct ClientInfo {
    pub public_key: PublicKey,
    pub nonce: [u8; 24],
    pub cipher_text: Vec<u8>,
}

impl ClientInfo {
    pub fn new(
        secret_key: SecretKey,
        server_key: PublicKey,
        meshkey: Option<&str>,
    ) -> anyhow::Result<Self> {
        let secret_key = secret_key.into();
        let public_key = BoxPublicKey::from(&secret_key);
        let server_key = server_key.into();

        let mut rng = rand_core::OsRng;
        let nonce = SalsaBox::generate_nonce(&mut rng);
        let plain_text: Vec<u8> = if let Some(meshkey) = meshkey {
            format!("{{\"version\": 2, \"meshKey\": \"{meshkey}\"}}")
                .as_bytes()
                .to_vec()
        } else {
            b"{\"version\": 2, \"meshKey\": \"\"}".to_vec()
        };

        let b = SalsaBox::new(&server_key, &secret_key);

        let cipher_text = b
            .encrypt(&nonce, &plain_text[..])
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let nonce: [u8; 24] = nonce
            .to_vec()
            .try_into()
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        Ok(ClientInfo {
            public_key: public_key.into(),
            nonce,
            cipher_text,
        })
    }

    pub fn complete(&self, sk: &SecretKey) -> anyhow::Result<CompleteClientInfo> {
        let b = SalsaBox::new(&self.public_key.into(), &sk.into());
        let plain_text = b.decrypt(
            self.nonce.clone().as_ref().try_into()?,
            self.cipher_text.as_slice(),
        )?;
        let payload: ClientInfoPayload =
            serde_json::from_slice(&plain_text).with_context(|| "Client info parsing")?;

        Ok(CompleteClientInfo {
            public_key: self.public_key,
            nonce: self.nonce,
            payload,
        })
    }

    pub fn frame(self) -> Frame<ClientInfo> {
        Frame {
            frame_type: FrameType::ClientInfo,
            inner: SizeWrapper::new(self),
        }
    }
}

pub struct CompleteClientInfo {
    pub public_key: PublicKey,
    pub nonce: [u8; 24],
    pub payload: ClientInfoPayload,
}

#[derive(Decode, Encode, Default)]
pub struct ServerInfo {
    data: Vec<u8>,
}

impl ServerInfo {
    // This consume self
    pub fn frame(self) -> Frame<ServerInfo> {
        Frame {
            frame_type: FrameType::ServerInfo,
            inner: SizeWrapper::new(self),
        }
    }
}

#[derive(Debug, Decode, Encode)]
pub struct SendPacket {
    pub target: PublicKey,
    pub payload: Vec<u8>,
}

#[derive(Debug, Decode, Encode)]
pub struct RecvPacket {
    pub target: PublicKey,
    pub payload: Vec<u8>,
}

#[derive(Decode, Encode)]
pub struct ForwardPacket {
    pub source: PublicKey,
    pub target: PublicKey,
    pub payload: Vec<u8>,
}

impl ForwardPacket {
    pub fn new(source: PublicKey, target: PublicKey, payload: Vec<u8>) -> Self {
        ForwardPacket {
            source,
            target,
            payload,
        }
    }

    pub fn frame(self) -> Frame<ForwardPacket> {
        Frame {
            frame_type: FrameType::ForwardPacket,
            inner: SizeWrapper::new(self),
        }
    }
}

#[derive(Debug, Decode, Encode)]
pub struct PeerPresent {
    pub public_key: PublicKey,
}

#[derive(Default, Decode, Encode)]
pub struct WatchConns {
    pub data: Vec<u8>,
}

#[derive(Decode)]
pub struct Header {
    pub frame_type: FrameType,
    pub size: u32,
}

mod tests {
    use super::*;

    #[test]
    fn test_server_key_frame() {
        let data = &[
            1, 0, 0, 0, 40, 0x44, 0x45, 0x52, 0x50, 0xF0, 0x9F, 0x94, 0x91, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let server_key = ServerKey::new(PublicKey::new([0; 32]));

        let mut encoded_buf = Vec::new();
        server_key.clone().frame().encode(&mut encoded_buf).unwrap();
        assert_eq!(&encoded_buf, data);

        let decoded_server_key = Frame::<ServerKey>::decode(&mut &data[..])
            .unwrap()
            .inner
            .into_inner();
        assert_eq!(decoded_server_key.magic, server_key.magic);
        assert_eq!(decoded_server_key.public_key, server_key.public_key);
    }

    #[test]
    fn test_client_info() {
        let data = &[
            2, 0, 0, 0, 58, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
            5, 5, 5, 5, 5, 5, 5, 5, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
            2, 2, 2, 12, 12,
        ];
        let client_info = ClientInfo {
            public_key: PublicKey::new([5; 32]),
            nonce: [2; 24],
            cipher_text: vec![0xC, 0xC],
        };

        let mut encoded_buf = Vec::new();
        let frame = Frame {
            frame_type: FrameType::ClientInfo,
            inner: SizeWrapper::new(client_info.clone()),
        };
        frame.encode(&mut encoded_buf).unwrap();
        assert_eq!(&encoded_buf, data);

        let decoded_client_info = Frame::<ClientInfo>::decode(&mut &data[..])
            .unwrap()
            .inner
            .into_inner();
        assert_eq!(decoded_client_info.public_key, client_info.public_key);
        assert_eq!(decoded_client_info.nonce, client_info.nonce);
        assert_eq!(decoded_client_info.cipher_text, client_info.cipher_text);
    }
}
