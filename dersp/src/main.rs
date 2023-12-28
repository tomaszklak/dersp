mod client;
mod crypto;
mod proto;
mod service;

mod proto_old;

use crate::service::{DerpService, Service};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use log::info;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let service: Arc<Mutex<DerpService>> = DerpService::new();
    let listener = TcpListener::bind("127.0.0.1:8800").await.unwrap();
    info!("Listening on: {:?}", listener.local_addr());

    service.run(listener).await
}
