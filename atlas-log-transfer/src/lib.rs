use atlas_common::ordering::SeqNo;
use atlas_core::state_transfer::log_transfer::StatefulOrderProtocol;
use atlas_execution::serialize::SharedData;

pub mod messages;


pub struct CollabLogTransfer<D, OP, NT, PL>
where D: SharedData + 'static, OP: StatefulOrderProtocol<D, NT, PL>

{

    curr_seq: SeqNo,

}