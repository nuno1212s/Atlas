# Atlas

## A modular framework for building BFT/CFT protocols and applications

Atlas is an efficient and modular BFT/CFT framework, providing
the necessary building blocks to allow development of new
consensus protocols either probabilistic, deterministic, fo-
cused on blockchain or focused on SMR. By providing these
high performance, modular blocks as a base of development,
it also provides us with a fair way of comparing all imple-
mentations based on this architecture since they all rely on
the same base of operation.

Different from prior art in this field, usually implemented in garbage collected languages, Rust was
the language of choice to implement all the typical SMR sub-protocols present
in FeBFT. Many people are (rightfully so!) excited about the use of Rust
in new (and even older) software, because of its safety properties associated
with the many compile time checks present in the compiler, that are able to
hunt down common use-after-free as well as concurrency related bugs.

There are infinitely many use cases for BFT systems, which will undoubtedly improve the
availability of a digital service. However, a less robust class of systems, called CFT
systems, are often utilized in place of BFT systems, based on their greater performance.
Despite this, with the evolution of hardware, and especially the growing popularity of
blockchain technology, BFT systems are becoming more attractive to distributed system
developers.

## How to use this library to develop a BFT SMR application:

Generally, to use this library to develop applications, you will need to implement the following trait:

```rust
/// An application for a state machine replication protocol.
pub trait Application<S>: Send {
    type AppData: ApplicationData;

    /// Returns the initial state of the application.
    fn initial_state() -> Result<S>;

    /// Process an unordered client request, and produce a matching reply
    /// Cannot alter the application state
    fn unordered_execution(&self, state: &S, request: Request<Self, S>) -> Reply<Self, S>;

    /// Process a user request, producing a matching reply,
    /// meanwhile updating the application state.
    fn update(&self, state: &mut S, request: Request<Self, S>) -> Reply<Self, S>;

}
```

Along with the associated types, by implementing this trait:

```rust
pub trait ApplicationData: Send + Sync {
    /// Represents the requests forwarded to replicas by the
    /// clients of the BFT system.
    type Request:Send + Clone;

    /// Represents the replies forwarded to clients by replicas
    /// in the BFT system.
    type Reply: Send + Clone;
}
```

Depending on your needs, you can utilize both Serde (<https://lib.rs/crates/serde>) or Capnproto (<https://capnproto.org/>) (WIP as of 20-11-2023) for message serialization.

## Choosing an appropriate state

In the Application abstraction, we see that we require the end user to provide a generic type, S, which will then be utilized throughout the application. This type is the State. Our application abstractions are made to allow the developer to have much greater flexibility in their choice of state, giving them the ability to choose from an existing state type or develop his own state type, unlike other frameworks which bundle the state with the application abstraction.

This makes developing applications faster, simpler and more secure since we are able to share knowledge and use existing established state types whenever possible.

### Choosing your ordering protocol and state transfer protocol

At the moment, we have FeBFT, a PBFT based consensus protocol with several optimizations that make it an extremelly fast protocol. You can get it at <https://github.com/SecureSolutionsLab/febft> which also contains a simple implementation of a state transfer algorithm.

## Implementing your own ordering protocol or state transfer protocol

We provide an abstraction for the implementation of ordering protocols and state transfer protocols (along with the ability to change all modules that are a part of this library).

The abstraction for the ordering protocol requires you to implement the following trait:

```rust
///
/// The trait for an ordering protocol to be implemented in Atlas
pub trait OrderingProtocol<D, NT>: Orderable where D: SharedData + 'static {

    /// The type which implements OrderingProtocolMessage, to be implemented by the developer
    type Serialization: OrderingProtocolMessage + 'static;

    /// The configuration type the protocol wants to accept
    type Config;

    /// Initialize this ordering protocol with the given configuration, executor, timeouts and node
    fn initialize(config: Self::Config, args: OrderingProtocolArgs<D, NT>) -> Result<Self> where
        Self: Sized;

    /// Get the current view of the ordering protocol
    fn view(&self) -> View<Self::Serialization>;

    /// Handle a protocol message that was received while we are executing another protocol
    fn handle_off_ctx_message(&mut self, message: StoredMessage<Protocol<ProtocolMessage<Self::Serialization>>>);

    /// Handle the protocol being executed having changed (for example to the state transfer protocol)
    /// This is important for some of the protocols, which need to know when they are being executed or not
    fn handle_execution_changed(&mut self, is_executing: bool) -> Result<()>;

    /// Poll from the ordering protocol in order to know what we should do next
    /// We do this to check if there are already messages waiting to be executed that were received ahead of time and stored.
    /// Or whether we should run state tranfer or wait for messages from other replicas
    fn poll(&mut self) -> OrderProtocolPoll<ProtocolMessage<Self::Serialization>>;

    /// Process a protocol message that we have received
    /// This can be a message received from the poll() method or a message received from other replicas.
    fn process_message(&mut self, message: StoredMessage<Protocol<ProtocolMessage<Self::Serialization>>>) -> Result<OrderProtocolExecResult>;

    /// Handle a timeout received from the timeouts layer
    fn handle_timeout(&mut self, timeout: Vec<RqTimeout>) -> Result<OrderProtocolExecResult>;
}
```

Associated with this, you must also implement the associated types for the needed messages, by implementing the following trait:

```rust
/// We do not need a serde module since serde serialization is just done on the network level.
/// The abstraction for ordering protocol messages.
pub trait OrderingProtocolMessage: Send {

    /// The ordering protocol type for network views
    /// Required to implement the trait NetworkView, which is made up of simple network information methods
    #[cfg(feature = "serialize_capnp")]
    type ViewInfo: NetworkView + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type ViewInfo: NetworkView + for<'a> Deserialize<'a> + Serialize + Send + Clone + Debug;

    /// The type for the protocol message.
    /// If you want to have multiple protocol messages, you should use Rust's powerful enum system, similar to the rest
    /// of the library
    #[cfg(feature = "serialize_capnp")]
    type ProtocolMessage: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type ProtocolMessage: for<'a> Deserialize<'a> + Serialize + Send + Clone + Debug;

    /// This is only necessary when utilizing capnp
    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: febft_capnp::consensus_messages_capnp::protocol_message::Builder, msg: &Self::ProtocolMessage) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: febft_capnp::consensus_messages_capnp::protocol_message::Reader) -> Result<Self::ProtocolMessage>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_view_capnp(builder: febft_capnp::cst_messages_capnp::view_info::Builder, msg: &Self::ViewInfo) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_view_capnp(reader: febft_capnp::cst_messages_capnp::view_info::Reader) -> Result<Self::ViewInfo>;
}
```

This abstraction is not enough to currently implement an ordering protocol for Atlas, since we currently only have an SMR replica abstraction, which implicitly requires a state and therefore require state transfer protocols.
To make an ordering protocol compatible with state transfering, you must implement the following trait:

```rust
/// An order protocol that uses the state transfer protocol to manage its state.
pub trait StatefulOrderProtocol<D: SharedData + 'static, NT>: OrderingProtocol<D, NT> {
    /// The serialization abstraction for the types that are required by the state transfer protocol
    type StateSerialization: StatefulOrderProtocolMessage + 'static;

    fn initialize_with_initial_state(config: Self::Config, args: OrderingProtocolArgs<D, NT>, initial_state: Arc<ReadOnly<Checkpoint<D::State>>>) -> Result<Self> where
        Self: Sized;

    /// Get the current sequence number of the protocol, combined with a proof of it so we can send it to other replicas
    fn sequence_number_with_proof(&self) -> Result<Option<(SeqNo, SerProof<Self::StateSerialization>)>>;

    /// Verify the sequence number sent by another replica. This doesn't pass a mutable reference since we don't want to
    /// make any changes to the state of the protocol here (or allow the implementer to do so). Instead, we want to
    /// just verify this sequence number
    fn verify_sequence_number(&self, seq_no: SeqNo, proof: &SerProof<Self::StateSerialization>) -> Result<bool>;

    /// Install a state received from other replicas in the system
    /// Should only alter the necessary things within its own state and
    /// then should return the state and a list of all requests that should
    /// then be executed by the application.
    fn install_state(&mut self, state: Arc<ReadOnly<Checkpoint<D::State>>>,
                     view_info: View<Self::Serialization>,
                     dec_log: DecLog<Self::StateSerialization>) -> Result<(D::State, Vec<D::Request>)>;

    /// Install a given sequence number
    fn install_seq_no(&mut self, seq_no: SeqNo) -> Result<()>;

    /// Snapshot the current log of the replica
    fn snapshot_log(&mut self) -> Result<(Arc<ReadOnly<Checkpoint<D::State>>>,
                                          View<Self::Serialization>,
                                          DecLog<Self::StateSerialization>)>;

    /// Finalize the checkpoint of the protocol by receiving a checkpoint from the execution layer
    /// This will be triggered after a checkpoint is requested by the ordering protocol
    fn finalize_checkpoint(&mut self, state: Arc<ReadOnly<Checkpoint<D::State>>>) -> Result<()>;
}

```

With this, we msut also provide some extra types for serialization, needed by the state transfer protocol with the following trait:

```rust
/// The messages for the stateful ordering protocol
pub trait StatefulOrderProtocolMessage: Send {

    /// The entire declare log needed for the recovering replica to reach
    /// from the latest checkpoint to the current decision of the quorum.
    #[cfg(feature = "serialize_serde")]
    type DecLog: OrderProtocolLog + for<'a> Deserialize<'a> + Serialize + Send + Clone;
    
    #[cfg(feature = "serialize_capnp")]
    type DecLog: OrderProtocolLog + Send + Clone;


    /// A proof of a given Sequence number in the consensus protocol
    /// This is used when requesting the latest consensus id in the state transfer protocol,
    /// in order to verify that a given consensus id is valid
    #[cfg(feature = "serialize_serde")]
    type Proof: OrderProtocolProof + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type Proof: OrderProtocolProof + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_declog_capnp(builder: febft_capnp::cst_messages_capnp::dec_log::Builder, msg: &Self::DecLog) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_declog_capnp(reader: febft_capnp::cst_messages_capnp::dec_log::Reader) -> Result<Self::DecLog>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_proof_capnp(builder: febft_capnp::cst_messages_capnp::proof::Builder, msg: &Self::Proof) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_proof_capnp(reader: febft_capnp::cst_messages_capnp::proof::Reader) -> Result<Self::Proof>;
}
```

## Implementing your own state transfer protocol

Implementing a new state transfer protocol is just as simple. We currently assume the State is just a Serializable object, defined in the execution layer (so by the application developer), but upcoming updates might change this. So this state transfer protocol is currently only valid for this definition of replica.

```rust
/// A trait for the implementation of the state transfer protocol
pub trait StateTransferProtocol<D, OP, NT> where
    D: SharedData + 'static,
    OP: StatefulOrderProtocol<D, NT> + 'static {

    /// The type which implements StateTransferMessage, to be implemented by the developer
    type Serialization: StateTransferMessage + 'static;

    /// The configuration type the protocol wants to accept
    type Config;

    /// Initialize the state transferring protocol with the given configuration, timeouts and communication layer
    fn initialize(config: Self::Config, timeouts: Timeouts, node: Arc<NT>) -> Result<Self>
        where Self: Sized;

    /// Request the latest state from the rest of replicas
    fn request_latest_state(&mut self,
                            order_protocol: &mut OP) -> Result<()>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Handle a state transfer protocol message that was received while executing the ordering protocol
    fn handle_off_ctx_message(&mut self,
                              order_protocol: &mut OP,
                              message: StoredMessage<StateTransfer<CstM<Self::Serialization>>>)
                              -> Result<()>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Process a state transfer protocol message, received from other replicas
    /// We also provide a mutable reference to the stateful ordering protocol, so the 
    /// state can be installed (if that's the case)
    fn process_message(&mut self,
                       order_protocol: &mut OP,
                       message: StoredMessage<StateTransfer<CstM<Self::Serialization>>>)
                       -> Result<STResult<D>>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Handle having received a state from the application
    fn handle_state_received_from_app(&mut self,
                                      order_protocol: &mut OP,
                                      state: Arc<ReadOnly<Checkpoint<D::State>>>) -> Result<()>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Handle a timeout being received from the timeout layer
    fn handle_timeout(&mut self, order_protocol: &mut OP, timeout: Vec<RqTimeout>) -> Result<STTimeoutResult>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;
}
```

Along with the corresponding message types, defined by the trait State Transfer Message.
