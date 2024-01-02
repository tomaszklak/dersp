use crate::proto::data::{FrameType, Header};
use anyhow::anyhow;
use codec::Decode;
use tokio::io::{AsyncRead, AsyncReadExt};

const HEADER_SIZE: usize = 5;
/// Max TCP packet size is 65535
const MAX_TCP_PACKET_SIZE: usize = u16::MAX as usize;

pub struct Message {
    pub ty: FrameType,
    pub buffer: Vec<u8>,
}

enum PartMessage {
    InsufficientData,
    Message(Message),
}

#[derive(Default)]
pub struct InputBuffer {
    data: Vec<u8>,
}

impl InputBuffer {
    pub fn input_data(&mut self, data: &[u8]) {
        self.data.extend(data);
    }

    fn next_message(&mut self) -> anyhow::Result<PartMessage> {
        if self.data.len() < HEADER_SIZE {
            return Ok(PartMessage::InsufficientData);
        }

        let mut header = [0; HEADER_SIZE];
        header.copy_from_slice(&self.data[..HEADER_SIZE]);
        let header = Header::decode(&mut header.as_slice()).map_err(|_| anyhow!("Decode error"))?;

        let message_size = HEADER_SIZE + (header.size as usize);
        if self.data.len() >= message_size {
            // We can extract a message
            let buffer = self.data.drain(..message_size).collect();
            return Ok(PartMessage::Message(Message {
                ty: header.frame_type,
                buffer,
            }));
        } else {
            // Insufficient data
            return Ok(PartMessage::InsufficientData);
        }
    }
}

pub struct DerpReader<T: AsyncRead + Unpin> {
    reader: T,
    read_buffer: [u8; MAX_TCP_PACKET_SIZE],
    input_buffer: InputBuffer,
}

impl<T: AsyncRead + Unpin> DerpReader<T> {
    pub fn new(reader: T) -> Self {
        DerpReader {
            reader,
            read_buffer: [0; MAX_TCP_PACKET_SIZE],
            input_buffer: InputBuffer::default(),
        }
    }

    pub async fn get_next_message(&mut self) -> anyhow::Result<Message> {
        loop {
            let message = self.input_buffer.next_message()?;
            match message {
                PartMessage::InsufficientData => {
                    let size = self.reader.read(&mut self.read_buffer).await?;
                    self.input_buffer.input_data(&self.read_buffer[..size]);
                }

                PartMessage::Message(message) => return Ok(message),
            }
        }
    }
}
