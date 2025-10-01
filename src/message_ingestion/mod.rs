use anyhow::Error;
use atlas_common::node_id::NodeId;
use atlas_common::Err;
use atlas_metrics::metrics::{metric_duration, metric_store_count_max};
use std::time::Instant;
use thiserror::Error;

use crate::lookup_table::{MessageModule, PeerStubLookupTable};
use crate::message::{Header, WireMessage};
use crate::message_signing::IngestionError;
use crate::metric::{COMM_DESERIALIZE_VERIFY_TIME_ID, INCOMING_MESSAGE_SIZE_ID};
use crate::serialization::{deserialize_message, Serializable};

/// Process a message received from the byte layer of the network.
/// Requires the lookup table to be able to get the appropriate type to deserialize the message.
/// Then, stubs are retrieved from the peer stub lookup table and the message is pushed to the appropriate stub.
pub(crate) fn process_wire_message_message<R, O, S, A>(
    message: WireMessage,
    authenticated: bool,
    //network_info: &impl NetworkInformationProvider,
    // lookup_table: &impl LookupTable<R, O, S, A>,
    stubs: &impl PeerStubLookupTable<R, O, S, A>,
) -> Result<(), IngestMessageError>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    let deserialize_start_time = Instant::now();

    let (header, module, message) = message.into_inner();

    if !authenticated {
        match module {
            MessageModule::Reconfiguration => {}
            _ => {
                return Err!(IngestMessageError::UnAuthenticatedMessage(
                    module,
                    header.from()
                ));
            }
        }
    } /*else if let Err(e) = verify_ser_message_validity(network_info, &header, &message) {
          warn!(
              "Failed to verify message validity for message module: {:?}",
              module
          );

          return Err!(e);
      }*/

    metric_store_count_max(INCOMING_MESSAGE_SIZE_ID, message.len() + Header::LENGTH);

    // FIXME: Is this part with the lookup table even necessary? We just directly type in the types anyways so I think it is redundant.
    //let serialization_mod = lookup_table.get_module_for_message(&module);

    let stub = stubs.get_stub_for_message(&module);

    match &module {
        MessageModule::Reconfiguration => {
            let m = deserialize_message::<&[u8], R>(&message)?;

            stub.push_reconfiguration(header, m)?;
        }
        MessageModule::Protocol => {
            let m = deserialize_message::<&[u8], O>(&message)?;

            stub.push_protocol(header, m)?;
        }
        MessageModule::StateProtocol => {
            let m = deserialize_message::<&[u8], S>(&message)?;

            stub.push_state_protocol(header, m)?;
        }
        MessageModule::Application => {
            let m = deserialize_message::<&[u8], A>(&message)?;

            stub.push_application(header, m)?;
        }
    }

    metric_duration(
        COMM_DESERIALIZE_VERIFY_TIME_ID,
        deserialize_start_time.elapsed(),
    );

    Ok(())
}

#[derive(Debug, Error)]
pub enum IngestMessageError {
    #[error("Failed to deserialize message: {0:?}")]
    DeserializationError(#[from] Error),
    #[error("Failed to process wire message: {0:?}")]
    SignatureVerificationFailure(#[from] IngestionError),
    #[error("Attempted to process message without authenticated flag, but message was not a reconfiguration message (module: {0:?}, node {1:?})"
    )]
    UnAuthenticatedMessage(MessageModule, NodeId),
}
