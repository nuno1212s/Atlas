use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::iter;

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use atlas_comm_mio::config::{MIOConfig, TcpConfig, TlsConfig};
use atlas_comm_mio::ByteStubType;
use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::peer_addr::PeerAddr;
use atlas_communication::byte_stub::{NodeIncomingStub, NodeStubController};
use atlas_communication::message::WireMessage;
use atlas_communication::reconfiguration;
use atlas_communication::reconfiguration::NetworkInformationProvider;
use getset::Getters;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pemfile::{read_one, Item};
use thiserror::Error;
use tracing::{debug, info};

#[derive(Clone, Getters)]
struct MockStubController {
    #[get = "pub"]
    node_id: NodeId,

    stubs: Arc<Mutex<BTreeMap<NodeId, MockStub>>>,
    /// Channel to push received messages from the byte network layer
    input_tx: ChannelSyncTx<WireMessage>,
    /// Channel to receive messages from the byte network layer
    #[get = "pub(crate)"]
    input_rx: ChannelSyncRx<WireMessage>,
}

#[derive(Clone)]
struct MockStubInput(NodeId, ChannelSyncTx<WireMessage>);

#[derive(Clone)]
struct MockStubOutput(ByteStubType);

#[derive(Clone)]
struct MockStub(MockStubInput, MockStubOutput);

#[derive(Error, Debug)]
#[error("A mock error definition")]
struct MockError;

impl NodeIncomingStub for MockStubInput {
    type Error = MockError;

    fn handle_message<NI>(
        &self,
        _network_info: &Arc<NI>,
        message: WireMessage,
    ) -> Result<(), MockError>
    where
        NI: NetworkInformationProvider + 'static,
    {
        debug!("{:?} // Received message: {:?}", self.0, message.header());

        self.1.send(message).map_err(|_| MockError)
    }
}

impl MockStubController {
    fn new(node_id: NodeId) -> Self {
        let (tx, rx) = channel::sync::new_bounded_sync(32, Some("MockStubController"));
        let stubs = Arc::new(Mutex::new(Default::default()));

        Self {
            node_id,
            stubs,
            input_tx: tx,
            input_rx: rx,
        }
    }

    fn create_stub_for(&self, node: NodeId) -> MockStubInput {
        let tx = self.input_tx.clone();

        MockStubInput(node, tx)
    }

    fn output_stub_for(&self, node: &NodeId) -> Option<MockStubOutput> {
        self.stubs
            .lock()
            .unwrap()
            .get(node)
            .map(|stub| stub.1.clone())
    }
}

impl NodeStubController<ByteStubType, MockStubInput> for MockStubController {
    type Error = MockError;

    fn has_stub_for(&self, node: &NodeId) -> bool {
        self.stubs.lock().unwrap().contains_key(node)
    }

    fn generate_stub_for(
        &self,
        node: NodeId,
        byte_stub: ByteStubType,
    ) -> Result<MockStubInput, MockError> {
        let input_stub = self.create_stub_for(node);

        let output_stub = MockStubOutput(byte_stub);

        let stub = MockStub(input_stub.clone(), output_stub);

        self.stubs
            .lock()
            .expect("Failed to lock stubs mutex")
            .insert(node, stub);

        Ok(input_stub)
    }

    fn get_stub_for(&self, node: &NodeId) -> Option<MockStubInput> {
        if self.node_id == *node {
            return Some(MockStubInput(*node, self.input_tx.clone()));
        }

        self.stubs
            .lock()
            .unwrap()
            .get(node)
            .map(|stub| stub.0.clone())
    }

    fn shutdown_stubs_for(&self, node: &NodeId) -> Result<(), MockError> {
        self.stubs.lock().unwrap().remove(node);

        Ok(())
    }
}

#[inline]
fn read_private_keys_from_file(
    mut file: BufReader<File>,
) -> atlas_common::error::Result<Vec<PrivateKeyDer<'static>>> {
    let mut certs = Vec::new();

    for item in iter::from_fn(|| read_one(&mut file).transpose()) {
        match item.context("Failed to read private key from file")? {
            Item::Pkcs1Key(rsa) => certs.push(PrivateKeyDer::Pkcs1(rsa)),
            Item::Pkcs8Key(rsa) => certs.push(PrivateKeyDer::Pkcs8(rsa)),
            Item::Sec1Key(rsa) => certs.push(PrivateKeyDer::Sec1(rsa)),
            _ => {
                return Err(anyhow!("Certificate given in place of a key"));
            }
        }
    }

    Ok(certs)
}

fn read_certificates_from_file(
    mut file: &mut BufReader<File>,
) -> atlas_common::error::Result<Vec<CertificateDer<'static>>> {
    let mut certs = Vec::new();

    for item in iter::from_fn(|| read_one(&mut file).transpose()) {
        match item.context("Failed to read certificate from file")? {
            Item::X509Certificate(cert) => {
                certs.push(cert);
            }
            _ => {
                return Err(anyhow!("Key given in place of a certificate"));
            }
        }
    }

    Ok(certs)
}

fn default_config(node: u32) -> atlas_common::error::Result<MIOConfig> {
    info!("Loading configuration for node {}", node);
    info!("Current directory: {:?}", std::env::current_dir()?);

    let mut root_store = RootCertStore::empty();

    let cert = {
        let mut file = BufReader::new(
            File::open("./tests/ca-root/crt")
                .context("Failed to open certificate file")
                .context(format!("Current dir: {:?}", std::env::current_dir()))?,
        );
        let certs = read_certificates_from_file(&mut file)?;

        root_store
            .add(certs[0].clone())
            .context("Failed to add certificate to root store")?;

        certs
    };

    let chain = {
        let mut file = BufReader::new(
            File::open(format!("./tests/ca-root/srv{node}/chain"))
                .context("Failed to open certificate file")
                .context(format!("Current dir: {:?}", std::env::current_dir()))?,
        );
        let mut certs = read_certificates_from_file(&mut file)?;

        certs.extend(cert);

        certs
    };

    let sk = {
        let file = BufReader::new(
            File::open(format!("./tests/ca-root/srv{node}/key"))
                .context("Failed to open certificate file")
                .context(format!("Current dir: {:?}", std::env::current_dir()))?,
        );

        read_private_keys_from_file(file)?
            .pop()
            .ok_or(anyhow!("No private key found"))?
    };

    let client_config = ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, sk)
        .unwrap();

    Ok(MIOConfig {
        epoll_worker_count: 2,
        tcp_configs: TcpConfig {
            bind_addrs: None,
            network_config: TlsConfig {
                async_client_config: client_config.clone(),
                async_server_config: server_config.clone(),
                sync_server_config: server_config,
                sync_client_config: client_config,
            },
            replica_concurrent_connections: 1,
            client_concurrent_connections: 1,
        },
    })
}

#[derive(Getters, Clone)]
pub struct NodeInfo<K> {
    #[get = "pub(crate)"]
    node_info: reconfiguration::NodeInfo,
    #[get = "pub(crate)"]
    key: K,
}

pub struct MockNetworkInfo {
    own_node: NodeInfo<Arc<KeyPair>>,
    other_nodes: BTreeMap<NodeId, NodeInfo<PublicKey>>,
}

impl NetworkInformationProvider for MockNetworkInfo {
    fn own_node_info(&self) -> &reconfiguration::NodeInfo {
        &self.own_node.node_info
    }

    fn get_key_pair(&self) -> &Arc<KeyPair> {
        &self.own_node.key
    }

    fn get_node_info(&self, node: &NodeId) -> Option<reconfiguration::NodeInfo> {
        self.other_nodes
            .get(node)
            .map(|info| info.node_info.clone())
    }
}

struct MockNetworkInfoFactory {
    nodes: BTreeMap<NodeId, NodeInfo<Arc<KeyPair>>>,
}

impl MockNetworkInfoFactory {
    fn initialize_for(node_count: usize, port: u16) -> atlas_common::error::Result<Self> {
        let buf = [0; 32];
        let mut map = BTreeMap::default();

        for node_id in 0..node_count {
            let node_id = u32::try_from(node_id).expect("Failed to convert node_id to u32");
            let key = KeyPair::from_bytes(buf.as_slice())?;

            let info = NodeInfo {
                node_info: reconfiguration::NodeInfo::new(
                    NodeId::from(node_id),
                    NodeType::Replica,
                    PublicKey::from(key.public_key()),
                    PeerAddr::new(
                        format!("127.0.0.1:{}", port + (node_id as u16)).parse()?,
                        String::from("localhost"),
                    ),
                ),
                key: Arc::new(key),
            };

            map.insert(NodeId(node_id), info);
        }

        Ok(Self { nodes: map })
    }

    fn generate_network_info_for(
        &self,
        node_id: NodeId,
    ) -> atlas_common::error::Result<MockNetworkInfo> {
        let own_network_id = self
            .nodes
            .get(&node_id)
            .ok_or(anyhow!("Node not found"))?
            .clone();

        let other_nodes: BTreeMap<NodeId, NodeInfo<PublicKey>> = self
            .nodes
            .iter()
            .filter(|(id, _)| **id != node_id)
            .map(|(id, info)| {
                (
                    *id,
                    NodeInfo {
                        node_info: info.node_info.clone(),
                        key: PublicKey::from(info.key.public_key()),
                    },
                )
            })
            .collect();

        Ok(MockNetworkInfo {
            own_node: own_network_id,
            other_nodes,
        })
    }
}

#[cfg(test)]
mod conn_test {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::{
        default_config, MockNetworkInfo, MockNetworkInfoFactory, MockStubController, MockStubInput,
    };
    use anyhow::{anyhow, Context};
    use atlas_comm_mio::MIOTCPNode;
    use atlas_common::error::*;
    use atlas_common::node_id::NodeId;
    use atlas_communication::byte_stub::connections::NetworkConnectionController;
    use atlas_communication::byte_stub::{
        ByteNetworkController, ByteNetworkControllerInit, ByteNetworkStub,
    };
    use atlas_communication::lookup_table::MessageModule;
    use atlas_communication::message::WireMessage;
    use bytes::Bytes;
    use tracing::{debug, info, warn};
    use tracing_test::traced_test;

    fn initialize_node_set(
        node_count: u32,
        port: u16,
    ) -> Result<BTreeMap<NodeId, MIOTCPNode<MockNetworkInfo, MockStubInput, MockStubController>>>
    {
        let factory = MockNetworkInfoFactory::initialize_for(node_count as usize, port)?;

        let mut nodes = BTreeMap::default();

        for node in 0..node_count {
            // Initialize all of the nodes
            let node = NodeId::from(node);

            let network_info = factory.generate_network_info_for(node)?;

            let mock_stub_controller = MockStubController::new(node);

            let nt_node = MIOTCPNode::initialize_controller(
                Arc::new(network_info),
                default_config(node.0)?,
                mock_stub_controller,
            )
            .context(format!("Failed to initialize node {node:?}"))?;

            nodes.insert(node, nt_node);
        }

        Ok(nodes)
    }

    #[test]
    pub fn test_message_mod_serialization() -> Result<()> {
        let msg_mod = MessageModule::Protocol;

        let vec = bincode::serde::encode_to_vec(&msg_mod, bincode::config::standard())?;

        let mod_bytes = Bytes::from(vec);

        println!("Mod bytes {mod_bytes:x?}");

        let (de_ser_msg_mod, _msg_mod_size): (MessageModule, usize) =
            bincode::serde::decode_from_slice(mod_bytes.as_ref(), bincode::config::standard())?;

        assert_eq!(de_ser_msg_mod, msg_mod);

        Ok(())
    }

    #[test]
    #[traced_test]
    pub fn test_conn() -> Result<()> {
        const NODE_COUNT: u32 = 3;
        const TEST_PORT: u16 = 11000;

        debug!("Initializing node set");

        let nodes = initialize_node_set(NODE_COUNT, TEST_PORT)?;

        warn!("Node set initialized, making connections");

        for node in 0..NODE_COUNT {
            let node_id = NodeId::from(node);

            info!("Connecting node {:?}", node_id);

            let node_ref = nodes.get(&node_id).ok_or(anyhow!("Node not found"))?;

            // Since a <-> b => b <-> a, then 1 only needs to connect to 2, and 2 only needs to connect to 3
            for other_node in node + 1..NODE_COUNT {
                let other_node_id = NodeId::from(other_node);

                info!("Connecting to node {:?}", other_node_id);

                for result_wait in node_ref
                    .connection_controller()
                    .connect_to_node(other_node_id)?
                {
                    result_wait.recv()?.context(format!(
                        "Failed to receive result of connecting to node {other_node_id:?}"
                    ))?;
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(50));

        for node in 0..NODE_COUNT {
            let node_ref = nodes.get(&NodeId::from(node)).unwrap();

            for other_node in 0..NODE_COUNT {
                if other_node == node {
                    continue;
                }

                assert!(
                    node_ref
                        .connection_controller()
                        .has_connection(&NodeId::from(other_node)),
                    "Node {node:?} does not have connection to node {other_node:?}"
                );
            }
        }

        Ok(())
    }

    #[test]
    #[traced_test]
    pub fn test_msg_delivery() -> Result<()> {
        let nodes = initialize_node_set(2, 12000)?;

        let node_0_id = NodeId::from(0u32);

        let node_1_id = NodeId::from(1u32);

        let result = nodes
            .get(&node_0_id)
            .unwrap()
            .connection_controller()
            .connect_to_node(node_1_id);

        result.into_iter().for_each(|result| {
            result.into_iter().for_each(|result| {
                result
                    .recv()
                    .unwrap()
                    .context("Failed to receive result of connecting to node 1")
                    .unwrap();
            });
        });

        let node = nodes.get(&node_0_id).unwrap();
        let node_1 = nodes.get(&node_1_id).unwrap();

        let msg = Bytes::from("Hello, world!");

        let mut digest = atlas_common::crypto::hash::Context::new();

        digest.update(msg.as_ref());

        let digest = digest.finish();

        let output_tx = node
            .stub_controller()
            .output_stub_for(&node_1_id)
            .ok_or(anyhow!("Failed to get output stub for node 1"))?;

        let wire_msg = WireMessage::new(
            node_0_id,
            node_1_id,
            MessageModule::Reconfiguration,
            msg.clone(),
            0,
            Some(digest),
            None,
        );

        output_tx.0.dispatch_message(wire_msg)?;

        let message = node_1
            .stub_controller()
            .input_rx()
            .recv()
            .context("Failed to receive message from node 0")?;

        assert_eq!(*(message.header().digest()), digest);
        assert_eq!(message.payload(), msg.as_ref());

        Ok(())
    }
}
