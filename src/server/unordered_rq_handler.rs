use crate::metric::UNORDERED_OPS_PER_SECOND_ID;
use atlas_common::error::Result;
use atlas_common::quiet_unwrap;
use atlas_core::executor::DecisionExecutorHandle;
use atlas_metrics::metrics::metric_increment;
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_core::request_pre_processing::UnorderedRqHandles;
use atlas_smr_core::SMRReq;
use tracing::error;

pub(super) fn start_unordered_rq_thread<O: ApplicationData>(
    unordered_rqs: UnorderedRqHandles<SMRReq<O>>,
    executor_handle: impl DecisionExecutorHandle<SMRReq<O>> + Send,
) -> Result<()> {
    std::thread::Builder::new()
        .name("Unordered-RQ-Passer".to_string())
        .spawn(move || unordered_rq_thread::<O>(unordered_rqs, executor_handle))?;

    Ok(())
}

fn unordered_rq_thread<O: ApplicationData>(
    unordered_rqs: UnorderedRqHandles<SMRReq<O>>,
    executor_handle: impl DecisionExecutorHandle<SMRReq<O>>,
) {
    while let Ok(work_message) = unordered_rqs.recv() {
        let op_count = work_message.len();

        quiet_unwrap!(executor_handle.queue_update_unordered(work_message.into()));

        metric_increment(UNORDERED_OPS_PER_SECOND_ID, Some(op_count as u64));
    }
}
