use crate::{
    client::{Client, WriteLoopCommands},
    crypto::{PublicKey, SecretKey},
    mesh_client::MeshClient,
    proto::handle_handshake,
    Config,
};
use anyhow::{bail, ensure};
use log::{debug, info, trace, warn};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    spawn,
    sync::{
        mpsc::{channel, Receiver, Sender},
        RwLock,
    },
};

pub trait Service {
    async fn run(&self, listener: TcpListener) -> anyhow::Result<()>;
}

#[derive(Debug)]
pub struct DerpService {
    peers_sinks: HashMap<PublicKey, Sender<WriteLoopCommands>>,
    mesh: HashMap<PublicKey, Sender<WriteLoopCommands>>,
    command_sender: Sender<ServiceCommand>,
    meshkey: Option<String>,
}

impl DerpService {
    pub async fn add_new_client(
        &mut self,
        socket: TcpStream,
        client_pk: PublicKey,
        meshkey: Option<String>,
    ) -> anyhow::Result<()> {
        let can_mesh = match (&self.meshkey, &meshkey) {
            (None, None) => false,
            (None, Some(_)) => {
                bail!(
                    "Client {client_pk:?} ({:?}) tried to mesh with a server that can't mesh",
                    socket.peer_addr()
                )
            }
            (Some(_), None) => false,
            (Some(server_meshkey), Some(client_meshkey)) => {
                ensure!(
                    server_meshkey == client_meshkey,
                    "Client {client_pk:?} ({:?}) tried to mesh with a wrong key",
                    socket.peer_addr()
                );
                true
            }
        };
        let client = Client::new(socket, client_pk, can_mesh)?;
        let sink = client.run(self.command_sender.clone()).await?;

        info!("will insert {client_pk:?} to peers (can mesh: {can_mesh})");
        if let Some(old) = self.peers_sinks.insert(client_pk, sink) {
            warn!("Newer client with {client_pk:?}: {old:?}");
        }

        self.notify_all_mesh_peers(client_pk).await;

        Ok(())
    }

    pub async fn new(config: Config) -> anyhow::Result<Arc<RwLock<Self>>> {
        let meshkey = config.meshkey;

        let (s, r) = channel(1);
        let service_sk = SecretKey::gen();
        info!("Service public key: {}", service_sk.public());

        let ret = Arc::new(RwLock::new(Self {
            peers_sinks: Default::default(),
            mesh: Default::default(),
            command_sender: s.clone(),
            meshkey: meshkey.clone(),
        }));
        spawn(command_loop(r, ret.clone()));
        if let Some(meshkey) = meshkey {
            for addr in config.mesh_peers {
                let mesh_client =
                    MeshClient::new(&addr, service_sk, meshkey.clone(), s.clone()).await?;
                match mesh_client.start().await {
                    Ok((sender, mesh_peer_pk)) => {
                        ret.write().await.mesh.insert(mesh_peer_pk, sender);
                    }
                    Err(e) => warn!("Failed to start peer client for {addr}: {e}"),
                }
            }
        } else {
            warn!(
                "Can't peer without a meshkey, ignoring mesh peers: {:?}",
                config.mesh_peers
            );
        }
        Ok(ret)
    }

    async fn notify_all_mesh_peers(&self, client_pk: PublicKey) {
        trace!("Will notify all mesh about new client: {client_pk:?}");
        let mesh = self.mesh.clone();
        spawn(async move {
            for (peer, sink) in mesh {
                if let Err(e) = sink
                    .send(WriteLoopCommands::PeerPresent(client_pk.clone()))
                    .await
                {
                    warn!("Failed to notify mesh peer {peer} about client {client_pk:?}: {e}");
                }
            }
        });
    }
}

// TODO: should this be RWLock instead of Mutex?
impl Service for Arc<RwLock<DerpService>> {
    async fn run(&self, listener: TcpListener) -> anyhow::Result<()> {
        loop {
            // TODO: handle panic!
            if let Ok((socket, peer_addr)) = listener.accept().await {
                let service = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(socket, peer_addr, service).await {
                        warn!("Client {peer_addr:?} failed: {e:?}");
                    }
                });
            }
        }
    }
}

async fn handle_client(
    mut socket: TcpStream,
    peer_addr: SocketAddr,
    service: Arc<RwLock<DerpService>>,
) -> anyhow::Result<()> {
    debug!("Got connection from: {peer_addr:?}");
    let sk = SecretKey::gen();
    let (client_pk, meshkey) = handle_handshake(&mut socket, &sk).await?;

    service
        .write()
        .await
        .add_new_client(socket, client_pk, meshkey)
        .await?;

    Ok(())
}

async fn command_loop(
    mut r: Receiver<ServiceCommand>,
    service: Arc<RwLock<DerpService>>,
) -> anyhow::Result<()> {
    loop {
        match r.recv().await {
            Some(ServiceCommand::SendPacket {
                source,
                target,
                payload,
            }) => {
                debug!("send packet to {target:?}");
                let sink = match service.read().await.peers_sinks.get(&target) {
                    Some(sink) => sink.clone(),
                    None => {
                        continue;
                    }
                };
                sink.send(WriteLoopCommands::SendPacket {
                    source,
                    target,
                    payload,
                })
                .await?;
            }
            Some(ServiceCommand::SubscribeForPeerChanges(mesh_peer_pk, mesh_sink)) => {
                let current_peers: Vec<PublicKey> = {
                    let mut service = service.write().await;
                    if let Some(_old) = service.mesh.insert(mesh_peer_pk, mesh_sink.clone()) {
                        warn!("Mesh peer for {mesh_peer_pk:?} overwriten");
                    }
                    let service = service.downgrade();
                    service
                        .peers_sinks
                        .keys()
                        // TODO: should we not send it:
                        .filter(|pk| !service.mesh.contains_key(pk))
                        .copied()
                        .collect()
                };

                notify_about_all_clients(mesh_peer_pk, mesh_sink, current_peers);

                trace!("Peer {mesh_peer_pk:?} added to mesh");
            }
            Some(ServiceCommand::PeerPresent(pk, sink)) => {
                let mut service = service.write().await;
                match service.peers_sinks.entry(pk) {
                    std::collections::hash_map::Entry::Occupied(_) => {
                        warn!("Ignoring already known peer: {pk:?}");
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        info!("will insert {pk:?} to peers (via peer present)");
                        e.insert(sink);
                    }
                }
            }
            Some(ServiceCommand::_Stop) => return Ok(()),
            None => return Ok(()),
        }
    }
}

fn notify_about_all_clients(
    mesh_peer_pk: PublicKey,
    mesh_sink: Sender<WriteLoopCommands>,
    clients_pk: Vec<PublicKey>,
) {
    spawn(async move {
        for pk in clients_pk {
            if let Err(e) = mesh_sink.send(WriteLoopCommands::PeerPresent(pk)).await {
                warn!("Failed to notify mesh peer {mesh_peer_pk:?} about client {pk:?}: {e}");
            }
        }
    });
}

pub enum ServiceCommand {
    _Stop,
    SendPacket {
        source: PublicKey,
        target: PublicKey,
        payload: Vec<u8>,
    },
    SubscribeForPeerChanges(PublicKey, Sender<WriteLoopCommands>),
    PeerPresent(PublicKey, Sender<WriteLoopCommands>),
}
