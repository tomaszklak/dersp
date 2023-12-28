use std::{io::Cursor, net::SocketAddr};

use anyhow::{anyhow, bail};
use httparse::Status;
use log::debug;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    spawn,
};

use crate::{
    crypto::SecretKey,
    proto_old::{exchange_keys, read_server_info, write_frame},
};

/// Max TCP packet size is 65535
const MAX_TCP_PACKET_SIZE: usize = u16::MAX as usize;

pub struct PeerClient {
    addr: SocketAddr,
    secret_key: SecretKey,
    meshkey: String,
}

impl PeerClient {
    pub fn new(addr: SocketAddr, secret_key: SecretKey, meshkey: String) -> Self {
        Self {
            addr,
            secret_key,
            meshkey,
        }
    }

    pub async fn start(self) -> anyhow::Result<()> {
        let stream = TcpStream::connect(self.addr).await?;
        spawn(self.run(stream));
        Ok(())
    }

    pub async fn run(self, stream: TcpStream) -> anyhow::Result<()> {
        let (mut r, mut w) = stream.into_split();

        let leftovers = connect_http(&mut r, &mut w).await?;
        let mut reader = Cursor::new(leftovers).chain(r);

        exchange_keys(&mut reader, &mut w, self.secret_key, Some(&self.meshkey))
            .await
            .map_err(|e| anyhow!("{e}"))?;

        read_server_info(&mut reader)
            .await
            .map_err(|e| anyhow!("{e}"))?;

        debug!("Will register for updates of peers");
        write_frame(&mut w, crate::proto_old::FrameType::WatchConns, Vec::new())
            .await
            .map_err(|e| anyhow!("{e}"))?;
        debug!("did register for updates of peers");

        loop {}

        Ok(())
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
