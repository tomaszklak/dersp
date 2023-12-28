use std::{io::Cursor, net::SocketAddr};

use anyhow::{anyhow, bail};
use httparse::Status;
use log::{debug, trace};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{tcp::OwnedWriteHalf, TcpStream},
    spawn,
    sync::mpsc::{channel, Receiver, Sender},
};

use crate::{
    client::WriteLoopCommands,
    crypto::{PublicKey, SecretKey},
    proto_old::{
        exchange_keys, read_frame, read_server_info, write_frame, write_peer_present, FrameType,
    },
    service::ServiceCommand,
};

/// Max TCP packet size is 65535
const MAX_TCP_PACKET_SIZE: usize = u16::MAX as usize;

pub struct MeshClient {
    addr: SocketAddr,
    secret_key: SecretKey,
    meshkey: String,
    command_sender: Sender<ServiceCommand>,
}

impl MeshClient {
    pub fn new(
        addr: SocketAddr,
        secret_key: SecretKey,
        meshkey: String,
        command_sender: Sender<ServiceCommand>,
    ) -> Self {
        Self {
            addr,
            secret_key,
            meshkey,
            command_sender,
        }
    }

    pub async fn start(self) -> anyhow::Result<(Sender<WriteLoopCommands>, PublicKey)> {
        let stream = TcpStream::connect(self.addr).await?;
        let (sender, receiver) = channel(1);
        let (mesh_peer_pk_sender, mesh_peer_pk_receiver) = tokio::sync::oneshot::channel();
        spawn(self.run(stream, sender.clone(), receiver, mesh_peer_pk_sender));
        let mesh_peer_pk = mesh_peer_pk_receiver.await?;
        Ok((sender, mesh_peer_pk))
    }

    pub async fn run(
        self,
        stream: TcpStream,
        sender: Sender<WriteLoopCommands>,
        receiver: Receiver<WriteLoopCommands>,
        mesh_peer_pk_sender: tokio::sync::oneshot::Sender<PublicKey>,
    ) -> anyhow::Result<()> {
        // TODO: handle closing of the mesh_peer_pk_sender when there is some error?
        // Maybe this is already handled by the receiver returning result?
        let server_addr = stream.peer_addr()?;
        let (mut r, mut w) = stream.into_split();

        let leftovers = connect_http(&mut r, &mut w).await?;
        let mut reader = Cursor::new(leftovers).chain(r);

        let mesh_peer_pk = exchange_keys(&mut reader, &mut w, self.secret_key, Some(&self.meshkey))
            .await
            .map_err(|e| anyhow!("{e}"))?;

        mesh_peer_pk_sender
            .send(mesh_peer_pk)
            .map_err(|e| anyhow!("{e}"))?;

        read_server_info(&mut reader)
            .await
            .map_err(|e| anyhow!("{e}"))?;

        write_frame(&mut w, crate::proto_old::FrameType::WatchConns, Vec::new())
            .await
            .map_err(|e| anyhow!("{e}"))?;

        trace!(
            "starting read loop of mesh client {} connected to {mesh_peer_pk} ({})",
            self.secret_key.public(),
            server_addr
        );

        spawn(write_loop(receiver, w));

        loop {
            let next_frame = read_frame(&mut reader).await;
            if let Ok(next_frame) = &next_frame {
                trace!("next frame: {:?}", next_frame.0);
            }

            match next_frame {
                Ok((FrameType::PeerPresent, buf)) => {
                    let client_pk: PublicKey = buf.try_into().map_err(|e| anyhow!("{e:?}"))?;
                    trace!("Got peer present for {client_pk}");
                    self.command_sender
                        .send(ServiceCommand::PeerPresent(client_pk, sender.clone()))
                        .await
                        .unwrap();
                }
                Ok((FrameType::RecvPacket, buf)) => {
                    let sender_pk: PublicKey = buf[..32].try_into().unwrap();
                    debug!("Got recv packet from sender {sender_pk}");
                    self.command_sender
                        .send(ServiceCommand::SendPacket(sender_pk, buf[32..].to_vec()))
                        .await
                        .unwrap();
                }
                Ok(_) => todo!(),
                Err(_) => todo!(),
            }
        }
    }
}

async fn write_loop(mut r: Receiver<WriteLoopCommands>, mut writer: OwnedWriteHalf) {
    loop {
        match r.recv().await {
            Some(WriteLoopCommands::PeerPresent(pk)) => {
                write_peer_present(&mut writer, &pk).await.unwrap();
            }
            Some(x) => todo!("{x:?}"),
            None => todo!(),
        }
    }
}

async fn connect_http<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    reader: &mut R,
    writer: &mut W,
    // server_keepalives: &DerpKeepaliveConfig,
    // host: &str,
) -> anyhow::Result<Vec<u8>> {
    writer
        .write_all(
            format!(
                // TODO: host header!
                "GET /derp HTTP/1.1\r\n\
                Connection: Upgrade\r\n\
                Upgrade: WebSocket\r\n\
                User-Agent: telio/{} {}\r\n\r\n",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                // TODO: server_keepalives.tcp_keepalive,
                // TODO: server_keepalives.derp_keepalive,
            )
            .as_bytes(),
        )
        .await?;

    let mut data = [0_u8; MAX_TCP_PACKET_SIZE];
    let data_len = reader.read(&mut data).await?;

    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut res = httparse::Response::new(&mut headers);
    let res_len = match res.parse(&data)? {
        Status::Partial => {
            bail!("HTTP Response not full");
        }
        Status::Complete(len) => len,
    };
    Ok(data
        .get(res_len..data_len)
        .ok_or_else(|| anyhow!("Out of bounds index for data buffer"))?
        .to_vec())
}
