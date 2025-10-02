#![allow(dead_code)]

use crate::config::ClientPoolConfig;
use crate::message::StoredMessage;
use crate::metric::{
    CLIENT_POOL_COLLECT_TIME_ID, CLIENT_POOL_SLEEP_TIME_ID, RQ_CLIENT_POOL_TIME_SPENT_ID,
};
use crate::stub::BatchedModuleIncomingStub;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::channel::TryRecvError;
use atlas_common::node_id::NodeId;
use atlas_common::Err;

use atlas_metrics::metrics::metric_duration;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, error, info};

pub struct PooledStubOutput<M>(ChannelSyncRx<ClientRqBatchOutput<M>>);

impl<M> Clone for PooledStubOutput<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M> From<ChannelSyncRx<ClientRqBatchOutput<M>>> for PooledStubOutput<M> {
    fn from(value: ChannelSyncRx<ClientRqBatchOutput<M>>) -> Self {
        Self(value)
    }
}

pub type ClientRqBatchOutput<T> = Vec<T>;

pub type ClientPeer<T> = Arc<ConnectedClientPeer<T>>;

pub struct ConnectedClientPeer<T> {
    peer_id: NodeId,
    queue: Mutex<Vec<(T, Instant)>>,
    disconnect: AtomicBool,
}

///Client pool design, where each pool contains a number of clients (Maximum of BATCH_SIZE clients
/// per pool). This is to prevent starvation for each client, as when we are performing
/// the fair collection of requests from the clients, if there are more clients than batch size
/// then we will get very unfair distribution of requests
///
/// This will push Vecs of T types into the ChannelTx provided
/// The type T is not wrapped in any other way
/// no socket handling is done here
/// This is just built on top of the actual per client connection socket stuff and each socket
/// should push items into its own ConnectedPeer instance
pub struct ConnectedPeersGroup<T: Send> {
    own_id: NodeId,
    //We can use mutexes here since there will only be concurrency on client connections and dcs
    //And since each client has his own reference to push data to, this only needs to be accessed by the thread
    //That's producing the batches and the threads of clients connecting and disconnecting
    client_pools: Mutex<BTreeMap<usize, Arc<ConnectedPeersPool<T>>>>,
    batch_transmission: ChannelSyncTx<ClientRqBatchOutput<T>>,
    batch_reception: ChannelSyncRx<ClientRqBatchOutput<T>>,
    per_client_cache: usize,
    //What batch size should we target for each batch (there is no set limit on requests,
    //Just a hint on when it should move on)
    batch_target_size: usize,
    //How much time can be spent gathering batches
    batch_timeout_micros: u64,
    //How much time should the thread sleep in between batch collection
    batch_sleep_micros: u64,
    clients_per_pool: usize,
    //Counter used to keep track of the created pools
    pool_id_counter: AtomicUsize,
}

pub struct ConnectedPeersPool<T: Send> {
    pool_id: usize,
    //We can use mutexes here since there will only be concurrency on client connections and dcs
    //And since each client has his own reference to push data to, this only needs to be accessed by the thread
    //That's producing the batches and the threads of clients connecting and disconnecting
    connected_clients: Mutex<Vec<ClientPeer<T>>>,
    batch_transmission: ChannelSyncTx<ClientRqBatchOutput<T>>,
    finish_execution: AtomicBool,
    owner: Arc<ConnectedPeersGroup<T>>,
    batch_size: usize,
    client_limit: usize,
    batch_timeout_micros: u64,
    batch_sleep_micros: u64,
}

impl<T> ConnectedPeersGroup<T>
where
    T: Send + 'static,
{
    pub fn new(
        client_pool_config: ClientPoolConfig,
        batch_transmission: ChannelSyncTx<ClientRqBatchOutput<T>>,
        batch_reception: ChannelSyncRx<ClientRqBatchOutput<T>>,
        own_id: NodeId,
    ) -> Arc<Self> {
        Arc::new(Self {
            own_id,
            client_pools: Mutex::new(BTreeMap::new()),
            per_client_cache: client_pool_config.per_client_bound(),
            batch_timeout_micros: client_pool_config.batch_timeout_micros(),
            batch_sleep_micros: client_pool_config.batch_sleep_micros(),
            batch_target_size: client_pool_config.batch_limit(),
            batch_transmission,
            batch_reception,
            clients_per_pool: client_pool_config.clients_per_pool(),
            pool_id_counter: AtomicUsize::new(0),
        })
    }

    fn get_pool_id(&self) -> atlas_common::error::Result<usize> {
        const IT_LIMIT: usize = 100;

        let mut it_count = 0;

        let pool_id = loop {
            let pool_id = self.pool_id_counter.fetch_add(1, Ordering::Relaxed);

            it_count += 1;

            if it_count >= IT_LIMIT {
                return Err!(ClientPoolError::FailedToAllocateClientPoolID);
            }

            if !self.client_pools.lock().unwrap().contains_key(&pool_id) {
                break pool_id;
            }
        };

        Ok(pool_id)
    }

    pub fn init_client(self: &Arc<Self>, peer_id: NodeId) -> ClientPeer<T> {
        let connected_client = Arc::new(ConnectedClientPeer {
            peer_id,
            queue: Mutex::new(Vec::with_capacity(self.per_client_cache)),
            disconnect: AtomicBool::new(false),
        });

        let mut clone_queue = connected_client.clone();

        {
            let guard = self.client_pools.lock().unwrap();

            for pool in guard.values() {
                match pool.attempt_to_add(clone_queue) {
                    Ok(_) => {
                        return connected_client;
                    }
                    Err(queue) => {
                        clone_queue = queue;
                    }
                }
            }
        }

        //In the case all the pools are already full, allocate a new pool
        let pool_id = match self.get_pool_id() {
            Ok(pool_id) => pool_id,
            Err(_err) => {
                panic!("Failed to allocate new pool id");
            }
        };

        let pool = ConnectedPeersPool::new(
            pool_id,
            self.batch_target_size,
            self.batch_transmission.clone(),
            Arc::clone(self),
            self.clients_per_pool,
            self.batch_timeout_micros,
            self.batch_sleep_micros,
        );

        match pool.attempt_to_add(clone_queue) {
            Ok(_) => {}
            Err(_e) => {
                panic!("Failed to add pool to pool list.")
            }
        };
        {
            let mut guard = self.client_pools.lock().unwrap();

            let pool_clone = pool.clone();

            guard.insert(pool.pool_id, pool);

            let id = guard.len();

            pool_clone.start(id as u32);
        }

        connected_client
    }

    fn del_pool(&self, pool_id: usize) -> bool {
        debug!("{:?} // DELETING POOL {}", self.own_id, pool_id);

        match self.client_pools.lock().unwrap().remove(&pool_id) {
            None => false,
            Some(pool) => {
                pool.shutdown();
                info!("{:?} // DELETED POOL {}", self.own_id, pool_id);

                true
            }
        }
    }
}

impl<T> ConnectedPeersPool<T>
where
    T: Send + 'static,
{
    //We mark the owner as static since if the pool is active then
    //The owner also has to be active
    pub fn new(
        pool_id: usize,
        batch_size: usize,
        batch_transmission: ChannelSyncTx<ClientRqBatchOutput<T>>,
        owner: Arc<ConnectedPeersGroup<T>>,
        client_per_pool: usize,
        batch_timeout_micros: u64,
        batch_sleep_micros: u64,
    ) -> Arc<Self> {
        let result = Self {
            pool_id,
            connected_clients: Mutex::new(Vec::new()),
            batch_size,
            batch_transmission,
            batch_timeout_micros,
            batch_sleep_micros,
            client_limit: client_per_pool,
            finish_execution: AtomicBool::new(false),
            owner,
        };

        Arc::new(result)
    }

    pub fn start(self: Arc<Self>, pool_id: u32) {
        //Spawn the thread that will collect client requests
        //and then send the batches to the channel.
        std::thread::Builder::new()
            .name(format!("Peer pool collector thread #{pool_id}"))
            .spawn(move || {
                loop {
                    if self.finish_execution.load(Ordering::Relaxed) {
                        break;
                    }

                    let vec = match self.collect_requests(self.batch_size, &self.owner) {
                        Ok(vec) => vec,
                        Err(err) => {
                            match err {
                                ClientPoolError::ClosePool => {
                                    //The pool is empty, so to save CPU, delete it
                                    self.owner.del_pool(self.pool_id);

                                    self.finish_execution.store(true, Ordering::SeqCst);

                                    break;
                                }
                                _ => {
                                    break;
                                }
                            }
                        }
                    };

                    if !vec.is_empty() {
                        self.batch_transmission
                            .send(vec)
                            .expect("Failed to send proposed batch");

                        // Sleep for a determined amount of time to allow clients to send requests
                        let three_quarters_sleep = (self.batch_sleep_micros / 4) * 3;
                        let five_quarters_sleep = (self.batch_sleep_micros / 4) * 5;

                        let sleep_micros =
                            fastrand::u64(three_quarters_sleep..=five_quarters_sleep);

                        let sleep_start_time = Instant::now();

                        std::thread::sleep(Duration::from_micros(sleep_micros));

                        metric_duration(CLIENT_POOL_SLEEP_TIME_ID, sleep_start_time.elapsed());
                    }

                    // backoff.spin();
                }
            })
            .unwrap();
    }

    pub fn attempt_to_add(&self, client: ClientPeer<T>) -> Result<(), ClientPeer<T>> {
        let mut guard = self.connected_clients.lock().unwrap();

        if guard.len() < self.client_limit {
            guard.push(client);

            return Ok(());
        }

        Err(client)
    }

    pub fn attempt_to_remove(&self, client_id: &NodeId) -> Result<bool, ()> {
        let mut guard = self.connected_clients.lock().unwrap();

        match guard
            .iter()
            .position(|client| client.client_id().eq(client_id))
        {
            None => Err(()),
            Some(position) => {
                guard.swap_remove(position);

                Ok(guard.is_empty())
            }
        }
    }

    pub fn collect_requests(
        &self,
        batch_target_size: usize,
        _owner: &Arc<ConnectedPeersGroup<T>>,
    ) -> Result<Vec<T>, ClientPoolError> {
        let start = Instant::now();

        let vec_size = std::cmp::max(batch_target_size, self.owner.per_client_cache);

        let mut batch = Vec::with_capacity(vec_size);

        let guard = self.connected_clients.lock().unwrap();

        let mut dced = Vec::new();

        let mut connected_peers = Vec::with_capacity(guard.len());

        if guard.is_empty() {
            return Err!(ClientPoolError::ClosePool);
        }

        for connected_peer in &*guard {
            connected_peers.push(Arc::clone(connected_peer));
        }

        drop(guard);

        let start_point = fastrand::usize(0..connected_peers.len());

        let start_time = Instant::now();

        let mut replacement_vec = Vec::with_capacity(self.owner.per_client_cache);

        let mut collected_requests_per_revolution = 0;

        for index in 0..usize::MAX {
            let client = &connected_peers[(start_point + index) % connected_peers.len()];

            if client.is_dc() {
                dced.push(client.client_id());

                //Assign the remaining slots to the next client
                continue;
            }

            //Collect all possible requests from each client
            let mut rqs_dumped = client.dump_requests(replacement_vec);

            collected_requests_per_revolution += rqs_dumped.len();

            batch.append(&mut rqs_dumped);

            //The previous vec is now the new vec of the next node
            replacement_vec = rqs_dumped;

            if index > 0 && index % connected_peers.len() == 0 {
                //We have done a full circle on the requests
                collected_requests_per_revolution = 0;

                if batch.len() >= batch_target_size {
                    //We only check on each complete revolution since if we didn't do that
                    //We could have a situation where a single client's requests were
                    //Enough to fill an entire batch, so the rest of the clients
                    //Wouldn't even be checked
                    break;
                } else {
                    let current_time = Instant::now();

                    let collection_time = current_time.duration_since(start_time).as_micros();

                    if collection_time >= self.batch_timeout_micros as u128 {
                        //Check if a given amount of time limit has passed, to prevent us getting
                        //Stuck while checking for requests
                        break;
                    }

                    if collected_requests_per_revolution == 0 {
                        let time_until_timeout = self.batch_timeout_micros - collection_time as u64;

                        std::thread::sleep(Duration::from_micros(time_until_timeout / 2));
                    } else {
                        std::thread::yield_now();
                    }
                }
            }
        }

        //This might cause some lag since it has to access the intmap, but
        //Should be fine as it will only happen on client dcs
        if !dced.is_empty() {
            let mut guard = self.connected_clients.lock().unwrap();

            for node in &dced {
                //This is O(n*c) but there isn't really a much better way to do it I guess
                let option = guard.iter().position(|x| x.client_id().0 == node.0);

                match option {
                    None => {
                        //The client was already removed from the guard
                    }
                    Some(option) => {
                        guard.swap_remove(option);
                    }
                }
            }

            //If the pool is empty, delete it
            let should_delete_pool = guard.is_empty();

            drop(guard);

            //owner.del_cached_clients(dced);
            //TODO: Delete the disconnected clients from the general connected clients

            if should_delete_pool {
                return Err!(ClientPoolError::ClosePool);
            }
        }

        metric_duration(CLIENT_POOL_COLLECT_TIME_ID, start.elapsed());

        let mut accumulated_duration = Duration::new(0, 0);

        let batch: Vec<T> = batch
            .into_iter()
            .map(|(vec, insert_time)| {
                accumulated_duration += insert_time.elapsed();

                vec
            })
            .collect();

        if !batch.is_empty() {
            let nanos = accumulated_duration.as_nanos() / batch.len() as u128;

            metric_duration(
                RQ_CLIENT_POOL_TIME_SPENT_ID,
                Duration::from_nanos(nanos as u64),
            );
        }

        Ok(batch)
    }

    pub fn shutdown(&self) {
        info!(
            "{:?} // Pool {} is shutting down",
            self.owner.own_id, self.pool_id
        );

        self.finish_execution.store(true, Ordering::Relaxed);
    }
}

impl<T> ConnectedClientPeer<T> {
    pub fn client_id(&self) -> NodeId {
        self.peer_id
    }
    pub fn is_dc(&self) -> bool {
        self.disconnect.load(Ordering::Relaxed)
    }

    pub fn disconnect(&self) {
        self.disconnect.store(true, Ordering::Relaxed)
    }

    pub fn dump_requests(&self, replacement_vec: Vec<(T, Instant)>) -> Vec<(T, Instant)> {
        let mut guard = self.queue.lock().unwrap();

        std::mem::replace(&mut *guard, replacement_vec)
    }

    pub fn push_request(&self, msg: T) -> atlas_common::error::Result<()> {
        let mut sender_guard = self.queue.lock().unwrap();

        if self.disconnect.load(Ordering::Relaxed) {
            error!(
                "Failed to send to client {:?} as he was already disconnected",
                self.peer_id
            );

            Err!(ClientPoolError::PooledConnectionClosed(self.peer_id))
        } else {
            sender_guard.push((msg, Instant::now()));

            Ok(())
        }
    }
}

#[derive(Error, Debug)]
pub enum ClientPoolError {
    #[error("This error is meant to be used to close the pool")]
    ClosePool,
    #[error("The unpooled connection is closed {0:?}")]
    UnpooledConnectionClosed(NodeId),
    #[error("The pooled connection is closed {0:?}")]
    PooledConnectionClosed(NodeId),
    #[error("Failed to allocate client pool ID")]
    FailedToAllocateClientPoolID,
    #[error("Failed to receive from clients as there are no clients connected")]
    NoClientsConnected,
    #[error("Failed to get client connection as we can't connect to other clients")]
    CannotConnectToClients,
}

#[derive(Error, Debug)]
pub enum SendPeerError {
    #[error("Attempted to push replica bound message, {0:?}")]
    AttemptToPushReplicaMessageToClientConn(NodeId),
    #[error("Attempted to push client bound message to another replica {0:?}")]
    AttemptToPushClientMessageToReplicaConn(NodeId),
}

impl<M> AsRef<ChannelSyncRx<Vec<StoredMessage<M>>>> for PooledStubOutput<StoredMessage<M>>
where
    M: Send + Clone,
{
    fn as_ref(&self) -> &ChannelSyncRx<Vec<StoredMessage<M>>> {
        &self.0
    }
}

impl<M> BatchedModuleIncomingStub<M> for PooledStubOutput<StoredMessage<M>>
where
    M: Send + Clone,
{
    fn receive_messages(&self) -> atlas_common::error::Result<Vec<StoredMessage<M>>> {
        self.0.recv().map_err(|err| err.into())
    }

    fn try_receive_messages(
        &self,
        timeout: Option<Duration>,
    ) -> atlas_common::error::Result<Option<Vec<StoredMessage<M>>>> {
        match timeout {
            None => match self.0.try_recv() {
                Ok(message) => Ok(Some(message)),
                Err(err) => match err {
                    TryRecvError::ChannelEmpty => Ok(None),
                    TryRecvError::ChannelDc | TryRecvError::Timeout => {
                        Err!(err)
                    }
                },
            },
            Some(duration) => match self.0.recv_timeout(duration) {
                Ok(message) => Ok(Some(message)),
                Err(err) => match err {
                    TryRecvError::ChannelEmpty | TryRecvError::Timeout => Ok(None),
                    TryRecvError::ChannelDc => {
                        Err!(err)
                    }
                },
            },
        }
    }
}
