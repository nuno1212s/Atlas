use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use getset::{CopyGetters, Getters, MutGetters};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, info, warn};

use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::collections::HashMap;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::Err;
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration::NodeInfo;
use atlas_core::reconfiguration_protocol::{
    QuorumReconfigurationMessage, QuorumReconfigurationResponse, QuorumUpdateMessage,
    ReconfigurableNodeType,
};
use atlas_core::timeouts::timeout::TimeoutModHandle;

use crate::message::{
    CommittedQC, LockedQC, OperationMessage, QuorumAcceptResponse, QuorumCommitAcceptResponse,
    QuorumJoinReconfMessages, QuorumObtainInfoOpMessage,
};
use crate::quorum_config::network::QuorumConfigNetworkNode;
use crate::quorum_config::operations::client_notify_quorum_op::NotifyClientOperation;
use crate::quorum_config::operations::notify_stable_quorum::NotifyQuorumOperation;
use crate::quorum_config::operations::quorum_accept_op::QuorumAcceptNodeOperation;
use crate::quorum_config::operations::quorum_info_op::ObtainQuorumInfoOP;
use crate::quorum_config::operations::quorum_join_op::EnterQuorumOperation;
use crate::quorum_config::operations::{Operation, OperationObj, OperationResponse};
use crate::QuorumProtocolResponse;

pub mod network;

pub mod operations;

/// This whole module is completly experimental and is not yet ready for use
/// It's completely lacking support for timeouts, so most faults are not actually tolerated.
/// It's also ugly and did not turn out one bit like I initially thought, so it's a mess.
/// In the future I might remake it, but not really sure if it's worth it.
/// This is a simple observer of the quorum, which might then be extended to support
/// Other features, such as quorum reconfiguration or just keeping track of the quorum
/// (in the case of clients)
#[derive(Clone)]
pub struct QuorumObserver {
    quorum_view: Arc<Mutex<QuorumView>>,
}

#[derive(Clone)]
pub enum ReplicaState {
    Awaiting,
    ObtainingInfo,
    Joining,
    Member,
}

/// The current state of the client
#[derive(Clone)]
pub enum ClientState {
    Awaiting,
    ObtainingInfo,
    Idle,
}

/// The type of node we are representing
#[derive(Clone)]
pub enum NodeStatusType {
    ClientNode {
        quorum_comm: ChannelSyncTx<QuorumUpdateMessage>,
        current_state: ClientState,
    },
    QuorumNode {
        quorum_communication: ChannelSyncTx<QuorumReconfigurationMessage>,
        quorum_responses: ChannelSyncRx<QuorumReconfigurationResponse>,
        current_state: ReplicaState,
    },
}

#[derive(Getters, CopyGetters, MutGetters)]
/// The node structure that handles all information required by the node
pub struct InternalNode {
    #[get = "pub"]
    node_info: NodeInfo,

    // The observer, maintains the current view of the quorum
    // Since this is shared by a lot of the operations, it is
    // Send + Sync
    #[get = "pub"]
    observer: QuorumObserver,

    #[getset(get_mut = "pub", get = "pub")]
    data: NodeOpData,

    #[getset(get = "pub", get_mut = "pub(super)")]
    node_type: NodeStatusType,
}

/// The node structure, stores the current state of the node
/// and of the operations performed on said node
#[derive(Getters, CopyGetters, MutGetters)]
pub struct Node {
    #[getset(get = "pub", get_mut = "pub(super)")]
    node: InternalNode,

    #[getset(get = "pub", get_mut = "pub(super)")]
    ongoing_ops: OnGoingOperations,
}

/// The key type for the operation data.
/// we use this because we want to reduce the amount of hash functions we
/// have to run in order to reach our data, so we bundle these two together
#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct OpDataKey {
    op: &'static str,
    key: &'static str,
}

/// A global data cache for operations
/// Data here can be accessible across operations of the same type and
/// across operation types (all info is public)
pub struct NodeOpData {
    op_data: HashMap<OpDataKey, Box<dyn Any>>,
}

/// Management of all the ongoing operations
pub struct OnGoingOperations {
    op_seq_gen: SeqNo,

    ongoing_operations: BTreeMap<&'static str, OperationObj>,
    awaiting_reconfig_response: Option<&'static str>,
}

/// The current view of nodes in the network, as in which of them
/// are currently partaking in the consensus
#[derive(Clone, Debug, PartialEq, Eq, Hash, Getters, CopyGetters)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumView {
    #[get_copy = "pub"]
    sequence_number: SeqNo,

    #[get = "pub"]
    quorum_members: Vec<NodeId>,

    #[get_copy = "pub"]
    f: usize,
}

impl Orderable for QuorumView {
    fn sequence_number(&self) -> SeqNo {
        self.sequence_number
    }
}

impl QuorumView {
    pub fn empty() -> Self {
        QuorumView {
            sequence_number: SeqNo::ZERO,
            quorum_members: Vec::new(),
            f: 0,
        }
    }

    pub fn with_bootstrap_nodes(bootstrap_nodes: Vec<NodeId>) -> Self {
        QuorumView {
            sequence_number: SeqNo::ZERO,
            quorum_members: bootstrap_nodes,
            f: 1,
        }
    }

    pub fn next_with_added_node(&self, node_id: NodeId, f: usize) -> Self {
        QuorumView {
            sequence_number: self.sequence_number.next(),
            quorum_members: {
                let mut members = self.quorum_members.clone();
                members.push(node_id);
                members
            },
            f,
        }
    }
}

impl QuorumObserver {
    pub fn from_bootstrap(bootstrap_nodes: Vec<NodeId>) -> Self {
        Self {
            quorum_view: Arc::new(Mutex::new(QuorumView::with_bootstrap_nodes(
                bootstrap_nodes,
            ))),
        }
    }

    pub fn current_view(&self) -> QuorumView {
        self.quorum_view.lock().unwrap().clone()
    }

    pub fn install_quorum_view(&self, view: QuorumView) {
        let mut guard = self.quorum_view.lock().unwrap();

        *guard = view;
    }
}

impl Node {
    pub fn init_observer(bootstrap_nodes: Vec<NodeId>) -> QuorumObserver {
        QuorumObserver::from_bootstrap(bootstrap_nodes)
    }

    pub fn initialize_with_observer(
        node_info: NodeInfo,
        observer: QuorumObserver,
        node_type: ReconfigurableNodeType,
    ) -> Self {
        Self {
            node: InternalNode::init_with_observer(node_info, observer, node_type),
            ongoing_ops: OnGoingOperations::initialize(),
        }
    }

    pub fn initialize_node(
        node_info: NodeInfo,
        bootstrap_nodes: Vec<NodeId>,
        node_type: ReconfigurableNodeType,
    ) -> (Self, QuorumObserver) {
        let internal_node = InternalNode::initialize(node_info, bootstrap_nodes, node_type);

        let observer = internal_node.observer().clone();

        let node = Self {
            node: internal_node,
            ongoing_ops: OnGoingOperations::initialize(),
        };

        (node, observer)
    }

    pub fn iterate<NT>(&mut self, network: &NT) -> Result<QuorumProtocolResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        self.ongoing_ops.iterate(&mut self.node, network)
    }

    pub fn handle_message<NT>(
        &mut self,
        network: &NT,
        header: Header,
        message: OperationMessage,
    ) -> Result<QuorumProtocolResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        self.ongoing_ops
            .handle_message(&mut self.node, network, header, message)
    }

    pub fn handle_timeout<NT>(&mut self, _network: &NT, _timeouts: &TimeoutModHandle)
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        todo!("Still have to handle timeouts here")
    }
}

impl InternalNode {
    fn init_node_type_from(node_type: ReconfigurableNodeType) -> NodeStatusType {
        match node_type {
            ReconfigurableNodeType::ClientNode(tx) => NodeStatusType::ClientNode {
                quorum_comm: tx,
                current_state: ClientState::Awaiting,
            },
            ReconfigurableNodeType::QuorumNode(tx, rx) => NodeStatusType::QuorumNode {
                quorum_communication: tx,
                quorum_responses: rx,
                current_state: ReplicaState::Awaiting,
            },
        }
    }

    fn init_with_observer(
        node_info: NodeInfo,
        observer: QuorumObserver,
        node_type: ReconfigurableNodeType,
    ) -> Self {
        Self {
            node_info,
            observer,
            data: NodeOpData::new(),
            node_type: Self::init_node_type_from(node_type),
        }
    }

    fn initialize(
        node_info: NodeInfo,
        bootstrap_nodes: Vec<NodeId>,
        node_type: ReconfigurableNodeType,
    ) -> Self {
        Self {
            node_info,
            observer: QuorumObserver::from_bootstrap(bootstrap_nodes),
            data: NodeOpData::new(),
            node_type: Self::init_node_type_from(node_type),
        }
    }

    /// Are we currently part of the quorum?
    pub fn is_part_of_quorum(&self) -> bool {
        self.observer
            .current_view()
            .quorum_members()
            .contains(&self.node_id())
    }

    pub fn node_id(&self) -> NodeId {
        self.node_info.node_id()
    }
}

impl OnGoingOperations {
    // Initialize the ongoing operations structure
    pub fn initialize() -> Self {
        Self {
            op_seq_gen: SeqNo::ZERO,
            ongoing_operations: Default::default(),
            awaiting_reconfig_response: None,
        }
    }

    pub fn launch_operation(&mut self, operation: OperationObj) -> Result<()> {
        info!("Launching operation {}", operation.op_name());

        if self.ongoing_operations.contains_key(operation.op_name()) {
            return Err!(OperationErrors::AlreadyOngoingOperation(
                operation.op_name()
            ));
        }

        self.ongoing_operations
            .insert(operation.op_name(), operation);

        Ok(())
    }

    fn handle_operation_result<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        operation_name: &'static str,
        result: OperationResponse,
    ) -> Result<QuorumProtocolResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match result {
            OperationResponse::Completed | OperationResponse::CompletedFailed => {
                self.finish_operation_by_name(operation_name, node, network)?;

                self.handle_no_longer_awaiting_response_protocol(operation_name);

                let is_part_of_quorum = node.is_part_of_quorum();

                match node.node_type_mut() {
                    NodeStatusType::ClientNode { .. } => {
                        return match operation_name {
                            NotifyClientOperation::OP_NAME => {
                                info!("We have notified the client, calling initial setup done");

                                Ok(QuorumProtocolResponse::DoneInitialSetup)
                            }
                            ObtainQuorumInfoOP::OP_NAME => {
                                info!("We have obtained the quorum information, calling client notify operation.");

                                self.launch_client_notify_op(node)?;

                                Ok(QuorumProtocolResponse::Nil)
                            }
                            _ => Ok(QuorumProtocolResponse::Nil),
                        };
                    }
                    NodeStatusType::QuorumNode { current_state, .. } => {
                        if let ReplicaState::ObtainingInfo = current_state {
                            if is_part_of_quorum {
                                info!("We have obtained the quorum information, and we are part of the quorum, calling initial setup done");

                                *current_state = ReplicaState::Member;

                                self.launch_quorum_notify_op(node)?;

                                return Ok(QuorumProtocolResponse::DoneInitialSetup);
                            } else if operation_name == ObtainQuorumInfoOP::OP_NAME {
                                //TODO: This should only be done if we have completed with success, not failure
                                *current_state = ReplicaState::Joining;

                                self.launch_quorum_join_op(node)?;
                            }
                        } else if let ReplicaState::Joining = current_state {
                            if is_part_of_quorum {
                                info!("We have joined the quorum, calling initial setup done");

                                *current_state = ReplicaState::Member;

                                self.launch_quorum_notify_op(node)?;

                                return Ok(QuorumProtocolResponse::DoneInitialSetup);
                            }
                        }
                    }
                }
            }
            OperationResponse::AwaitingResponseProtocol => {
                if self.awaiting_reconfig_response.is_none() {
                    self.awaiting_reconfig_response = Some(operation_name);
                } else {
                    error!("The operation {} is already awaiting a response, but another operation is trying to await a response", operation_name);
                }
            }
            OperationResponse::NoLongerAwaitingResponseProtocol => {
                self.handle_no_longer_awaiting_response_protocol(operation_name);
            }
            OperationResponse::Processing => {}
        }

        Ok(QuorumProtocolResponse::Nil)
    }

    /// Iterate all ongoing operations
    pub fn iterate<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
    ) -> Result<QuorumProtocolResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        let mut finished_ops = Vec::new();

        if let NodeStatusType::QuorumNode {
            quorum_responses, ..
        } = node.node_type().clone()
        {
            while let Ok(response) = quorum_responses.try_recv() {
                if let Some(target) = self.awaiting_reconfig_response {
                    if let Some(op) = self.get_mut_operation_of_type(target) {
                        // Process the quorum messages appropriately

                        debug!(
                            "Received a quorum response for operation {} with response {:?}",
                            target, response
                        );

                        finished_ops.push((
                            op.op_name(),
                            op.handle_quorum_response(node, network, response)?,
                        ));
                    } else {
                        error!("We have a registered awaiting reconfig response, but we do not have an operation of that type ({:?})", target);
                    }
                } else {
                    error!(
                        "We have received a quorum response, but we are not awaiting one. ({:?})",
                        response
                    );
                }
            }
        }

        match &mut node.node_type {
            NodeStatusType::ClientNode { current_state, .. } => {
                if let ClientState::Awaiting = current_state {
                    if !self.has_operation_of_type(ObtainQuorumInfoOP::OP_NAME) {
                        info!("Launching quorum info operation as we have not yet obtained the quorum info");

                        *current_state = ClientState::ObtainingInfo;

                        self.launch_quorum_obtain_info_op(node)?;
                    }
                }
            }
            NodeStatusType::QuorumNode { current_state, .. } => {
                if let ReplicaState::Awaiting = current_state {
                    if !self.has_operation_of_type(ObtainQuorumInfoOP::OP_NAME) {
                        info!("Launching quorum info operation as we have not yet obtained the quorum info");

                        *current_state = ReplicaState::ObtainingInfo;

                        self.launch_quorum_obtain_info_op(node)?;
                    }
                }
            }
        }

        for (op_name, op) in self.ongoing_operations.iter_mut() {
            let result = match op.iterate(node, network) {
                Ok(res) => res,
                Err(err) => {
                    error!("Error while iterating operation: {:?}", err);
                    continue;
                }
            };

            finished_ops.push((*op_name, result));
        }

        finished_ops
            .into_iter()
            .map(|(op_name, op_res)| self.handle_operation_result(node, network, op_name, op_res))
            .collect::<Result<Vec<QuorumProtocolResponse>>>()
            .map(|responses| {
                responses
                    .into_iter()
                    .reduce(|acc, res| match (&acc, &res) {
                        (QuorumProtocolResponse::DoneInitialSetup, _) => acc,
                        (_, QuorumProtocolResponse::DoneInitialSetup) => res,
                        (QuorumProtocolResponse::UpdatedQuorum(_), _) => acc,
                        (_, QuorumProtocolResponse::UpdatedQuorum(_)) => res,
                        _ => QuorumProtocolResponse::Nil,
                    })
                    .unwrap_or(QuorumProtocolResponse::Nil)
            })
    }

    pub fn handle_message<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        header: Header,
        message: OperationMessage,
    ) -> Result<QuorumProtocolResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        let response: OperationResponse;
        let op_name: &'static str;

        match &message {
            OperationMessage::QuorumInfoOp(info) => match info {
                QuorumObtainInfoOpMessage::RequestInformationMessage => {
                    ObtainQuorumInfoOP::respond_to_request(node, network, header)?;

                    return Ok(QuorumProtocolResponse::Nil);
                }
                QuorumObtainInfoOpMessage::QuorumInformationResponse(_) => {
                    let possible_operation =
                        self.get_mut_operation_of_type(ObtainQuorumInfoOP::OP_NAME);

                    if let Some(op) = possible_operation {
                        response = op.handle_received_message(node, network, header, message)?;
                        op_name = op.op_name();
                    } else {
                        warn!("Received a quorum information response message, but we are not awaiting one (No on going operation). Ignoring it.");

                        return Ok(QuorumProtocolResponse::Nil);
                    }
                }
            },
            OperationMessage::QuorumReconfiguration(reconf_message) => match node.node_type() {
                NodeStatusType::ClientNode { .. } => {
                    error!("Received a quorum reconfiguration message on a client node");

                    return Ok(QuorumProtocolResponse::Nil);
                }
                NodeStatusType::QuorumNode { current_state, .. } => {
                    info!(
                        "Received a quorum reconfiguration message: {:?} from {:?}",
                        reconf_message,
                        header.from()
                    );

                    if self.has_operation_of_type(QuorumAcceptNodeOperation::OP_NAME) {
                        let operation = self
                            .get_mut_operation_of_type(QuorumAcceptNodeOperation::OP_NAME)
                            .unwrap();

                        response =
                            operation.handle_received_message(node, network, header, message)?;
                        op_name = operation.op_name();
                    } else if self.has_operation_of_type(EnterQuorumOperation::OP_NAME) {
                        let operation = self
                            .get_mut_operation_of_type(EnterQuorumOperation::OP_NAME)
                            .unwrap();

                        response =
                            operation.handle_received_message(node, network, header, message)?;
                        op_name = operation.op_name();
                    } else {
                        match reconf_message {
                            QuorumJoinReconfMessages::RequestJoinQuorum(_) => match current_state {
                                ReplicaState::Member => {
                                    info!("Received a request to join the quorum while we do not have any ongoing accept operations, launching an accept operation");

                                    let node_accept =
                                        QuorumAcceptNodeOperation::initialize(header.from());

                                    self.launch_operation(OperationObj::QuorumAcceptOp(
                                        node_accept,
                                    ))?;

                                    return self.handle_message(node, network, header, message);
                                }
                                _ => {
                                    warn!("Received a request to join the quorum, but we are not a member of the quorum");
                                }
                            },
                            _ => error!(
                                "This message should have already been handled by an operation"
                            ),
                        }

                        return Ok(QuorumProtocolResponse::Nil);
                    }
                }
            },
        }

        self.handle_operation_result(node, network, op_name, response)
    }

    fn finish_operation_by_name<NT>(
        &mut self,
        op_name: &'static str,
        node: &mut InternalNode,
        network: &NT,
    ) -> Result<()>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        let possible_op = self.ongoing_operations.remove(op_name);

        info!(
            "Attempting to finish operation by name {} (Is present {})",
            op_name,
            possible_op.is_some()
        );

        if let Some(operation) = possible_op {
            self.finish_operation(node, network, operation)?;
        };

        Ok(())
    }

    fn finish_operation<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        op: OperationObj,
    ) -> Result<()>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        info!("Finished operation {}", op.op_name());

        op.finish(node, network)?;

        Ok(())
    }

    pub fn get_operation_of_type(&self, op_type: &'static str) -> Option<&OperationObj> {
        self.ongoing_operations.get(op_type)
    }

    pub fn get_mut_operation_of_type(
        &mut self,
        op_type: &'static str,
    ) -> Option<&mut OperationObj> {
        self.ongoing_operations.get_mut(op_type)
    }

    pub fn has_operation_of_type(&self, op_type: &'static str) -> bool {
        self.ongoing_operations.contains_key(op_type)
    }

    fn launch_quorum_join_op(&mut self, node: &InternalNode) -> Result<()> {
        info!("Launching a quorum join operation");

        let op = EnterQuorumOperation::initialize(node);

        self.launch_operation(OperationObj::QuorumJoinOp(op))
    }

    fn launch_client_notify_op(&mut self, node: &InternalNode) -> Result<()> {
        NotifyClientOperation::can_execute(node)?;

        let operation = NotifyClientOperation::initialize();

        self.launch_operation(OperationObj::NotifyClientOp(operation))
    }

    // function to launch a quorum obtain info operation
    fn launch_quorum_obtain_info_op(&mut self, node: &InternalNode) -> Result<()> {
        // Check if we are able to execute this operation
        ObtainQuorumInfoOP::can_execute(node)?;

        let quorum = node.observer().current_view();

        let threshold = get_quorum_for_f(quorum.f());

        let quorum_members = quorum.quorum_members().clone();

        let op = ObtainQuorumInfoOP::initialize(threshold, quorum_members);

        self.launch_operation(OperationObj::QuorumInfoOp(op))
    }

    fn launch_quorum_notify_op(&mut self, node: &InternalNode) -> Result<()> {
        NotifyQuorumOperation::can_execute(node)?;

        let operation = NotifyQuorumOperation::initialize();

        self.launch_operation(OperationObj::NotifyQuorumOp(operation))
    }

    // Function that handles an operation no longer needing to receive quorum responses
    fn handle_no_longer_awaiting_response_protocol(&mut self, operation_name: &'static str) {
        if let Some(str) = &self.awaiting_reconfig_response {
            if **str == *operation_name {
                info!(
                    "The operation {} is no longer waiting for a quorum response",
                    operation_name
                );

                self.awaiting_reconfig_response = None;
            }
        }
    }
}

impl NodeStatusType {
    // Getters for the members of the enum,
    // which assume the given type matches the one the getter necessitates
    pub fn quorum_communication(&self) -> &ChannelSyncTx<QuorumReconfigurationMessage> {
        match self {
            NodeStatusType::QuorumNode {
                quorum_communication,
                ..
            } => quorum_communication,
            _ => unreachable!("This node type does not have a quorum communication channel"),
        }
    }

    pub fn quorum_responses(&self) -> &ChannelSyncRx<QuorumReconfigurationResponse> {
        match self {
            NodeStatusType::QuorumNode {
                quorum_responses, ..
            } => quorum_responses,
            _ => unreachable!("This node type does not have a quorum response channel"),
        }
    }

    pub fn client_communication(&self) -> &ChannelSyncTx<QuorumUpdateMessage> {
        match self {
            NodeStatusType::ClientNode { quorum_comm, .. } => quorum_comm,
            _ => unreachable!("This node type does not have a client communication channel"),
        }
    }

    pub fn current_replica_state(&self) -> &ReplicaState {
        match self {
            NodeStatusType::QuorumNode { current_state, .. } => current_state,
            _ => unreachable!("This node type does not have a replica state"),
        }
    }
}

impl NodeOpData {
    pub fn new() -> Self {
        Self {
            op_data: Default::default(),
        }
    }

    pub fn get<T: 'static>(&self, op: &'static str, key: &'static str) -> Option<&T> {
        self.op_data
            .get(&((op, key).into()))
            .and_then(|v| v.downcast_ref::<T>())
    }

    pub fn get_mut<T: 'static>(&mut self, op: &'static str, key: &'static str) -> Option<&mut T> {
        self.op_data
            .get_mut(&((op, key).into()))
            .and_then(|v| v.downcast_mut::<T>())
    }

    pub fn insert<T: 'static>(&mut self, op: &'static str, key: &'static str, value: T) {
        self.op_data.insert((op, key).into(), Box::new(value));
    }

    pub fn remove<T: 'static>(&mut self, op: &'static str, key: &'static str) -> Option<Box<T>> {
        self.op_data
            .remove(&((op, key).into()))
            .and_then(|v| v.downcast::<T>().ok())
    }
}

impl Default for NodeOpData {
    fn default() -> Self {
        Self::new()
    }
}

impl From<(&'static str, &'static str)> for OpDataKey {
    fn from(value: (&'static str, &'static str)) -> Self {
        Self {
            op: value.0,
            key: value.1,
        }
    }
}

pub trait QuorumCert: Orderable {
    type IndividualType: QuorumCertPart;

    fn quorum(&self) -> &QuorumView;

    fn proofs(&self) -> &[Self::IndividualType];
}

pub trait QuorumCertPart {
    fn view(&self) -> &QuorumView;
}

/// Get the amount of tolerated faults for a network of n nodes
/// This returns the amount of faults that can be tolerated
pub fn get_f_for_n(n: usize) -> usize {
    (n - 1) / 3
}

/// Get the amount of nodes required to form a quorum in a BFT network which tolerates [f] faults
pub fn get_quorum_for_f(f: usize) -> usize {
    2 * f + 1
}

pub fn get_quorum_for_n(n: usize) -> usize {
    get_quorum_for_f(get_f_for_n(n))
}

impl Orderable for LockedQC {
    fn sequence_number(&self) -> SeqNo {
        self.quorum().sequence_number()
    }
}

impl QuorumCert for LockedQC {
    type IndividualType = StoredMessage<QuorumAcceptResponse>;

    fn quorum(&self) -> &QuorumView {
        self.quorum()
    }

    fn proofs(&self) -> &[Self::IndividualType] {
        self.proofs().as_slice()
    }
}

impl QuorumCertPart for StoredMessage<QuorumAcceptResponse> {
    fn view(&self) -> &QuorumView {
        self.message().view()
    }
}

impl Orderable for CommittedQC {
    fn sequence_number(&self) -> SeqNo {
        self.quorum().sequence_number()
    }
}

impl QuorumCert for CommittedQC {
    type IndividualType = StoredMessage<QuorumCommitAcceptResponse>;

    fn quorum(&self) -> &QuorumView {
        self.quorum()
    }

    fn proofs(&self) -> &[Self::IndividualType] {
        self.proofs().as_slice()
    }
}

impl QuorumCertPart for StoredMessage<QuorumCommitAcceptResponse> {
    fn view(&self) -> &QuorumView {
        self.message().view()
    }
}

#[derive(Error, Debug)]
pub enum OperationErrors {
    #[error("An operation of the same type {0} is already ongoing")]
    AlreadyOngoingOperation(&'static str),
}
