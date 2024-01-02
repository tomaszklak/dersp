use crate::{
    crypto::PublicKey,
    inout::DerpReader,
    proto::data::{ForwardPacket, Frame, FrameType, PeerPresent, RecvPacket, SendPacket},
    proto::{write_forward_packet, write_peer_present},
    service::ServiceCommand,
};
use anyhow::{anyhow, Result};
use codec::{Decode, Encode, SizeWrapper};
use log::{debug, trace, warn};
use std::net::SocketAddr;
use tokio::{
    io::AsyncWriteExt,
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    spawn,
    sync::mpsc::{channel, Receiver, Sender},
};

pub struct Client {
    _peer: SocketAddr,
    r: OwnedReadHalf,
    w: OwnedWriteHalf,
    pk: PublicKey,
    can_mesh: bool,
}

impl Client {
    pub fn new(socket: TcpStream, pk: PublicKey, can_mesh: bool) -> Result<Self> {
        let _peer = socket.peer_addr()?;
        let (r, w) = socket.into_split();
        Ok(Self {
            _peer,
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
        let sink = Self::start_write_loop(w, self.pk, self.can_mesh);
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
                warn!("[{pk:?}] Read loop failed: {e}");
                // TODO: close whole client?
            }
        });
    }

    pub async fn read_loop(
        r: OwnedReadHalf,
        pk: PublicKey,
        command_sender: Sender<ServiceCommand>,
        can_mesh: bool,
        our_sink: Sender<WriteLoopCommands>,
    ) -> anyhow::Result<()> {
        trace!("[{pk:?}] starting read loop");
        let mut derp_reader = DerpReader::new(r);

        loop {
            let message = derp_reader.get_next_message().await?;
            trace!("[{pk:?}] next frame: {:?}", message.ty);

            match message.ty {
                FrameType::SendPacket => {
                    let send_packet = Frame::<SendPacket>::decode(&mut message.buffer.as_slice())
                        .map_err(|_| anyhow!("Decode error"))?
                        .inner
                        .into_inner();
                    let is_forward = send_packet.target != pk;
                    debug!("[{pk:?}] send_packet: {send_packet:?}, can mesh: {can_mesh}, is forward: {is_forward}");
                    command_sender
                        .send(ServiceCommand::SendPacket {
                            source: pk,
                            target: send_packet.target,
                            payload: send_packet.payload,
                        })
                        .await?;
                }

                FrameType::WatchConns => {
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

                FrameType::PeerPresent => {
                    let peer_present = Frame::<PeerPresent>::decode(&mut message.buffer.as_slice())
                        .map_err(|_| anyhow!("Decode error"))?
                        .inner
                        .into_inner();
                    debug!(
                        "[{pk:?}] will handle messages for {:?} (can mesh: {can_mesh})",
                        peer_present.public_key,
                    );
                    command_sender
                        .send(ServiceCommand::PeerPresent(
                            peer_present.public_key,
                            our_sink.clone(),
                        ))
                        .await
                        .unwrap();
                }

                frame_type => todo!("frame type: {frame_type:?}"),
            }
        }
    }

    pub fn start_write_loop(
        w: OwnedWriteHalf,
        pk: PublicKey,
        can_mesh: bool,
    ) -> Sender<WriteLoopCommands> {
        let (s, r) = channel(1);

        spawn(Self::write_loop(r, w, pk, can_mesh));

        s
    }
    pub async fn write_loop(
        mut r: Receiver<WriteLoopCommands>,
        mut w: OwnedWriteHalf,
        pk: PublicKey,
        can_mesh: bool,
    ) -> anyhow::Result<()> {
        loop {
            match r.recv().await {
                Some(WriteLoopCommands::SendPacket {
                    source,
                    target,
                    payload,
                }) => match (can_mesh, target != pk) {
                    (true, true) => {
                        trace!("[{pk:?}] Will forward packet from {source:?} to {target:?}");
                        let forward_packet = ForwardPacket::new(source, target, payload);
                        write_forward_packet(&mut w, forward_packet).await?;
                    }

                    (_, false) => {
                        let mut writing_buffer = Vec::new();
                        trace!("[{pk:?}] Will send {} bytes to {target}", payload.len());
                        let frame = Frame {
                            frame_type: FrameType::RecvPacket,
                            inner: SizeWrapper::new(RecvPacket { target, payload }),
                        };
                        frame.encode(&mut writing_buffer)?;
                        w.write_all(&writing_buffer)
                            .await
                            .map_err(|e| anyhow!("{e}"))?;
                    }

                    (false, true) => todo!(),
                },
                Some(WriteLoopCommands::_Stop) => {
                    debug!("[{pk:?}] write loop stopping");
                    return Ok(());
                }
                Some(WriteLoopCommands::PeerPresent(pk)) => {
                    trace!("[{pk:?}] Sending peer present with {pk}");
                    write_peer_present(&mut w, &pk).await?;
                }
                None => {
                    debug!("[{pk:?}] write loop stopping (no more commands)");
                    return Ok(());
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum WriteLoopCommands {
    SendPacket {
        source: PublicKey,
        target: PublicKey,
        payload: Vec<u8>,
    },
    PeerPresent(PublicKey),
    _Stop,
}
