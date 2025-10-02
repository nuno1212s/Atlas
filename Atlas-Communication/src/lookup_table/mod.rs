use std::marker::PhantomData;
use std::sync::Arc;

use enum_map::{enum_map, Enum, EnumMap};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use strum::EnumIter;

use atlas_common::error::*;

use crate::byte_stub::incoming::InternalStubTX;
use crate::message::Header;
use crate::serialization::Serializable;

#[derive(Clone, Debug, EnumIter, Enum, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageModule {
    Reconfiguration,
    Protocol,
    StateProtocol,
    Application,
}

#[derive(Clone)]
pub enum ModMessageWrapped<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    Reconfiguration(R::Message),
    Protocol(O::Message),
    StateProtocol(S::Message),
    Application(A::Message),
}

/// A trait that defines the behaviour of the lookup table
pub trait LookupTable<R, O, S, A>: Send + Sync + Clone
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    /// Returns the serialization type that is responsible for handling messages of the given module.
    fn get_module_for_message(
        &self,
        module: &MessageModule,
    ) -> &MessageModuleSerialization<R, O, S, A>;
}

/// The look up table to get the appropriate stub for a given message.
pub trait PeerStubLookupTable<R, O, S, A>: Send + Sync + Clone
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn get_stub_for_message(&self, module: &MessageModule) -> &MessageInputStubs<R, O, S, A>;
}

#[derive(Clone)]
pub enum MessageModuleSerialization<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    Reconfiguration(PhantomData<fn() -> R>),
    Protocol(PhantomData<fn() -> O>),
    StateProtocol(PhantomData<fn() -> S>),
    Application(PhantomData<fn() -> A>),
}

/// A wrapper for the various types of stubs we can have.
/// We require this intermediate layer because we need
/// to be able to node the type of the stub we are dealing with
/// and that can only be done if the compiler has all the information at compile time already.
///
/// So this enum is a necessary evil, along with its associated methods due to the type system.
#[derive(Clone)]
pub enum MessageInputStubs<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    Reconfiguration(InternalStubTX<R::Message>),
    Protocol(InternalStubTX<O::Message>),
    StateProtocol(InternalStubTX<S::Message>),
    Application(InternalStubTX<A::Message>),
}

/// An enum map based lookup table.
///
/// This should go very fasttttttt.
pub struct EnumLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    enum_map: Arc<EnumMap<MessageModule, MessageModuleSerialization<R, O, S, A>>>,
}

impl<R, O, S, A> EnumLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub fn init() -> Self {
        let map = enum_map! {
                MessageModule::Reconfiguration => MessageModuleSerialization::Reconfiguration(Default::default()),
                MessageModule::Protocol => MessageModuleSerialization::Protocol(Default::default()),
                MessageModule::StateProtocol => MessageModuleSerialization::StateProtocol(Default::default()),
                MessageModule::Application => MessageModuleSerialization::Application(Default::default()),
        };

        Self {
            enum_map: Arc::new(map),
        }
    }
}

impl<R, O, S, A> ModMessageWrapped<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub fn get_module(&self) -> MessageModule {
        match self {
            ModMessageWrapped::Reconfiguration(_) => MessageModule::Reconfiguration,
            ModMessageWrapped::Protocol(_) => MessageModule::Protocol,
            ModMessageWrapped::StateProtocol(_) => MessageModule::StateProtocol,
            ModMessageWrapped::Application(_) => MessageModule::Application,
        }
    }
}

impl<R, O, S, A> LookupTable<R, O, S, A> for EnumLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn get_module_for_message(
        &self,
        module: &MessageModule,
    ) -> &MessageModuleSerialization<R, O, S, A> {
        &self.enum_map[module.clone()]
    }
}

impl<R, O, S, A> MessageInputStubs<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub(crate) fn push_reconfiguration(&self, header: Header, message: R::Message) -> Result<()> {
        match self {
            MessageInputStubs::Reconfiguration(reconf_stub) => {
                reconf_stub.handle_message(header, message)
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn push_protocol(&self, header: Header, message: O::Message) -> Result<()> {
        match self {
            MessageInputStubs::Protocol(protocol_stub) => {
                protocol_stub.handle_message(header, message)
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn push_state_protocol(&self, header: Header, message: S::Message) -> Result<()> {
        match self {
            MessageInputStubs::StateProtocol(state_protocol_stub) => {
                state_protocol_stub.handle_message(header, message)
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn push_application(&self, header: Header, message: A::Message) -> Result<()> {
        match self {
            MessageInputStubs::Application(application_stub) => {
                application_stub.handle_message(header, message)
            }
            _ => unreachable!(),
        }
    }
}

impl<R, O, S, A> Clone for EnumLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn clone(&self) -> Self {
        Self {
            enum_map: self.enum_map.clone(),
        }
    }
}

impl<R, O, S, A> Default for EnumLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn default() -> Self {
        Self::init()
    }
}
