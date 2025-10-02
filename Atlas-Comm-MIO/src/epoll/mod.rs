mod epoll_worker;

use crate::conn_util::{ReadingBuffer, WritingBuffer};
use crate::connections::{ByteMessageSendStub, Connections, PeerConn};
use crate::epoll::epoll_worker::EpollWorker;
use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::channel::SendError;
use atlas_common::node_id::NodeId;
use atlas_common::socket::MioSocket;
use atlas_communication::byte_stub::{NodeIncomingStub, NodeStubController};
use atlas_communication::reconfiguration::NetworkInformationProvider;
use mio::Token;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tracing::error;

pub type EpollWorkerId = u32;

pub const DEFAULT_WORK_CHANNEL: usize = 128;

pub(crate) enum EpollWorkerMessage<CN> {
    NewConnection(Box<NewConnection<CN>>),
    CloseConnection(Token),
}

pub(crate) fn init_worker_group_handle<CN>(
    worker_count: u32,
) -> (
    EpollWorkerGroupHandle<CN>,
    Vec<ChannelSyncRx<EpollWorkerMessage<CN>>>,
) {
    let mut workers = Vec::with_capacity(worker_count as usize);

    let mut receivers = Vec::with_capacity(worker_count as usize);

    for _ in 0..worker_count {
        let (tx, rx) =
            channel::sync::new_bounded_sync(DEFAULT_WORK_CHANNEL, Some("Worker Group Handle"));

        workers.push(tx);
        receivers.push(rx);
    }

    (
        EpollWorkerGroupHandle {
            workers,
            round_robin: AtomicUsize::new(0),
        },
        receivers,
    )
}

pub(crate) fn initialize_worker_group<NI, CN, CNP>(
    connections: Arc<Connections<NI, CN, CNP>>,
    receivers: Vec<ChannelSyncRx<EpollWorkerMessage<CN>>>,
) -> Result<(), io::Error>
where
    NI: NetworkInformationProvider + 'static,
    CN: NodeIncomingStub + 'static,
    CNP: NodeStubController<ByteMessageSendStub, CN> + 'static,
{
    for (worker_id, rx) in receivers.into_iter().enumerate() {
        let worker = EpollWorker::new(worker_id as u32, connections.clone(), rx)?;

        std::thread::Builder::new()
            .name(format!("Epoll Worker {worker_id}"))
            .spawn(move || {
                if let Err(err) = worker.epoll_worker_loop() {
                    error!("Epoll worker {worker_id} failed with error: {err:?}");
                }
            })
            .expect("Failed to launch worker thread");
    }

    Ok(())
}

/// A handle to the worker group that handles the epoll events
/// Allows us to register new connections to the epoll workers
pub struct EpollWorkerGroupHandle<CN> {
    workers: Vec<ChannelSyncTx<EpollWorkerMessage<CN>>>,
    round_robin: AtomicUsize,
}

/// The object detailing the information about a new connection
pub(crate) struct NewConnection<CN> {
    conn_id: u32,
    peer_id: NodeId,
    my_id: NodeId,
    socket: MioSocket,
    reading_info: ReadingBuffer,
    writing_info: Option<WritingBuffer>,
    peer_conn: Arc<PeerConn<CN>>,
}

impl<CN> EpollWorkerGroupHandle<CN> {
    /// Assigns a socket to any given worker
    pub(super) fn assign_socket_to_worker(
        &self,
        conn_details: NewConnection<CN>,
    ) -> Result<(), WorkerError> {
        let round_robin = self.round_robin.fetch_add(1, Ordering::Relaxed);

        let epoll_worker = round_robin % self.workers.len();

        let worker = self.workers.get(epoll_worker).ok_or(
            WorkerError::FailedToAllocateWorkerForConnection(
                epoll_worker,
                conn_details.peer_id,
                conn_details.conn_id,
            ),
        )?;

        let conn_id = conn_details.conn_id;

        worker
            .send(EpollWorkerMessage::NewConnection(Box::new(conn_details)))
            .map_err(|err| WorkerError::SendInitMessageFailed(err, epoll_worker, conn_id))?;

        Ok(())
    }

    /// Order a disconnection of a given connection from a worker
    pub fn disconnect_connection_from_worker(
        &self,
        epoll_worker: EpollWorkerId,
        conn_id: Token,
    ) -> Result<(), WorkerError> {
        let worker = self.workers.get(epoll_worker as usize).ok_or(
            WorkerError::FailedToGetWorkerForConnection(epoll_worker, conn_id),
        )?;

        worker
            .send(EpollWorkerMessage::CloseConnection(conn_id))
            .map_err(|err| WorkerError::SendCloseMessageFailed(err, epoll_worker, conn_id))?;

        Ok(())
    }
}

impl<CN> Clone for EpollWorkerGroupHandle<CN> {
    fn clone(&self) -> Self {
        Self {
            workers: self.workers.clone(),
            round_robin: AtomicUsize::new(0),
        }
    }
}

impl<CN> NewConnection<CN> {
    pub(crate) fn new(
        conn_id: u32,
        peer_id: NodeId,
        my_id: NodeId,
        socket: MioSocket,
        reading_info: ReadingBuffer,
        writing_info: Option<WritingBuffer>,
        peer_conn: Arc<PeerConn<CN>>,
    ) -> Self {
        Self {
            conn_id,
            peer_id,
            my_id,
            socket,
            reading_info,
            writing_info,
            peer_conn,
        }
    }
}

#[derive(Error, Debug)]
pub enum WorkerError {
    #[error("Failed to allocate worker (planned {0:?}) for connection {2} with node {1:?}")]
    FailedToAllocateWorkerForConnection(usize, NodeId, u32),
    #[error("Failed to get the corresponding worker for connection {0:?}, {1:?}")]
    FailedToGetWorkerForConnection(EpollWorkerId, Token),
    #[error("Failed to dispatch message to the channel. {0} for epoll worker {1} and token {2:?}")]
    SendCloseMessageFailed(SendError, EpollWorkerId, Token),
    #[error("Failed to dispatch init conn message to the channel. {0} for epoll worker {1} and conn id {2}")]
    SendInitMessageFailed(SendError, usize, u32),
}
