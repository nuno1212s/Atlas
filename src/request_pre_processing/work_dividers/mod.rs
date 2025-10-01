use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::Header;

use crate::messages::{ClientRqInfo, SessionBased};
use crate::request_pre_processing::{operation_key_raw, WorkPartitioner};

pub struct WDRoundRobin;

impl WorkPartitioner for WDRoundRobin {
    fn get_worker_for<O>(rq_info: &Header, message: &O, worker_count: usize) -> usize
    where
        O: SessionBased,
    {
        let op_key = operation_key_raw(rq_info.from(), message.session_number());

        (op_key % worker_count as u64) as usize
    }

    fn get_worker_for_processed(rq_info: &ClientRqInfo, worker_count: usize) -> usize {
        Self::get_worker_for_raw(rq_info.sender(), rq_info.session_number(), worker_count)
    }

    fn get_worker_for_raw(from: NodeId, session: SeqNo, worker_count: usize) -> usize {
        let op_key = operation_key_raw(from, session);

        (op_key % worker_count as u64) as usize
    }
}
