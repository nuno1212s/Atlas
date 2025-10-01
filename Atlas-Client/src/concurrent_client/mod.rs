use crate::client;
use crate::client::{get_request_key, register_wrapped_callback, ClientData, RequestCallbackArc};
use crate::client::{register_callback, Client, ClientConfig, ClientType, RequestCallback};
use anyhow::Context;
use atlas_common::channel::oneshot::OneShotRx;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_common::{channel, quiet_unwrap};
use atlas_communication::stub::RegularNetworkStub;
use atlas_core::ordering_protocol::OrderProtocolTolerance;
use atlas_core::reconfiguration_protocol::ReconfigurationProtocol;
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_core::networking::client::SMRClientNetworkNode;
use atlas_smr_core::serialize::SMRSysMsg;
use dashmap::DashMap;
use std::sync::{Arc, Mutex};
use tracing::error;

pub type CleanUpTask<D: ApplicationData> = dyn Fn((SeqNo, Result<D::Reply>)) + Send + Sync;

/// A client implementation that will automagically manage all the sessions required, re-utilizing them
/// as much as possible
/// Can be cloned in order to be used in multiple locations simultaneously
#[derive(Clone)]
pub struct ConcurrentClient<RF, D: ApplicationData + 'static, NT: 'static> {
    id: NodeId,
    client_data: Arc<ClientData<RF, D>>,
    session_return: ChannelSyncTx<Client<RF, D, NT>>,
    sessions: ChannelSyncRx<Client<RF, D, NT>>,
    cleanup_task: Arc<CleanUpTask<D>>,
    cleanup_task_data: Arc<DashMap<SeqNo, RequestData<RF, D, NT>>>,
}

struct RequestData<RF, D: ApplicationData + 'static, NT: 'static> {
    client_return: Mutex<Option<OneShotRx<Client<RF, D, NT>>>>,
    callback: RqCallback<D>,
}

enum RqCallback<D: ApplicationData> {
    ArcCallback(RequestCallbackArc<D>),
}

pub async fn bootstrap_client<RP, D, NT, ROP>(
    id: NodeId,
    cfg: ClientConfig<RP, D, NT>,
    session_limit: usize,
) -> Result<ConcurrentClient<RP, D, NT::AppNode>>
where
    RP: ReconfigurationProtocol + 'static,
    D: ApplicationData + 'static,
    NT: SMRClientNetworkNode<RP::InformationProvider, RP::Serialization, D> + 'static,
    ROP: OrderProtocolTolerance,
{
    // Creates a new concurrent client, with the given configuration
    let (tx, rx) = channel::sync::new_bounded_sync(session_limit, None::<String>);

    let client = client::bootstrap_client::<RP, D, NT, ROP>(id, cfg).await?;

    let id = client.id();
    let data = client.client_data().clone();

    // Populate the channel with the given session limit
    for _ in 1..session_limit {
        tx.send(client.clone())?;
    }

    tx.send(client)?;

    let (cleanup_task, cleanup_task_data) = initialize_clean_up::<RP, D, NT::AppNode>(tx.clone());

    Ok(ConcurrentClient {
        id,
        client_data: data,
        session_return: tx,
        sessions: rx,
        cleanup_task,
        cleanup_task_data,
    })
}

#[allow(clippy::type_complexity)]
fn initialize_clean_up<RP, D, NT>(
    session_return: ChannelSyncTx<Client<RP, D, NT>>,
) -> (
    Arc<CleanUpTask<D>>,
    Arc<DashMap<SeqNo, RequestData<RP, D, NT>>>,
)
where
    RP: ReconfigurationProtocol + 'static,
    D: ApplicationData + 'static,
    NT: Send + Sync + 'static,
{
    let cleanup_task_data: Arc<DashMap<SeqNo, RequestData<RP, D, NT>>> =
        Arc::new(DashMap::default());

    let cleanup_task_data_clone = cleanup_task_data.clone();

    let cleanup_task = Arc::new(move |(session_id, reply)| {
        let cleanup_data = cleanup_task_data_clone.remove(&session_id);

        if let Some((_, request)) = cleanup_data {
            match request.callback {
                RqCallback::ArcCallback(callback) => {
                    callback(reply);
                }
            }

            let client = request
                .client_return
                .lock()
                .unwrap()
                .take()
                .expect("Failed to get returned session")
                .recv()
                .expect("Failed to get returned session");

            quiet_unwrap!(session_return.send(client));
        }
    });

    (cleanup_task, cleanup_task_data)
}

impl<RF, D, NT> ConcurrentClient<RF, D, NT>
where
    D: ApplicationData + 'static,
    RF: ReconfigurationProtocol,
    NT: 'static,
{
    /// Creates a new concurrent client, from an already existing client
    pub fn from_client(client: Client<RF, D, NT>, session_limit: usize) -> Result<Self>
    where
        NT: RegularNetworkStub<SMRSysMsg<D>>,
    {
        let (tx, rx) = channel::sync::new_bounded_sync(session_limit, None::<String>);

        let id = client.id();
        let data = client.client_data().clone();

        // Populate the channel with the given session limit
        for _ in 1..session_limit {
            tx.send(client.clone())?;
        }

        tx.send(client)?;

        let (cleanup_task, cleanup_task_data) = initialize_clean_up::<RF, D, NT>(tx.clone());

        Ok(Self {
            id,
            client_data: data,
            session_return: tx,
            sessions: rx,
            cleanup_task,
            cleanup_task_data,
        })
    }

    #[inline]
    pub fn id(&self) -> NodeId {
        self.id
    }

    fn get_session(&self) -> Result<Client<RF, D, NT>> {
        self.sessions.recv().context("Failed to get session")
    }

    fn register_callback_cleanup(&self, session_id: SeqNo, callback: RequestData<RF, D, NT>) {
        self.cleanup_task_data.insert(session_id, callback);
    }

    /// Updates the replicated state of the application running
    /// on top of `atlas`.
    pub async fn update<T>(&self, request: D::Request) -> Result<D::Reply>
    where
        T: ClientType<RF, D, NT> + 'static,
        NT: RegularNetworkStub<SMRSysMsg<D>>,
    {
        let mut session = self.get_session()?;

        let result = session.update::<T>(request).await;

        self.session_return.send(session)?;

        result
    }

    ///Update the SMR state with the given operation
    /// The callback should be a function to execute when we receive the response to the request.
    ///
    /// FIXME: This callback is going to be executed in an important thread for client performance,
    /// So in the callback, we should not perform any heavy computations / blocking operations as that
    /// will hurt the performance of the client. If you wish to perform heavy operations, move them
    /// to other threads to prevent slowdowns
    pub fn update_callback<T>(
        &self,
        request: D::Request,
        callback: RequestCallback<D>,
    ) -> Result<()>
    where
        T: ClientType<RF, D, NT> + 'static,
        NT: RegularNetworkStub<SMRSysMsg<D>>,
    {
        let mut session = self.get_session()?;

        let session_id = session.session_id();

        let session_return = self.session_return.clone();

        let operation_id = session.next_operation_id();

        let rq_key = get_request_key(session_id, operation_id);

        let (return_session_tx, return_session_rx) = channel::oneshot::new_oneshot_channel();

        let callback = Box::new(move |reply| {
            callback(reply);

            let client = return_session_rx
                .recv()
                .expect("Failed to get returned session");

            quiet_unwrap!(session_return.send(client));
        });

        register_callback(session_id, rq_key, &*self.client_data, callback);

        session.update_callback_inner::<T>(request, operation_id);

        return_session_tx.send(session)?;

        Ok(())
    }
    pub fn update_imm_callback<T>(
        &self,
        request: D::Request,
        callback: RequestCallbackArc<D>,
    ) -> Result<()>
    where
        T: ClientType<RF, D, NT> + 'static,
        NT: RegularNetworkStub<SMRSysMsg<D>>,
    {
        let mut session = self.get_session()?;

        let session_id = session.session_id();

        let operation_id = session.next_operation_id();

        let rq_key = get_request_key(session_id, operation_id);

        let (return_session_tx, return_session_rx) = channel::oneshot::new_oneshot_channel();

        self.register_callback_cleanup(
            session_id,
            RequestData {
                client_return: Mutex::new(Some(return_session_rx)),
                callback: RqCallback::ArcCallback(callback),
            },
        );

        register_wrapped_callback(
            session_id,
            rq_key,
            &*self.client_data,
            self.cleanup_task.clone(),
        );

        session.update_callback_inner::<T>(request, operation_id);

        return_session_tx.send(session)?;

        Ok(())
    }
}
