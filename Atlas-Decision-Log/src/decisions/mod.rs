use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_core::ordering_protocol::{
    DecisionAD, DecisionMetadata, ProtocolConsensusDecision, ShareableConsensusMessage,
};
use atlas_logging_core::decision_log::LoggingDecision;

/// A struct to store the ongoing decision known parameters
pub struct OnGoingDecision<RQ, OP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
{
    // The seq number of this decision
    seq: SeqNo,
    // Whether this decision has been marked as completed by the ordering protocol
    completed: bool,
    // The metadata of the decision, optional since it's usually the
    metadata: Option<DecisionMetadata<RQ, OP>>,
    additional_data: Vec<DecisionAD<RQ, OP>>,
    // The messages that compose this decision, to be transformed into a given proof
    messages: Vec<ShareableConsensusMessage<RQ, OP>>,
    // The decision information from the ordering protocol
    protocol_decision: Option<ProtocolConsensusDecision<RQ>>,
    // The information about the decision that is being logged.
    // This is what is going to be used to send to the persistent
    // Logging layer in order to better control when a given sequence
    // number is completely persisted
    logging_decision: LoggingDecision,
}

/// The completed decision object with all necessary information to be transformed
/// into a proof, which will be put into the decision log
pub struct CompletedDecision<RQ, OP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
{
    seq: SeqNo,
    metadata: DecisionMetadata<RQ, OP>,
    additional_data: Vec<DecisionAD<RQ, OP>>,
    messages: Vec<ShareableConsensusMessage<RQ, OP>>,
    protocol_decision: ProtocolConsensusDecision<RQ>,
    logged_info: LoggingDecision,
}

impl<RQ, OP> Orderable for OnGoingDecision<RQ, OP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
{
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl<RQ, OP> OnGoingDecision<RQ, OP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
{
    pub fn init(seq: SeqNo) -> Self {
        Self {
            seq,
            completed: false,
            metadata: None,
            additional_data: vec![],
            messages: vec![],
            protocol_decision: None,
            logging_decision: LoggingDecision::init_empty(seq),
        }
    }

    pub fn insert_metadata(&mut self, metadata: DecisionMetadata<RQ, OP>) {
        let _ = self.metadata.insert(metadata);
    }

    pub fn insert_additional_data(&mut self, data: DecisionAD<RQ, OP>) {
        self.additional_data.push(data);
    }

    pub fn insert_component_message(&mut self, partial: ShareableConsensusMessage<RQ, OP>) {
        self.logging_decision.insert_message::<RQ, OP>(&partial);

        self.messages.push(partial);
    }

    pub fn insert_requests(&mut self, protocol_decision: ProtocolConsensusDecision<RQ>) {
        self.protocol_decision = Some(protocol_decision)
    }

    pub fn completed(&mut self) {
        self.completed = true;
    }

    pub fn is_completed(&self) -> bool {
        self.completed
    }

    pub fn into_completed_decision(self) -> CompletedDecision<RQ, OP> {
        CompletedDecision {
            seq: self.seq,
            metadata: self.metadata.unwrap(),
            additional_data: self.additional_data,
            messages: self.messages,
            protocol_decision: self.protocol_decision.unwrap(),
            logged_info: self.logging_decision,
        }
    }
}

#[allow(clippy::type_complexity)]
impl<RQ, OP> CompletedDecision<RQ, OP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
{
    pub fn into(
        self,
    ) -> (
        SeqNo,
        DecisionMetadata<RQ, OP>,
        Vec<DecisionAD<RQ, OP>>,
        Vec<ShareableConsensusMessage<RQ, OP>>,
        ProtocolConsensusDecision<RQ>,
        LoggingDecision,
    ) {
        (
            self.seq,
            self.metadata,
            self.additional_data,
            self.messages,
            self.protocol_decision,
            self.logged_info,
        )
    }
}
