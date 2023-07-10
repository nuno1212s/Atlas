use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use log::error;
use mio::{Token};
use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::socket::MioSocket;
use crate::mio_tcp::connections::{Connections, PeerConnection};
use crate::mio_tcp::connections::epoll_group::epoll_workers::EpollWorker;
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::Serializable;

pub mod epoll_workers;

pub type EpollWorkerId = u32;

// This will just handle creating and deleting connections so it can be small
pub const DEFAULT_WORKER_CHANNEL: usize = 128;

pub fn init_worker_group_handle<NI, RM, PM>(worker_count: u32) -> (EpollWorkerGroupHandle<RM, PM>, Vec<ChannelSyncRx<EpollWorkerMessage<RM, PM>>>)
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    let mut workers = Vec::with_capacity(worker_count as usize);

    let mut receivers = Vec::with_capacity(worker_count as usize);

    for _ in 0..worker_count {
        let (tx, rx) = channel::new_bounded_sync(DEFAULT_WORKER_CHANNEL);

        workers.push(tx);
        receivers.push(rx);
    }

    (EpollWorkerGroupHandle {
        workers,
        round_robin: AtomicUsize::new(0),
    }, receivers)
}

pub fn initialize_worker_group<NI, RM, PM>(connections: Arc<Connections<NI, RM, PM>>, receivers: Vec<ChannelSyncRx<EpollWorkerMessage<RM, PM>>>) -> Result<()>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    for (worker_id, rx) in receivers.into_iter().enumerate() {
        let worker = EpollWorker::new(worker_id as u32, connections.clone(), rx)?;

        std::thread::Builder::new().name(format!("Epoll Worker {}", worker_id))
            .spawn(move || {
                if let Err(err) = worker.epoll_worker_loop() {
                    error!("Epoll worker {} failed with error: {:?}", worker_id,err);
                }
            }).expect("Failed to launch worker thread");
    }

    Ok(())
}

/// A handle to the worker group that handles the epoll events
/// Allows us to register new connections to the epoll workers
pub struct EpollWorkerGroupHandle<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    workers: Vec<ChannelSyncTx<EpollWorkerMessage<RM, PM>>>,
    round_robin: AtomicUsize,
}

pub struct NewConnection<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    conn_id: u32,
    peer_id: NodeId,
    my_id: NodeId,
    socket: MioSocket,
    peer_conn: Arc<PeerConnection<RM, PM>>,
}

pub enum EpollWorkerMessage<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    NewConnection(NewConnection<RM, PM>),
    CloseConnection(Token),
}

impl<RM, PM> EpollWorkerGroupHandle<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    /// Assigns a socket to any given worker
    pub fn assign_socket_to_worker(&self, conn_details: NewConnection<RM, PM>) -> atlas_common::error::Result<()> {
        let round_robin = self.round_robin.fetch_add(1, Ordering::Relaxed);

        let worker = self.workers.get(round_robin % self.workers.len())
            .ok_or(Error::simple_with_msg(ErrorKind::Communication, "Failed to get worker for connection?"))?;

        worker.send(EpollWorkerMessage::NewConnection(conn_details)).wrapped(ErrorKind::Communication)?;

        Ok(())
    }

    /// Order a disconnection of a given connection from a worker
    pub fn disconnect_connection_from_worker(&self, epoll_worker: EpollWorkerId, conn_id: Token) -> atlas_common::error::Result<()> {
        let worker = self.workers.get(epoll_worker as usize)
            .ok_or(Error::simple_with_msg(ErrorKind::Communication, "Failed to get worker for connection?"))?;

        worker.send(EpollWorkerMessage::CloseConnection(conn_id)).wrapped(ErrorKind::Communication)?;

        Ok(())
    }
}

impl<RM, PM> Clone for EpollWorkerGroupHandle<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    fn clone(&self) -> Self {
        Self {
            workers: self.workers.clone(),
            round_robin: AtomicUsize::new(0),
        }
    }
}
