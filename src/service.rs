use std::sync::Arc;

use crate::{
    client::{Client, WriteLoopCommands},
    crypto::PublicKey,
};
use log::{debug, info, warn};
use rustc_hash::FxHashMap;
use tokio::{
    net::TcpStream,
    spawn,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
};

#[derive(Debug)]
pub struct DerpService {
    sinks: FxHashMap<PublicKey, Sender<WriteLoopCommands>>,
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
