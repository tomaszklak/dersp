use crate::{
    crypto::PublicKey,
    proto_old::{parse_send_packet, read_frame, write_frame, FrameType},
    service::ServiceCommand,
};
use anyhow::Result;
use log::{debug, trace, warn};
use std::net::SocketAddr;
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    spawn,
    sync::mpsc::{channel, Receiver, Sender},
};

pub struct Client {
    peer: SocketAddr,
    r: OwnedReadHalf,
    w: OwnedWriteHalf,
    pk: PublicKey,
    can_mesh: bool,
}

impl Client {
    pub fn new(socket: TcpStream, pk: PublicKey, can_mesh: bool) -> Result<Self> {
        let peer = socket.peer_addr()?;
        let (r, w) = socket.into_split();
        Ok(Self {
            peer,
            r,
            w,
            pk,
            can_mesh,
        })
    }

    pub async fn run(
        self,
        command_sender: Sender<ServiceCommand>,
    ) -> Result<Sender<WriteLoopCommands>> {
        let w = self.w;
        let sink = Self::start_write_loop(w, self.pk);
        let r = self.r;
        Self::start_read_loop(r, self.pk, command_sender, self.can_mesh, sink.clone());

        Ok(sink)
    }

    pub fn start_read_loop(
        r: OwnedReadHalf,
        pk: PublicKey,
        command_sender: Sender<ServiceCommand>,
        can_mesh: bool,
        our_sink: Sender<WriteLoopCommands>,
    ) {
        spawn(async move {
            if let Err(e) = Self::read_loop(r, pk, command_sender, can_mesh, our_sink).await {
                warn!("Read loop of {pk} failed");
                // TODO: close whole client?
            }
        });
    }

    pub async fn read_loop(
        mut r: OwnedReadHalf,
        pk: PublicKey,
        command_sender: Sender<ServiceCommand>,
        can_mesh: bool,
        our_sink: Sender<WriteLoopCommands>,
    ) -> anyhow::Result<()> {
        loop {
            match read_frame(&mut r).await {
                Ok((FrameType::SendPacket, buf)) => {
                    debug!("send packet buf size: {}", buf.len());
                    let send_packet = parse_send_packet(&buf)?;
                    debug!("send packet: {send_packet:?}");
                    command_sender
                        .send(ServiceCommand::SendPacket(
                            send_packet.target,
                            send_packet.payload.to_vec(),
                        ))
                        .await?;
                }
                Ok((FrameType::WatchConns, _)) => {
                    if !can_mesh {
                        // TODO: close this connection
                    } else {
                        command_sender
                            .send(ServiceCommand::SubscribeForPeerChanges(
                                pk,
                                our_sink.clone(),
                            ))
                            .await?;
                    }
                }
                Ok((frame_type, _buf)) => todo!("frame type: {frame_type:?}"),
                Err(e) => {
                    warn!("{pk}: Exiting read loop - next frame failed to read: {e}");
                    return Err(e);
                }
            }
        }
    }

    pub fn start_write_loop(w: OwnedWriteHalf, pk: PublicKey) -> Sender<WriteLoopCommands> {
        let (s, r) = channel(1);

        spawn(Self::write_loop(r, w, pk));

        s
    }
    pub async fn write_loop(
        mut r: Receiver<WriteLoopCommands>,
        mut w: OwnedWriteHalf,
        pk: PublicKey,
    ) -> anyhow::Result<()> {
        loop {
            match r.recv().await {
                Some(WriteLoopCommands::SendPacket(buf)) => {
                    trace!("Will send {} bytes to {pk}", buf.len());
                    let mut data = vec![];
                    data.extend_from_slice(&pk);
                    data.extend_from_slice(&buf);
                    write_frame(&mut w, FrameType::RecvPacket, data)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                Some(WriteLoopCommands::Stop) => {
                    debug!("{pk} write loop stopping");
                    return Ok(());
                }
                None => {
                    debug!("{pk} write loop stopping (no more commands)");
                    return Ok(());
                }
            }
        }
    }
}

pub enum WriteLoopCommands {
    SendPacket(Vec<u8>),
    Stop,
}
