use crate::{
    client::{Client, WriteLoopCommands},
    crypto::{PublicKey, SecretKey},
    proto::handle_handshake,
};
use async_trait::async_trait;
use log::{debug, info, warn};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    spawn,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
};

#[async_trait]
pub trait Service {
    async fn run(&self, listener: TcpListener) -> anyhow::Result<()>;
}

#[derive(Debug)]
pub struct DerpService {
    sinks: HashMap<PublicKey, Sender<WriteLoopCommands>>,
    command_sender: Sender<ServiceCommand>,
}

impl DerpService {
    pub async fn start_new_client(
        &mut self,
        socket: TcpStream,
        client_pk: PublicKey,
    ) -> anyhow::Result<()> {
        let client = Client::new(socket, client_pk)?;
        let sink = client.run(self.command_sender.clone()).await?;

        info!("will insert {client_pk}");
        if let Some(old) = self.sinks.insert(client_pk, sink) {
            warn!("Newer client with {client_pk}: {old:?}");
        }

        Ok(())
    }

    pub fn new() -> Arc<Mutex<Self>> {
        let (s, r) = channel(1);

        let ret = Arc::new(Mutex::new(Self {
            sinks: Default::default(),
            command_sender: s,
        }));
        spawn(command_loop(r, ret.clone()));
        ret
    }
}

#[async_trait]
impl Service for Arc<Mutex<DerpService>> {
    async fn run(&self, listener: TcpListener) -> anyhow::Result<()> {
        loop {
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
    service: Arc<Mutex<DerpService>>,
) -> anyhow::Result<()> {
    debug!("Got connection from: {peer_addr:?}");
    let sk = SecretKey::gen();
    let client_pk = handle_handshake(&mut socket, &sk).await?;

    service
        .lock()
        .await
        .start_new_client(socket, client_pk)
        .await?;

    Ok(())
}

async fn command_loop(
    mut r: Receiver<ServiceCommand>,
    service: Arc<Mutex<DerpService>>,
) -> anyhow::Result<()> {
    loop {
        match r.recv().await {
            Some(ServiceCommand::SendPacket(pk, buf)) => {
                debug!("send packet to {pk}");
                let service = service.lock().await;
                match service.sinks.get(&pk) {
                    Some(sink) => {
                        sink.send(WriteLoopCommands::SendPacket(buf)).await?;
                    }
                    None => {}
                }
            }
            Some(ServiceCommand::Stop) => return Ok(()),
            None => return Ok(()),
        }
    }
}

pub enum ServiceCommand {
    Stop,
    SendPacket(PublicKey, Vec<u8>),
}
