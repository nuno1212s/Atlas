use std::io;
use std::io::{Read, Write};
use bytes::{Buf, Bytes, BytesMut};
use log::{debug, trace};
use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::{Error, ErrorKind};
use atlas_common::socket::MioSocket;
use crate::message::{Header, WireMessage};
use crate::mio_tcp::connections::{NetworkSerializedMessage, SEND_QUEUE_SIZE};

/// The reading buffer for a connection
pub(super) struct ReadingBuffer {
    pub(super) read_bytes: usize,
    pub(super)current_header: Option<Header>,
    pub(super) read_buffer: BytesMut,
}

/// The writing buffer for a TCP connection
pub(super) struct WritingBuffer {
    written_bytes: usize,
    current_header: Option<Bytes>,
    current_message: Bytes,
}

/// Result of trying to write until block in a socket
pub(super) enum ConnectionWriteWork {
    /// The connection is broken
    ConnectionBroken,
    /// The connection is working
    Working,
    /// The requested work is done
    Done,
}

/// The result of trying to read from a connection until block
pub(super) enum ConnectionReadWork {
    ConnectionBroken,

    Working,

    WorkingAndReceived(Vec<(Header, BytesMut)>),

    ReceivedAndDone(Vec<(Header, BytesMut)>),
}

pub(super) fn try_write_until_block(socket: &mut MioSocket, writing_buffer: &mut WritingBuffer) -> atlas_common::error::Result<ConnectionWriteWork> {
    loop {
        if let Some(header) = writing_buffer.current_header.as_ref() {
            match socket.write(&header[writing_buffer.written_bytes..]) {
                Ok(0) => return Ok(ConnectionWriteWork::ConnectionBroken),
                Ok(n) => {
                    // We have successfully written n bytes
                    if n + writing_buffer.written_bytes < header.len() {
                        writing_buffer.written_bytes += n;

                        continue;
                    } else {
                        // It will always write atmost header.len() bytes, since
                        // That's the length of the buffer
                        writing_buffer.written_bytes = 0;
                        writing_buffer.current_header = None;
                    }
                }
                Err(err) if would_block(&err) => {
                    trace!("Would block writing header");
                    break;
                }
                Err(err) if interrupted(&err) => continue,
                Err(err) => { return Err(Error::wrapped(ErrorKind::Communication, err)); }
            }
        } else {
            match socket.write(&writing_buffer.current_message[writing_buffer.written_bytes..]) {
                Ok(0) => return Ok(ConnectionWriteWork::ConnectionBroken),
                Ok(n) => {
                    // We have successfully written n bytes
                    if n + writing_buffer.written_bytes < writing_buffer.current_message.len() {
                        writing_buffer.written_bytes += n;

                        continue;
                    } else {
                        // We have written all that we have to write.
                        return Ok(ConnectionWriteWork::Done);
                    }
                }
                Err(err) if would_block(&err) => {
                    trace!("Would block writing body");
                    break;
                }
                Err(err) if interrupted(&err) => continue,
                Err(err) => { return Err(Error::wrapped(ErrorKind::Communication, err)); }
            }
        }
    }

    Ok(ConnectionWriteWork::Working)
}

pub(super) fn read_until_block(socket: &mut MioSocket, read_info: &mut ReadingBuffer) -> atlas_common::error::Result<ConnectionReadWork> {
    let mut read_messages = Vec::new();

    loop {
        if let Some(header) = &read_info.current_header {
            // We are currently reading a message
            let currently_read = read_info.read_bytes;
            let bytes_to_read = header.payload_length() - currently_read;

            let read = if bytes_to_read > 0 {
                match socket.read(&mut read_info.read_buffer[currently_read..]) {
                    Ok(0) => {
                        // Connection closed
                        debug!("Connection closed while reading body bytes to read: {},  currently read: {}", bytes_to_read, currently_read);
                        return Ok(ConnectionReadWork::ConnectionBroken);
                    }
                    Ok(n) => {
                        // We still have more to read
                        n
                    }
                    Err(err) if would_block(&err) => break,
                    Err(err) if interrupted(&err) => continue,
                    Err(err) => { return Err(Error::wrapped(ErrorKind::Communication, err)); }
                }
            } else {
                // Only read if we need to read from the socket.
                // If not, keep parsing the messages that are in the read buffer
                0
            };

            if read >= bytes_to_read {
                let header = std::mem::replace(&mut read_info.current_header, None).unwrap();

                let message = read_info.read_buffer.split_to(header.payload_length());

                read_messages.push((header, message));

                read_info.read_bytes = read_info.read_buffer.len();

                read_info.read_buffer.reserve(Header::LENGTH);
                read_info.read_buffer.resize(Header::LENGTH, 0);
            } else {
                read_info.read_bytes += read;
            }
        } else {
            // We are currently reading a header
            let currently_read_bytes = read_info.read_bytes;
            let bytes_to_read = Header::LENGTH - currently_read_bytes;

            let n = if bytes_to_read > 0 {
                trace!("Reading message header with {} left to read", bytes_to_read);

                match socket.read(&mut read_info.read_buffer[currently_read_bytes..]) {
                    Ok(0) => {
                        // Connection closed
                        debug!("Connection closed while reading header bytes to read {}, current read bytes {}", bytes_to_read, currently_read_bytes);
                        return Ok(ConnectionReadWork::ConnectionBroken);
                    }
                    Ok(n) => {
                        // We still have to more to read
                        n
                    }
                    Err(err) if would_block(&err) => break,
                    Err(err) if interrupted(&err) => continue,
                    Err(err) => { return Err(Error::wrapped(ErrorKind::Communication, err)); }
                }
            } else {
                // Only read if we need to read from the socket. (As we are missing bytes)
                // If not, keep parsing the messages that are in the read buffer
                0
            };

            if n >= bytes_to_read {
                let header = Header::deserialize_from(&read_info.read_buffer[..Header::LENGTH])?;

                *(&mut read_info.current_header) = Some(header);

                if n > bytes_to_read {
                    //TODO: This will never happen since our buffer is HEADER::LENGTH sized

                    // We have read more than we should for the current message,
                    // so we can't clear the buffer
                    read_info.read_buffer.advance(Header::LENGTH);

                    read_info.read_bytes = read_info.read_buffer.len();

                    read_info.read_buffer.reserve(header.payload_length());
                    read_info.read_buffer.resize(header.payload_length(), 0);
                } else {
                    // We have read the header
                    read_info.read_buffer.clear();
                    read_info.read_buffer.reserve(header.payload_length());
                    read_info.read_buffer.resize(header.payload_length(), 0);
                    read_info.read_bytes = 0;
                }
            } else {
                read_info.read_bytes += n;
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
            read_buffer: BytesMut::with_capacity(Header::LENGTH),
        }
    }

    pub fn init_with_size(size: usize) -> Self {
        let mut read_buf = BytesMut::with_capacity(size);

        read_buf.resize(size, 0);

        Self {
            read_bytes: 0,
            current_header: None,
            read_buffer: read_buf,
        }
    }
}

impl WritingBuffer {
    pub fn init_from_message(message: WireMessage) -> atlas_common::error::Result<Self> {
        let (header, payload) = message.into_inner();

        let mut header_bytes = BytesMut::with_capacity(Header::LENGTH);

        header_bytes.resize(Header::LENGTH, 0);

        header.serialize_into(&mut header_bytes[..Header::LENGTH])?;

        let header_bytes = header_bytes.freeze();

        Ok(Self {
            written_bytes: 0,
            current_header: Some(header_bytes),
            current_message: payload,
        })
    }
}

pub fn initialize_send_channel() -> (ChannelSyncTx<NetworkSerializedMessage>, ChannelSyncRx<NetworkSerializedMessage>) {
    channel::new_bounded_sync(SEND_QUEUE_SIZE)
}
