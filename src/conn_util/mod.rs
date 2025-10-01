#![allow(dead_code)]

use crate::config::TcpConfig;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::socket::MioSocket;
use atlas_common::{channel, Err};
use atlas_communication::lookup_table::MessageModule;
use atlas_communication::message::{Header, MessageErrors, NetworkSerializedMessage, WireMessage};

use bincode::error::EncodeError;
use bytes::{Buf, Bytes, BytesMut};
use getset::Getters;
use std::io;
use std::io::{Read, Write};
use std::mem::size_of;
use std::time::Instant;
use thiserror::Error;
use tracing::{trace, warn};

pub type Callback = Option<Box<dyn FnOnce(bool) + Send>>;

pub type ConnMessage = (NetworkSerializedMessage, Instant);

/// The amount of parallel TCP connections we should try to maintain for
/// each connection
#[derive(Clone)]
pub struct ConnCounts {
    replica_connections: usize,
    client_connections: usize,
}

impl ConnCounts {
    pub(crate) fn from_tcp_config(tcp: &TcpConfig) -> Self {
        Self {
            replica_connections: tcp.replica_concurrent_connections,
            client_connections: tcp.client_concurrent_connections,
        }
    }

    /// How many connections should we maintain with a given node
    pub(crate) fn get_connections_to_node(&self, my_type: NodeType, other_type: NodeType) -> usize {
        match (my_type, other_type) {
            (NodeType::Replica, NodeType::Replica) => self.replica_connections,
            _ => self.client_connections,
        }
    }
}

/// The reading buffer for a connection
pub(crate) struct ReadingBuffer {
    pub(crate) read_bytes: usize,
    pub(crate) current_header: Option<Header>,
    pub(crate) message_module: Option<MessageModule>,
    pub(crate) reading_buffer: BytesMut,
}

/// The writing buffer for a TCP connection
#[derive(Getters)]
#[get = "pub(crate)"]
pub(crate) struct WritingBuffer {
    written_bytes: usize,
    current_header: Option<Bytes>,
    message_module: Option<Bytes>,
    current_message: Bytes,
}

/// Result of trying to write until block in a socket
pub(crate) enum ConnectionWriteWork {
    /// The connection is broken
    ConnectionBroken(usize, usize),
    /// The connection is working
    Working,
    /// The requested work is done
    Done,
}

/// The result of trying to read from a connection until block
pub(crate) enum ConnectionReadWork {
    ConnectionBroken(usize, usize),

    Working,

    WorkingAndReceived(Vec<WireMessage>),

    ReceivedAndDone(Vec<WireMessage>),
}

enum InternalWorkResult {
    ConnectionBroken(usize, usize),
    Working,
    WouldBlock,
    Interrupted,
    Done,
}

fn attempt_to_write_bytes_until_block(
    socket: &mut MioSocket,
    written_bytes: &mut usize,
    buffer: &Bytes,
) -> Result<InternalWorkResult, io::Error> {
    match socket.write(&buffer[*written_bytes..]) {
        Ok(0) => Ok(InternalWorkResult::ConnectionBroken(
            *written_bytes,
            buffer.len(),
        )),
        Ok(n) => {
            // We have successfully written n bytes
            if n + *written_bytes < buffer.len() {
                *written_bytes += n;

                Ok(InternalWorkResult::Working)
            } else {
                // It will always write atmost header.len() bytes, since
                // That's the length of the buffer
                *written_bytes = 0;

                Ok(InternalWorkResult::Done)
            }
        }
        Err(err) if would_block(&err) => {
            trace!("Would block writing header");
            Ok(InternalWorkResult::WouldBlock)
        }
        Err(err) if interrupted(&err) => Ok(InternalWorkResult::Interrupted),
        Err(err) => Err(err),
    }
}

pub(crate) fn try_write_until_block(
    socket: &mut MioSocket,
    writing_buffer: &mut WritingBuffer,
) -> Result<ConnectionWriteWork, io::Error> {
    loop {
        if let Some(header) = writing_buffer.current_header.as_ref() {
            match attempt_to_write_bytes_until_block(
                socket,
                &mut writing_buffer.written_bytes,
                header,
            )? {
                InternalWorkResult::ConnectionBroken(written_bytes, total_bytes) => {
                    return Ok(ConnectionWriteWork::ConnectionBroken(
                        written_bytes,
                        total_bytes,
                    ))
                }
                InternalWorkResult::Working => {}
                InternalWorkResult::WouldBlock => break,
                InternalWorkResult::Interrupted => continue,
                InternalWorkResult::Done => {
                    writing_buffer.current_header = None;
                }
            }
        } else if let Some(msg_mod) = writing_buffer.message_module.as_ref() {
            match attempt_to_write_bytes_until_block(
                socket,
                &mut writing_buffer.written_bytes,
                msg_mod,
            )? {
                InternalWorkResult::ConnectionBroken(written_bytes, total_bytes) => {
                    return Ok(ConnectionWriteWork::ConnectionBroken(
                        written_bytes,
                        total_bytes,
                    ))
                }
                InternalWorkResult::Working => {}
                InternalWorkResult::WouldBlock => break,
                InternalWorkResult::Interrupted => continue,
                InternalWorkResult::Done => {
                    writing_buffer.message_module = None;
                }
            }
        } else {
            match attempt_to_write_bytes_until_block(
                socket,
                &mut writing_buffer.written_bytes,
                &writing_buffer.current_message,
            )? {
                InternalWorkResult::ConnectionBroken(written_bytes, total_bytes) => {
                    return Ok(ConnectionWriteWork::ConnectionBroken(
                        written_bytes,
                        total_bytes,
                    ))
                }
                InternalWorkResult::Working => {}
                InternalWorkResult::WouldBlock => break,
                InternalWorkResult::Interrupted => continue,
                InternalWorkResult::Done => {
                    return Ok(ConnectionWriteWork::Done);
                }
            }
        }
    }

    Ok(ConnectionWriteWork::Working)
}

fn attempt_to_read_bytes_until_block(
    socket: &mut MioSocket,
    read_bytes: &mut usize,
    bytes_to_read: usize,
    buffer: &mut BytesMut,
) -> Result<InternalWorkResult, io::Error> {
    let read = if bytes_to_read > 0 {
        match socket.read(&mut buffer[*read_bytes..]) {
            Ok(0) => {
                return Ok(InternalWorkResult::ConnectionBroken(
                    *read_bytes,
                    bytes_to_read,
                ))
            }
            Ok(n) => n,
            Err(err) if would_block(&err) => return Ok(InternalWorkResult::WouldBlock),
            Err(err) if interrupted(&err) => return Ok(InternalWorkResult::Interrupted),
            Err(err) => {
                return Err!(err);
            }
        }
    } else {
        return Ok(InternalWorkResult::Done);
    };

    *read_bytes += read;

    if read >= bytes_to_read {
        Ok(InternalWorkResult::Done)
    } else {
        Ok(InternalWorkResult::Working)
    }
}

impl ReadingBuffer {
    fn prepare_for_next_read(&mut self) {
        self.reading_buffer.reserve(Header::LENGTH);
        self.reading_buffer.resize(Header::LENGTH, 0);
    }
}

pub(crate) fn read_until_block(
    socket: &mut MioSocket,
    read_info: &mut ReadingBuffer,
) -> Result<ConnectionReadWork, ReadMessageError> {
    let mut read_messages = Vec::new();

    loop {
        if let Some(header) = &read_info.current_header {
            // We have already read the header of the message

            if read_info.message_module.is_some() {
                // We have already read the module of the message, so
                // We are currently reading a message
                let currently_read = read_info.read_bytes;
                let bytes_to_read = header.payload_length() - currently_read;

                /*trace!(
                    "Reading message with {} bytes to read payload {}, currently_read {}",
                    bytes_to_read,
                    header.payload_length(),
                    currently_read
                );*/

                let read = if bytes_to_read > 0 {
                    match socket.read(&mut read_info.reading_buffer[currently_read..]) {
                        Ok(0) => {
                            // Connection closed
                            warn!("Connection closed while reading body bytes to read: {},  currently read: {}", bytes_to_read, currently_read);

                            return Ok(ConnectionReadWork::ConnectionBroken(
                                currently_read,
                                bytes_to_read,
                            ));
                        }
                        Ok(n) => {
                            // We still have more to read
                            n
                        }
                        Err(err) if would_block(&err) => break,
                        Err(err) if interrupted(&err) => continue,
                        Err(err) => {
                            return Err(err.into());
                        }
                    }
                } else {
                    // Only read if we need to read from the socket.
                    // If not, keep parsing the messages that are in the read buffer
                    0
                };

                if read >= bytes_to_read {
                    let header = read_info.current_header.take().unwrap();

                    let module = read_info.message_module.take().unwrap();

                    let message = read_info.reading_buffer.split_to(header.payload_length());

                    read_messages.push(WireMessage::from_parts(header, module, message.freeze())?);

                    read_info.read_bytes = read_info.reading_buffer.len();

                    // Reserve with Header length as we are going to read the header (of the next message) next
                    read_info.reading_buffer.reserve(Header::LENGTH);
                    read_info.reading_buffer.resize(Header::LENGTH, 0);
                } else {
                    read_info.read_bytes += read;
                }
            } else {
                // We have already read the module of the message, so
                // We are currently reading a message
                let currently_read = read_info.read_bytes;
                let bytes_to_read = size_of::<MessageModule>() - currently_read;

                /*trace!(
                    "Reading message module with {} bytes to read, currently read {}",
                    bytes_to_read,
                    currently_read
                );*/

                let read = if bytes_to_read > 0 {
                    match socket.read(&mut read_info.reading_buffer[currently_read..]) {
                        Ok(0) => {
                            // Connection closed
                            warn!("Connection closed while reading message module bytes to read: {},  currently read: {}", bytes_to_read, currently_read);

                            return Ok(ConnectionReadWork::ConnectionBroken(
                                currently_read,
                                bytes_to_read,
                            ));
                        }
                        Ok(n) => {
                            // We still have more to read
                            n
                        }
                        Err(err) if would_block(&err) => break,
                        Err(err) if interrupted(&err) => continue,
                        Err(err) => {
                            return Err(err.into());
                        }
                    }
                } else {
                    // Only read if we need to read from the socket.
                    // If not, keep parsing the messages that are in the read buffer
                    0
                };

                if read >= bytes_to_read {
                    //FIXME: FIX THIS RIGHT NOW
                    let msg_mod = match read_info.reading_buffer[0] {
                        0 => MessageModule::Reconfiguration,
                        1 => MessageModule::Protocol,
                        2 => MessageModule::StateProtocol,
                        3 => MessageModule::Application,
                        _ => unreachable!(),
                    };

                    let msg_mod_size = size_of::<MessageModule>();

                    read_info.message_module = Some(msg_mod);

                    if read >= bytes_to_read {
                        read_info.reading_buffer.advance(msg_mod_size);
                        read_info.read_bytes = read_info.reading_buffer.len();
                    } else {
                        read_info.reading_buffer.clear();
                        read_info.read_bytes = 0;
                    }

                    read_info.reading_buffer.reserve(header.payload_length());
                    read_info.reading_buffer.resize(header.payload_length(), 0);
                } else {
                    read_info.read_bytes += read;
                }
            }
        } else {
            // We are currently reading a header
            let currently_read_bytes = read_info.read_bytes;
            let bytes_to_read = Header::LENGTH - currently_read_bytes;

            let read = if bytes_to_read > 0 {
                match socket.read(&mut read_info.reading_buffer[currently_read_bytes..]) {
                    Ok(0) => {
                        // Connection closed
                        warn!("Connection closed while reading header bytes to read {}, current read bytes {}", bytes_to_read, currently_read_bytes);
                        return Ok(ConnectionReadWork::ConnectionBroken(
                            currently_read_bytes,
                            bytes_to_read,
                        ));
                    }
                    Ok(n) => {
                        // We still have to more to read
                        n
                    }
                    Err(err) if would_block(&err) => break,
                    Err(err) if interrupted(&err) => continue,
                    Err(err) => {
                        return Err(err.into());
                    }
                }
            } else {
                // Only read if we need to read from the socket. (As we are missing bytes)
                // If not, keep parsing the messages that are in the read buffer
                0
            };

            if read >= bytes_to_read {
                let header = Header::deserialize_from(&read_info.reading_buffer[..Header::LENGTH])?;

                read_info.current_header = Some(header);

                if read > bytes_to_read {
                    //TODO: This will never happen since our buffer is HEADER::LENGTH sized

                    // We have read more than we should for the current message,
                    // so we can't clear the buffer
                    read_info.reading_buffer.advance(Header::LENGTH);
                    read_info.read_bytes = read_info.reading_buffer.len();
                } else {
                    // We have read the header
                    read_info.reading_buffer.clear();
                    read_info.read_bytes = 0;
                }

                read_info.reading_buffer.reserve(size_of::<MessageModule>());
                read_info
                    .reading_buffer
                    .resize(size_of::<MessageModule>(), 0);
            } else {
                read_info.read_bytes += read;
            }
        }
    }

    if !read_messages.is_empty() {
        Ok(ConnectionReadWork::WorkingAndReceived(read_messages))
    } else {
        Ok(ConnectionReadWork::Working)
    }
}

pub(crate) fn would_block(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

pub(crate) fn interrupted(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Interrupted
}

impl ReadingBuffer {
    pub fn init() -> Self {
        Self {
            read_bytes: 0,
            current_header: None,
            message_module: None,
            reading_buffer: BytesMut::with_capacity(Header::LENGTH),
        }
    }

    pub fn init_with_size(size: usize) -> Self {
        let mut read_buf = BytesMut::with_capacity(size);

        read_buf.resize(size, 0);

        Self {
            read_bytes: 0,
            current_header: None,
            message_module: None,
            reading_buffer: read_buf,
        }
    }
}

impl WritingBuffer {
    pub fn init_from_message(message: WireMessage) -> Result<Self, WritingBufferError> {
        let (header, module, payload) = message.into_inner();

        let mut header_bytes = BytesMut::with_capacity(Header::LENGTH);

        header_bytes.resize(Header::LENGTH, 0);

        header.serialize_into(&mut header_bytes[..Header::LENGTH])?;

        let mut mod_bytes = BytesMut::with_capacity(size_of::<MessageModule>());

        mod_bytes.resize(size_of::<MessageModule>(), 0);

        bincode::serde::encode_into_slice(module, &mut mod_bytes, bincode::config::standard())?;

        Ok(Self {
            written_bytes: 0,
            current_header: Some(header_bytes.freeze()),
            message_module: Some(mod_bytes.freeze()),
            current_message: payload,
        })
    }
}

pub fn initialize_send_channel(
    peer: NodeId,
) -> (ChannelSyncTx<ConnMessage>, ChannelSyncRx<ConnMessage>) {
    channel::sync::new_bounded_sync(
        2048,
        Some(format!("Network Peer Send Message {peer:?}").as_str()),
    )
}

#[derive(Error, Debug)]
pub enum ReadMessageError {
    #[error("Failed to read message due to IO Error {0}")]
    IoError(#[from] io::Error),
    #[error("Message creation error {0}")]
    MessageErrors(#[from] MessageErrors),
}

#[derive(Error, Debug)]
pub enum WritingBufferError {
    #[error("Failed to encode error due to {0:?}")]
    HeaderEncodeError(#[from] MessageErrors),
    #[error("Failed to encode message due to {0:?}")]
    MessageEncodeError(#[from] EncodeError),
}
