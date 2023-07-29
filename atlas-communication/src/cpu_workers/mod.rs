use std::sync::Arc;
use std::time::Instant;
use bytes::{Bytes, BytesMut};
use log::error;
use atlas_common::channel::{new_oneshot_channel, OneShotRx};
use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::{channel, threadpool};
use atlas_metrics::metrics::metric_duration;
use crate::client_pooling::ConnectedPeer;
use crate::message::{Header, NetworkMessage, NetworkMessageKind, StoredMessage};
use crate::metric::{COMM_DESERIALIZE_VERIFY_TIME_ID, COMM_SERIALIZE_SIGN_TIME_ID, THREADPOOL_PASS_TIME_ID};
use crate::reconfiguration_node::ReconfigurationMessageHandler;
use crate::serialize;
use crate::serialize::Serializable;

//TODO: Statistics

/// Serialize and digest a given message.
/// Returns a OneShotRx that can be recv() or awaited depending on whether it's being used
/// in synchronous or asynchronous workloads.
pub(crate) fn serialize_digest_message<RM, PM>(message: NetworkMessageKind<RM, PM>)
                                               -> OneShotRx<Result<(Bytes, Digest)>>
    where RM: Serializable + 'static, PM: Serializable + 'static {
    let (tx, rx) = new_oneshot_channel();

    let start = Instant::now();

    threadpool::execute(move || {
        metric_duration(THREADPOOL_PASS_TIME_ID, start.elapsed());

        // serialize
        let _ = tx.send(serialize_digest_no_threadpool(&message));
    });

    rx
}


/// Serialize and digest a request in the threadpool but don't actually send it. Instead, return the
/// the message back to us as well so we can do what ever we want with it.
pub(crate) fn serialize_digest_threadpool_return_msg<RM, PM>(message: NetworkMessageKind<RM, PM>)
                                                             -> OneShotRx<(NetworkMessageKind<RM, PM>, Result<(Bytes, Digest)>)>
    where RM: Serializable + 'static, PM: Serializable + 'static {
    let (tx, rx) = channel::new_oneshot_channel();

    threadpool::execute(move || {
        let result = serialize_digest_no_threadpool(&message);

        let _ = tx.send((message, result));
    });

    rx
}

/// Serialize and digest a given message, but without sending the job to the threadpool
/// Useful if we want to re-utilize this for other things
pub(crate) fn serialize_digest_no_threadpool<RM, PM>(message: &NetworkMessageKind<RM, PM>)
                                                     -> Result<(Bytes, Digest)>
    where RM: Serializable, PM: Serializable {
    let start = Instant::now();

    // TODO: Use a memory pool here
    let mut buf = Vec::with_capacity(512);

    let digest = match serialize::serialize_digest::<Vec<u8>, RM, PM>(message, &mut buf) {
        Ok(dig) => dig,
        Err(err) => {
            error!("Failed to serialize message {:?}. Message is {:?}", err, message);

            panic!("Failed to serialize message {:?}", err);
        }
    };

    let buf = Bytes::from(buf);

    metric_duration(COMM_SERIALIZE_SIGN_TIME_ID, start.elapsed());

    Ok((buf, digest))
}

/// Deserialize a given message without using the threadpool.
pub(crate) fn deserialize_message_no_threadpool<RM, PM>(header: Header, payload: BytesMut) -> Result<(NetworkMessageKind<RM, PM>, BytesMut)>
    where RM: Serializable + 'static, PM: Serializable + 'static {
    let start = Instant::now();

    //TODO: Verify signatures

    // deserialize payload
    let message = match serialize::deserialize_message::<&[u8], RM, PM>(&payload[..header.payload_length()]) {
        Ok(m) => m,
        Err(err) => {
            // errors deserializing -> faulty connection;
            // drop this socket
            error!("{:?} // Failed to deserialize message {:?}", header.to(), err);

            return Err(Error::wrapped(ErrorKind::CommunicationSerialize, err));
        }
    };

    metric_duration(COMM_DESERIALIZE_VERIFY_TIME_ID, start.elapsed());

    Ok((message, payload))
}

/// Deserialize the message that is contained in the given payload.
/// Returns a OneShotRx that can be recv() or awaited depending on whether it's being used
/// in synchronous or asynchronous workloads.
/// Also returns the bytes so we can re utilize them for our next operation.
pub(crate) fn deserialize_message<RM, PM>(header: Header, payload: BytesMut)
                                          -> OneShotRx<Result<(NetworkMessageKind<RM, PM>, BytesMut)>>
    where RM: Serializable + 'static, PM: Serializable + 'static {
    let (tx, rx) = new_oneshot_channel();

    let start = Instant::now();

    threadpool::execute(move || {
        metric_duration(THREADPOOL_PASS_TIME_ID, start.elapsed());

        let _ = tx.send(deserialize_message_no_threadpool(header, payload));
    });

    rx
}

pub(crate) fn deserialize_and_push_reconf_message<RM, PM>(header: Header, payload: BytesMut, reconf_handle: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>)
    where RM: Serializable + 'static, PM: Serializable + 'static {
    let start = Instant::now();

    threadpool::execute(move || {
        metric_duration(THREADPOOL_PASS_TIME_ID, start.elapsed());

        let (message, _) = deserialize_message_no_threadpool::<RM, PM>(header.clone(), payload).unwrap();

        match message {
            NetworkMessageKind::ReconfigurationMessage(reconf) => {
                reconf_handle.push_request(StoredMessage::new(header, reconf.into())).unwrap();
            }
            _ => unreachable!("Received a non-reconfiguration message while we still have no extra information about the node.")
        }
    });
}

pub(crate) fn deserialize_and_push_message<RM, PM>(header: Header, payload: BytesMut,
                                                   connection: Arc<ConnectedPeer<StoredMessage<PM::Message>>>,
                                                   reconf_handle: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>)
    where RM: Serializable + 'static, PM: Serializable + 'static {
    let start = Instant::now();

    threadpool::execute(move || {
        metric_duration(THREADPOOL_PASS_TIME_ID, start.elapsed());

        let (message, _) = deserialize_message_no_threadpool::<RM, PM>(header.clone(), payload).unwrap();

        match message {
            NetworkMessageKind::ReconfigurationMessage(reconf) => {
                reconf_handle.push_request(StoredMessage::new(header, reconf.into())).unwrap();
            }
            NetworkMessageKind::Ping(_) => {
                todo!("MIO does not currently use this (and the only one that uses this function is MIO so....)")
            }
            NetworkMessageKind::System(sys_msg) => {
                connection.push_request(StoredMessage::new(header, sys_msg.into())).unwrap();
            }
        }
    });
}