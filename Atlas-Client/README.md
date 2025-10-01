# Atlas-Client

## Client Implementation for the Atlas Framework

Atlas-Client provides a robust and flexible client implementation for interacting with distributed systems built on the Atlas framework. This module enables applications to send requests to a BFT/CFT replica system and process the responses in a fault-tolerant manner.

## Features

### Fault-Tolerant Communication

- **Request retransmission**: Automatically retries failed requests
- **Response quorum verification**: Ensures responses are valid by collecting quorums
- **Server selection**: Intelligently selects replicas for optimal performance
- **View-change awareness**: Adapts to view changes in the consensus system

### Client-Side Verification

- **Response validation**: Cryptographically verifies responses from replicas
- **Quorum certificates**: Collects and validates certificates from replica quorums
- **Signature aggregation**: Support for signature aggregation schemes (e.g., BLS)

### Usability Features

- **Request batching**: Combines multiple operations into efficient batches
- **Connection pooling**: Manages connections to replicas efficiently
- **Async interface**: Modern async/await API for easy integration

## Core Components

### Client

The main client interface for interacting with the replicated system:

```rust
pub struct Client<A: Application<S>, S, NT: NetworkNodeTypes> {
    // Implementation details
}

impl<A: Application<S>, S, NT: NetworkNodeTypes> Client<A, S, NT> {
    /// Creates a new client with the provided configuration
    pub fn new(config: ClientConfig) -> Result<Self>;
    
    /// Sends a request and waits for a quorum of responses
    pub async fn send_request(&self, request: A::AppData::Request) 
        -> Result<A::AppData::Reply>;
    
    /// Sends a read-only request that can be processed without consensus
    pub async fn send_unordered_request(&self, request: A::AppData::Request) 
        -> Result<A::AppData::Reply>;
    
    // Additional methods omitted for brevity
}
```

### Client Configuration

```rust
pub struct ClientConfig {
    /// Client ID
    pub client_id: NodeId,
    
    /// List of replica addresses
    pub replicas: Vec<(NodeId, String)>,
    
    /// Cryptographic keys for secure communication
    pub keys: ClientKeys,
    
    /// Configuration for request timeouts and retries
    pub timeouts: ClientTimeoutConfig,
    
    // Additional configuration options
}
```

## Usage Examples

### Basic Request-Reply Pattern

```rust
// Initialize client with configuration
let client = Client::<MyApplication, MyState, DefaultNetworkTypes>::new(config)?;

// Create a request
let request = MyRequest::new("operation", "data");

// Send the request and await the response
let reply = client.send_request(request).await?;

// Process the response
process_reply(reply);
```

### Read-Only Requests

```rust
// For read-only operations that don't modify state
let read_request = MyRequest::new("read", "data");

// Use unordered execution for better performance
let reply = client.send_unordered_request(read_request).await?;
```

### Session Management

```rust
// Start a session for related operations
let session = client.start_session().await?;

// Send multiple requests in the same session
let reply1 = session.send_request(request1).await?;
let reply2 = session.send_request(request2).await?;

// End the session
session.end().await?;
```

## Configuration Options

### Request Handling

- **Request timeout**: Maximum time to wait for responses
- **Retry policy**: Strategy for retrying failed requests
- **Concurrency limit**: Maximum concurrent requests

### Replica Selection

- **Selection strategy**: Round-robin, latency-based, or random
- **Blacklisting**: Temporary exclusion of failing replicas
- **Preferred replicas**: Prioritization of certain replicas

### Security Settings

- **TLS configuration**: Certificate and key settings
- **Signature scheme**: Choice of cryptographic signature algorithm
- **Signature verification**: Controls for response verification

## Integration with Atlas

Atlas-Client integrates with other Atlas components:
[Cargo.toml](Cargo.toml)
- **Atlas-Communication**: Uses the network layer for communication
- **Atlas-Common**: Leverages common utilities and abstractions
- **Atlas-SMR-Application**: Works with application-specific request/reply types

## Performance Considerations

- **Request batching**: Group related requests for efficiency
- **Connection reuse**: Keep connections open for multiple requests
- **Response caching**: Cache responses for read-only queries
- **Asynchronous processing**: Use non-blocking operations for better throughput

## Advanced Features

- **Speculative execution**: Support for speculative request execution
- **Request pipelining**: Sending multiple requests without waiting for responses
- **Load balancing**: Distributing requests across replicas
- **Monitoring**: Collection of performance metrics

## License

This module is licensed under the MIT License - see the LICENSE.txt file for details.
