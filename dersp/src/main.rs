mod client;
mod crypto;
mod proto;
mod service;

use crate::service::{DerpService, Service};
use log::info;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let service: Arc<Mutex<DerpService>> = DerpService::new();
    let listener = TcpListener::bind("127.0.0.1:8800").await.unwrap();
    info!("Listening on: {:?}", listener.local_addr());

    service.run(listener).await
}
