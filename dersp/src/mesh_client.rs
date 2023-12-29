use std::{io::Cursor, net::SocketAddr};

use anyhow::{anyhow, bail};
use codec::Decode;
use httparse::Status;
use log::{trace, warn};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{lookup_host, tcp::OwnedWriteHalf, TcpStream},
    spawn,
    sync::mpsc::{channel, Receiver, Sender},
};
use log::debug;

use crate::{
    client::WriteLoopCommands,
    crypto::{PublicKey, SecretKey},
    inout::DerpReader,
    proto::data::{ForwardPacket, Frame, FrameType, PeerPresent},
    proto::{exchange_keys, read_server_info, write_peer_present, write_watch_conns},
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
    pub async fn new(
        addr_or_host: &str,
        secret_key: SecretKey,
        meshkey: String,
        command_sender: Sender<ServiceCommand>,
    ) -> anyhow::Result<Self> {
        if let Some(addr) = lookup_host(addr_or_host).await?.next() {
            debug!("mesh peer {addr_or_host} is in fact: {addr}");
            Ok(Self {
                addr,
                secret_key,
                meshkey,
                command_sender,
            })
        } else {
            bail!("Failed to resolve {addr_or_host}");
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
        let reader = Cursor::new(leftovers).chain(r);
        let mut derp_reader = DerpReader::new(reader);

        let mesh_peer_pk = exchange_keys(
            &mut derp_reader,
            &mut w,
            self.secret_key,
            Some(&self.meshkey),
        )
        .await?;

        mesh_peer_pk_sender
            .send(mesh_peer_pk)
            .map_err(|e| anyhow!("{e}"))?;

        read_server_info(&mut derp_reader).await?;

        write_watch_conns(&mut w).await?;

        trace!(
            "starting read loop of mesh client {} connected to {mesh_peer_pk} ({})",
            self.secret_key.public(),
            server_addr
        );

        spawn(write_loop(receiver, w));

        if let Err(e) = self.read_loop(derp_reader, sender).await {
            warn!("[{mesh_peer_pk:?}] read loop failed: {e}");
            return Err(e);
        }

        Ok(())
    }

    async fn read_loop<T: AsyncRead + Unpin>(
        self,
        mut reader: DerpReader<T>,
        sender: Sender<WriteLoopCommands>,
    ) -> anyhow::Result<()> {
        loop {
            let message = reader.get_next_message().await?;

            trace!("next frame: {:?}", message.ty);

            match message.ty {
                FrameType::PeerPresent => {
                    let peer_present = Frame::<PeerPresent>::decode(&mut message.buffer.as_slice())
                        .map_err(|_| anyhow!("Decode error"))?
                        .inner
                        .into_inner();
                    trace!("Got peer present for {}", peer_present.public_key);
                    self.command_sender
                        .send(ServiceCommand::PeerPresent(
                            peer_present.public_key,
                            sender.clone(),
                        ))
                        .await
                        .unwrap();
                }

                FrameType::ForwardPacket => {
                    let forward_packet =
                        Frame::<ForwardPacket>::decode(&mut message.buffer.as_slice())
                            .map_err(|_| anyhow!("Decode error"))?
                            .inner
                            .into_inner();
                    self.command_sender
                        .send(ServiceCommand::SendPacket {
                            source: forward_packet.source,
                            target: forward_packet.target,
                            payload: forward_packet.payload,
                        })
                        .await?;
                }

                _ => todo!(),
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
