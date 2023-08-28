use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_execution::serialize::ApplicationData;
use atlas_core::serialize::ClientServiceMsg;
use std::sync::Arc;
use atlas_communication::FullNetworkNode;
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_core::ordering_protocol::OrderProtocolTolerance;
use atlas_core::ordering_protocol::reconfigurable_order_protocol::ReconfigurableOrderProtocol;
use atlas_core::reconfiguration_protocol::ReconfigurationProtocol;
use atlas_reconfiguration::message::ReconfData;
use atlas_reconfiguration::network_reconfig::NetworkInfo;
use crate::client::ClientData;
use crate::client::{Client, ClientConfig, ClientType, register_callback, RequestCallback};

/// A client implementation that will automagically manage all of the sessions required, reutilizing them
/// as much as possible
/// Can be cloned in order to be used in multiple locations simultaneously
#[derive(Clone)]
pub struct ConcurrentClient<RF, D: ApplicationData + 'static, NT: 'static> {
    id: NodeId,
    client_data: Arc<ClientData<RF, D>>,
    session_return: ChannelSyncTx<Client<RF, D, NT>>,
    sessions: ChannelSyncRx<Client<RF, D, NT>>,
}

impl<RF, D, NT> ConcurrentClient<RF, D, NT>
    where D: ApplicationData + 'static,
          RF: ReconfigurationProtocol, NT: 'static {
    /// Creates a new concurrent client, with the given configuration
    pub async fn boostrap_client<ROP>(cfg: ClientConfig<RF, D, NT>, session_limit: usize) -> Result<Self> where
        NT: FullNetworkNode<RF::InformationProvider, RF::Serialization, ClientServiceMsg<D>>,
        ROP: OrderProtocolTolerance + 'static {
        let (tx, rx) = channel::new_bounded_sync(session_limit);

        let client = Client::bootstrap::<ROP>(cfg).await?;

        let id = client.id();
        let data = client.client_data().clone();

        for _ in 1..session_limit {
            tx.send(client.clone()).wrapped(ErrorKind::CommunicationChannelCrossbeam)?;
        }

        tx.send(client).wrapped(ErrorKind::CommunicationChannelCrossbeam)?;

        Ok(Self {
            id: id,
            client_data: data,
            session_return: tx,
            sessions: rx,
        })
    }

    /// Creates a new concurrent client, from an already existing client
    pub fn from_client(client: Client<RF, D, NT>, session_limit: usize) -> Result<Self>
        where NT: FullNetworkNode<NetworkInfo, ReconfData, ClientServiceMsg<D>> {
        let (tx, rx) = channel::new_bounded_sync(session_limit);

        let id = client.id();
        let data = client.client_data().clone();

        for _ in 1..session_limit {
            tx.send(client.clone()).wrapped(ErrorKind::CommunicationChannelCrossbeam)?;
        }

        tx.send(client).wrapped(ErrorKind::CommunicationChannelCrossbeam)?;

        Ok(Self {
            id: id,
            client_data: data,
            session_return: tx,
            sessions: rx,
        })
    }

    #[inline]
    pub fn id(&self) -> NodeId {
        self.id
    }

    fn get_session(&self) -> Result<Client<RF, D, NT>> {
        self.sessions.recv().wrapped(ErrorKind::CommunicationChannelCrossbeam)
    }

    /// Updates the replicated state of the application running
    /// on top of `atlas`.
    pub async fn update<T>(&self, request: D::Request) -> Result<D::Reply> where T: ClientType<RF, D, NT> + 'static,
                                                                                 NT: FullNetworkNode<NetworkInfo, ReconfData, ClientServiceMsg<D>> {
        let mut session = self.get_session()?;

        let result = session.update::<T>(request).await;

        self.session_return.send(session).wrapped(ErrorKind::CommunicationChannelCrossbeam)?;

        result
    }

    ///Update the SMR state with the given operation
    /// The callback should be a function to execute when we receive the response to the request.
    ///
    /// FIXME: This callback is going to be executed in an important thread for client performance,
    /// So in the callback, we should not perform any heavy computations / blocking operations as that
    /// will hurt the performance of the client. If you wish to perform heavy operations, move them
    /// to other threads to prevent slowdowns
    pub fn update_callback<T>(&self, request: D::Request, callback: RequestCallback<D>) -> Result<()>
        where T: ClientType<RF, D, NT> + 'static,
              NT: FullNetworkNode<NetworkInfo, ReconfData, ClientServiceMsg<D>> {
        let mut session = self.get_session()?;

        let session_return = self.session_return.clone();

        let request_key = session.update_callback_inner::<T>(request);

        let session_id = session.session_id();

        let callback = Box::new(move |reply| {
            callback(reply);

            session_return.send(session).wrapped(ErrorKind::CommunicationChannelCrossbeam).unwrap();
        });

        register_callback(session_id, request_key, &*self.client_data, callback);

        Ok(())
    }
}