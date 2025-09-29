# Atlas

## A Modular Framework for Building BFT/CFT Protocols and Applications

Atlas is an efficient and modular Byzantine Fault Tolerant (BFT) and Crash Fault Tolerant (CFT) framework that provides the necessary building blocks for developing new consensus protocols, whether probabilistic, deterministic, blockchain-focused, or State Machine Replication (SMR) focused. By providing high-performance, modular components, Atlas enables fair comparison of all implementations based on this architecture as they rely on the same operational foundation.

## Why Rust?

Unlike most prior BFT implementations written in garbage-collected languages, Atlas is implemented in Rust. The language choice offers significant benefits:

- **Memory Safety**: Rust's compile-time checks prevent common use-after-free and concurrency-related bugs
- **Performance**: Rust provides near-native performance without garbage collection overhead
- **Concurrency**: Rust's ownership model allows for safe concurrent programming
- **Reliability**: Strong typing and compile-time guarantees increase system reliability

## Project Structure

Atlas is organized into multiple modules, each responsible for specific functionality:

- **Atlas-Common**: Core utilities and abstractions used throughout the framework
- **Atlas-Core**: Core BFT/CFT protocol components and abstractions
- **Atlas-Communication**: Network communication layer
- **Atlas-Comm-MIO**: MIO-based communication implementation
- **Atlas-SMR-Application**: Application abstractions for state machine replication
- **Atlas-SMR-Replica**: Replica implementation for state machine replication
- **Atlas-Client**: Client implementation for interacting with the system
- **Atlas-Persistent-Log**: Log persistence implementation
- **Atlas-Decision-Log**: Consensus decision logging system
- **Atlas-Reconfiguration**: System reconfiguration components
- **Atlas-Metrics**: Performance metrics collection
- **Atlas-Tools**: Utility tools for key generation and configuration

## Developing BFT/CFT Applications

To develop an application using Atlas, you'll need to implement the `Application` trait:

```rust
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

Along with implementing the `ApplicationData` trait:

```rust
pub trait ApplicationData: Send + Sync {
    /// Represents the requests forwarded to replicas by the
    /// clients of the BFT system.
    type Request: Send + Clone;

    /// Represents the replies forwarded to clients by replicas
    /// in the BFT system.
    type Reply: Send + Clone;
}
```

## Choosing an Appropriate State

Atlas provides flexibility in choosing your application state. The framework allows you to:

- Select from existing state types
- Develop your own custom state type
- Use shared state implementations when appropriate

This flexibility makes developing applications faster, simpler, and more secure by leveraging established state types when possible.

## Choosing Ordering and State Transfer Protocols

Atlas currently works with FeBFT, a PBFT-based consensus protocol with optimizations that make it extremely fast. You can find it at [https://github.com/SecureSolutionsLab/febft](https://github.com/SecureSolutionsLab/febft), which also includes a simple state transfer algorithm implementation.

## Implementing Custom Protocols

Atlas provides abstractions for implementing custom ordering and state transfer protocols. The ordering protocol abstraction requires implementing the `OrderingProtocol` trait:

```rust
pub trait OrderingProtocol<D, NT>: Orderable where D: SharedData + 'static {
    /// The type which implements OrderingProtocolMessage
    type Serialization: OrderingProtocolMessage + 'static;

    /// The configuration type the protocol accepts
    type Config;

    /// Initialize this ordering protocol
    fn initialize(config: Self::Config, args: OrderingProtocolArgs<D, NT>) -> Result<Self> where Self: Sized;

    /// Get the current view of the ordering protocol
    fn view(&self) -> View<Self::Serialization>;

    /// Handle protocol messages received during execution of another protocol
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

## Serialization Options

Atlas supports both Serde and Cap'n Proto for message serialization, giving you flexibility based on your performance and compatibility needs.

## Getting Started

To get started with Atlas:

1. Choose your application state model
2. Implement the Application and ApplicationData traits
3. Select or implement ordering and state transfer protocols
4. Configure your system
5. Build and deploy

## License

This project is licensed under the MIT License - see the LICENSE.txt file for details.

REMOVING:   libayatana-appindicator3-1  remmina  remmina-plugin-rdp  remmina-plugin-secret  remmina-plugin-vnc  update-notifier