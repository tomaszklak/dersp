use self::data::{ClientInfoFrame, ClientInfoPayload, Frame, FrameType, ServerInfo, ServerKey};

use crate::crypto::SecretKey;
use anyhow::{anyhow, ensure};
use codec::{Decode, Encode};
use log::debug;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub mod data;
const UPGRADE_MSG_SIZE: usize = 4096;

pub struct Server {
    socket: TcpStream,
    secret_key: SecretKey,
    state: ConnectionState,
}

impl Server {
    pub fn new(socket: TcpStream, secret_key: SecretKey) -> Self {
        Server {
            socket,
            secret_key,
            state: ConnectionState::FinalizingHttpPhase,
        }
    }

    // This should receive the new data. It should not interact directly with the socket.
    pub async fn handle(&mut self) -> anyhow::Result<()> {
        loop {
            match self.state {
                ConnectionState::FinalizingHttpPhase => {
                    self.finalize_http_phase().await?;
                    self.state = ConnectionState::PreparingServerKey;
                }

                ConnectionState::PreparingServerKey => {
                    self.write_server_key().await?;
                    self.state = ConnectionState::AwaitingClientInfo;
                }

                ConnectionState::AwaitingClientInfo => {
                    self.read_client_info().await?;
                    self.state = ConnectionState::PreparingServerInfo;
                }

                ConnectionState::PreparingServerInfo => {
                    self.write_server_info().await?;
                    self.state = ConnectionState::Established;
                }

                ConnectionState::Established => return Ok(()),
            }
        }
    }

    async fn finalize_http_phase(&mut self) -> anyhow::Result<()> {
        let mut buf = [0u8; UPGRADE_MSG_SIZE];
        let n = self.socket.read(&mut buf).await?; // TODO: timeout
        ensure!(n > 0, "empty initiall message");
        ensure!(n < UPGRADE_MSG_SIZE, "initial message too big");

        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut req = httparse::Request::new(&mut headers);
        let body_start = req.parse(&buf)?; // TODO: add context
        ensure!(body_start.is_complete());
        validate_headers(&headers)?;
        let body_start = body_start.unwrap();
        let _body = &buf[body_start..];
        // TODO: do something with body?
        self.socket.write(b"HTTP/1.1 200 OK\r\n\r\n").await?;

        Ok(())
    }

    async fn write_server_key(&mut self) -> anyhow::Result<()> {
        let mut server_key = ServerKey::new(self.secret_key.public());
        let mut buf = Vec::new();
        server_key.frame().encode(&mut buf)?;
        self.socket
            .write_all(&buf)
            .await
            .map_err(|e| anyhow!("{}", e))
    }

    async fn read_client_info(&mut self) -> anyhow::Result<()> {
        // TODO use only one prealocated buffer for read / write
        let mut buf = [0; 1024];
        self.socket.read(&mut buf).await?;

        let client_info = match FrameType::get_frame_type(&buf) {
            FrameType::ClientInfo => Frame::<ClientInfoFrame>::decode(&mut buf.as_slice())
                .map_err(|_| anyhow!("Decode error")),
            ty => anyhow::bail!("Unexpected message: {:?}", ty),
        }?;
        let client_info = client_info.inner.into_inner();
        debug!("Client public key: {:?}", client_info.public_key);

        let complete_info = client_info.complete(&self.secret_key)?;

        debug!("client info: {:?}", complete_info.payload);
        ensure!(
            complete_info.payload
                == ClientInfoPayload {
                    version: 2,
                    meshkey: "".to_owned()
                }
        );

        Ok(())
    }

    async fn write_server_info(&mut self) -> anyhow::Result<()> {
        let mut buf = Vec::new();
        ServerInfo::default().frame().encode(&mut buf)?;
        self.socket
            .write_all(&buf)
            .await
            .map_err(|e| anyhow!("{e}"))
    }
}

pub enum ConnectionState {
    FinalizingHttpPhase,
    PreparingServerKey,
    AwaitingClientInfo,
    PreparingServerInfo,
    Established,
}

fn validate_headers(headers: &[httparse::Header]) -> anyhow::Result<()> {
    for h in headers {
        if h.name == "Upgrade" {
            let value = std::str::from_utf8(h.value)?.to_ascii_lowercase();
            ensure!(
                value == "websocket" || value == "derp",
                "Unexpected Upgrade value {value}"
            );
        }

        if h.name == "Connection" {
            let value = std::str::from_utf8(h.value)?.to_ascii_lowercase();
            ensure!(value == "upgrade", "Unexpected Connection value {value}");
        }
    }

    Ok(())
}
