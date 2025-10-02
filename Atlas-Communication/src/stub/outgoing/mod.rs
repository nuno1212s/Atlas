use std::collections::BTreeMap;
use std::iter;

use bytes::Bytes;

use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;

use crate::byte_stub::connections::NetworkConnectionController;
use crate::byte_stub::ByteNetworkStub;
use crate::lookup_table::ModMessageWrapped;
use crate::message::{SerializedMessage, StoredMessage, StoredSerializedMessage};
use crate::message_outgoing::{send_message_to_targets, send_serialized_message_to_target};
use crate::reconfiguration::NetworkInformationProvider;
use crate::serialization;
use crate::serialization::Serializable;
use crate::stub::{
    ApplicationStub, ModuleOutgoingStub, OperationStub, ReconfigurationStub, StateProtocolStub,
};

impl<NI, CN, BNC, R, O, S, A> ModuleOutgoingStub<R::Message>
    for ReconfigurationStub<NI, CN, BNC, R, O, S, A>
where
    NI: NetworkInformationProvider,
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    fn send(&self, message: R::Message, target: NodeId, _flush: bool) -> Result<()> {
        let wrapped_message = ModMessageWrapped::Reconfiguration(message);

        send_message_to_targets(
            &self.conn_manager,
            None,
            &self.rng,
            wrapped_message,
            iter::once(target),
        );

        Ok(())
    }

    fn send_signed(&self, message: R::Message, target: NodeId, _flush: bool) -> Result<()> {
        let wrapped_message = ModMessageWrapped::Reconfiguration(message);

        send_message_to_targets(
            &self.conn_manager,
            Some(self.network_info.get_key_pair()),
            &self.rng,
            wrapped_message,
            iter::once(target),
        );

        Ok(())
    }

    fn broadcast(
        &self,
        message: R::Message,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let wrapped_message = ModMessageWrapped::Reconfiguration(message);

        send_message_to_targets(
            &self.conn_manager,
            None,
            &self.rng,
            wrapped_message,
            targets,
        );

        Ok(())
    }

    fn broadcast_signed(
        &self,
        message: R::Message,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let wrapped_message = ModMessageWrapped::Reconfiguration(message);

        send_message_to_targets(
            &self.conn_manager,
            Some(self.network_info.get_key_pair()),
            &self.rng,
            wrapped_message,
            target,
        );

        Ok(())
    }

    fn serialize_digest_message(
        &self,
        message: R::Message,
    ) -> Result<(SerializedMessage<R::Message>, Digest)> {
        let mut buf = Vec::new();

        let digest = serialization::serialize_digest::<Vec<u8>, R>(&message, &mut buf)?;

        Ok((SerializedMessage::new(message, Bytes::from(buf)), digest))
    }

    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<R::Message>>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        messages.into_iter().for_each(|(target, message)| {
            let (header, message) = message.into_inner();

            let (message, buf) = message.into_inner();

            let wrapped_message = ModMessageWrapped::<R, O, S, A>::Reconfiguration(message);

            let serialized_message = SerializedMessage::new(wrapped_message, buf);

            send_serialized_message_to_target(
                &self.conn_manager,
                StoredMessage::new(header, serialized_message),
                target,
            );
        });

        Ok(())
    }
}

impl<NI, CN, BNC, R, O, S, A> ModuleOutgoingStub<O::Message>
    for OperationStub<NI, CN, BNC, R, O, S, A>
where
    NI: NetworkInformationProvider,
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    fn send(&self, message: O::Message, target: NodeId, _flush: bool) -> Result<()> {
        let wrapped_message = ModMessageWrapped::Protocol(message);

        send_message_to_targets(
            &self.conn_manager,
            None,
            &self.rng,
            wrapped_message,
            iter::once(target),
        );

        Ok(())
    }

    fn send_signed(&self, message: O::Message, target: NodeId, _flush: bool) -> Result<()> {
        let wrapped_message = ModMessageWrapped::Protocol(message);

        send_message_to_targets(
            &self.conn_manager,
            Some(self.network_info.get_key_pair()),
            &self.rng,
            wrapped_message,
            iter::once(target),
        );

        Ok(())
    }

    fn broadcast(
        &self,
        message: O::Message,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let wrapped_message = ModMessageWrapped::Protocol(message);

        send_message_to_targets(
            &self.conn_manager,
            None,
            &self.rng,
            wrapped_message,
            targets,
        );

        Ok(())
    }

    fn broadcast_signed(
        &self,
        message: O::Message,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let wrapped_message = ModMessageWrapped::Protocol(message);

        send_message_to_targets(
            &self.conn_manager,
            Some(self.network_info.get_key_pair()),
            &self.rng,
            wrapped_message,
            target,
        );

        Ok(())
    }

    fn serialize_digest_message(
        &self,
        message: O::Message,
    ) -> Result<(SerializedMessage<O::Message>, Digest)> {
        let mut buf = Vec::new();

        let digest = serialization::serialize_digest::<Vec<u8>, O>(&message, &mut buf)?;

        Ok((SerializedMessage::new(message, Bytes::from(buf)), digest))
    }

    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<O::Message>>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        messages.into_iter().for_each(|(target, message)| {
            let (header, message) = message.into_inner();

            let (message, buf) = message.into_inner();

            let wrapped_message = ModMessageWrapped::<R, O, S, A>::Protocol(message);

            let serialized_message = SerializedMessage::new(wrapped_message, buf);

            send_serialized_message_to_target(
                &self.conn_manager,
                StoredMessage::new(header, serialized_message),
                target,
            );
        });

        Ok(())
    }
}

impl<NI, CN, BNC, R, O, S, A> ModuleOutgoingStub<S::Message>
    for StateProtocolStub<NI, CN, BNC, R, O, S, A>
where
    NI: NetworkInformationProvider,
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    fn send(&self, message: S::Message, target: NodeId, _flush: bool) -> Result<()> {
        let wrapped_message = ModMessageWrapped::StateProtocol(message);

        send_message_to_targets(
            &self.conn_manager,
            None,
            &self.rng,
            wrapped_message,
            iter::once(target),
        );

        Ok(())
    }

    fn send_signed(&self, message: S::Message, target: NodeId, _flush: bool) -> Result<()> {
        let wrapped_message = ModMessageWrapped::StateProtocol(message);

        send_message_to_targets(
            &self.conn_manager,
            Some(self.network_info.get_key_pair()),
            &self.rng,
            wrapped_message,
            iter::once(target),
        );

        Ok(())
    }

    fn broadcast(
        &self,
        message: S::Message,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let wrapped_message = ModMessageWrapped::StateProtocol(message);

        send_message_to_targets(
            &self.conn_manager,
            None,
            &self.rng,
            wrapped_message,
            targets,
        );

        Ok(())
    }

    fn broadcast_signed(
        &self,
        message: S::Message,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let wrapped_message = ModMessageWrapped::StateProtocol(message);

        send_message_to_targets(
            &self.conn_manager,
            Some(self.network_info.get_key_pair()),
            &self.rng,
            wrapped_message,
            target,
        );

        Ok(())
    }

    fn serialize_digest_message(
        &self,
        message: S::Message,
    ) -> Result<(SerializedMessage<S::Message>, Digest)> {
        let mut buf = Vec::new();

        let digest = serialization::serialize_digest::<Vec<u8>, S>(&message, &mut buf)?;

        Ok((SerializedMessage::new(message, Bytes::from(buf)), digest))
    }

    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<S::Message>>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        messages.into_iter().for_each(|(target, message)| {
            let (header, message) = message.into_inner();

            let (message, buf) = message.into_inner();

            let wrapped_message = ModMessageWrapped::<R, O, S, A>::StateProtocol(message);

            let serialized_message = SerializedMessage::new(wrapped_message, buf);

            send_serialized_message_to_target(
                &self.conn_manager,
                StoredMessage::new(header, serialized_message),
                target,
            );
        });

        Ok(())
    }
}

impl<NI, CN, BNC, R, O, S, A> ModuleOutgoingStub<A::Message>
    for ApplicationStub<NI, CN, BNC, R, O, S, A>
where
    NI: NetworkInformationProvider,
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    fn send(&self, message: A::Message, target: NodeId, _flush: bool) -> Result<()> {
        let wrapped_message = ModMessageWrapped::Application(message);

        send_message_to_targets(
            &self.conn_manager,
            None,
            &self.rng,
            wrapped_message,
            iter::once(target),
        );

        Ok(())
    }

    fn send_signed(&self, message: A::Message, target: NodeId, _flush: bool) -> Result<()> {
        let wrapped_message = ModMessageWrapped::Application(message);

        send_message_to_targets(
            &self.conn_manager,
            Some(self.network_info.get_key_pair()),
            &self.rng,
            wrapped_message,
            iter::once(target),
        );

        Ok(())
    }

    fn broadcast(
        &self,
        message: A::Message,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let wrapped_message = ModMessageWrapped::Application(message);

        send_message_to_targets(
            &self.conn_manager,
            None,
            &self.rng,
            wrapped_message,
            targets,
        );

        Ok(())
    }

    fn broadcast_signed(
        &self,
        message: A::Message,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let wrapped_message = ModMessageWrapped::Application(message);

        send_message_to_targets(
            &self.conn_manager,
            Some(self.network_info.get_key_pair()),
            &self.rng,
            wrapped_message,
            target,
        );

        Ok(())
    }

    fn serialize_digest_message(
        &self,
        message: A::Message,
    ) -> Result<(SerializedMessage<A::Message>, Digest)> {
        let mut buf = Vec::new();

        let digest = serialization::serialize_digest::<Vec<u8>, A>(&message, &mut buf)?;

        Ok((SerializedMessage::new(message, Bytes::from(buf)), digest))
    }

    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<A::Message>>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        messages.into_iter().for_each(|(target, message)| {
            let (header, message) = message.into_inner();

            let (message, buf) = message.into_inner();

            let wrapped_message = ModMessageWrapped::<R, O, S, A>::Application(message);

            let serialized_message = SerializedMessage::new(wrapped_message, buf);

            send_serialized_message_to_target(
                &self.conn_manager,
                StoredMessage::new(header, serialized_message),
                target,
            );
        });

        Ok(())
    }
}
