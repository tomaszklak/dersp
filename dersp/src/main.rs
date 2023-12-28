mod client;
mod crypto;
mod mesh_client;
mod proto;
mod service;

mod proto_old;

use crate::service::{DerpService, Service};
use clap::Parser;
use log::info;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Parser, Debug)]
#[command(version)]
pub struct Config {
    /// Path to the mesh key used to authenticate with other derp servers
    #[arg(long)]
    meshkey_path: Option<PathBuf>,

    /// List of other derp servers with which we should create a mesh
    #[arg(long)]
    mesh_peers: Vec<SocketAddr>,
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let config = Config::parse();
    info!("Config: {config:?}");

    let service: Arc<Mutex<DerpService>> = DerpService::new(config).await?;
    let listener = TcpListener::bind("127.0.0.1:8800").await?;
    info!("Listening on: {:?}", listener.local_addr());

    service.run(listener).await
}
