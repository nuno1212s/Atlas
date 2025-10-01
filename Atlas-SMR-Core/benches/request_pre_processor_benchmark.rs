#[divan::bench_group()]
mod pre_processor_benches {
    use atlas_common::channel;
    use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
    use atlas_common::crypto::hash::Digest;
    use atlas_common::node_id::NodeId;
    use atlas_common::ordering::SeqNo;
    use atlas_communication::lookup_table::MessageModule;
    use atlas_communication::message::{Buf, StoredMessage, WireMessage};
    use atlas_core::request_pre_processing::PreProcessorOutput;
    use atlas_smr_application::serialize::ApplicationData;
    use atlas_smr_core::message::OrderableMessage;
    use atlas_smr_core::request_pre_processing::worker::{
        PreProcessorWorkMessage, PreProcessorWorkMessageOuter, RequestPreProcessingWorker,
    };
    use atlas_smr_core::SMRReq;
    use divan::Bencher;
    #[cfg(feature = "serialize_serde")]
    use serde::{Deserialize, Serialize};
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, OnceLock};
    use std::thread;

    #[derive(Default, Clone, Debug)]
    #[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
    struct MockAppData;

    impl ApplicationData for MockAppData {
        type Request = MockAppData;
        type Reply = MockAppData;

        fn serialize_request<W>(_w: W, _request: &Self::Request) -> atlas_common::error::Result<()>
        where
            W: Write,
        {
            todo!()
        }

        fn deserialize_request<R>(_r: R) -> atlas_common::error::Result<Self::Request>
        where
            R: Read,
        {
            todo!()
        }

        fn serialize_reply<W>(_w: W, _reply: &Self::Reply) -> atlas_common::error::Result<()>
        where
            W: Write,
        {
            todo!()
        }

        fn deserialize_reply<R>(_r: R) -> atlas_common::error::Result<Self::Reply>
        where
            R: Read,
        {
            todo!()
        }
    }

    #[allow(dead_code)]
    struct PreProcessorHandles<D>
    where
        D: ApplicationData + 'static,
    {
        message_tx: ChannelSyncTx<PreProcessorWorkMessageOuter<D>>,
        ordered_rx: ChannelSyncRx<PreProcessorOutput<SMRReq<D>>>,
        unordered_rx: ChannelSyncRx<PreProcessorOutput<SMRReq<D>>>,
    }

    #[allow(dead_code)]
    fn setup_worker<D>() -> (RequestPreProcessingWorker<D>, PreProcessorHandles<D>)
    where
        D: ApplicationData + 'static,
    {
        let (message_channel_tx, message_channel_rx) =
            channel::sync::new_bounded_sync(1, Some("RequestPreProcessor"));
        let (ordered_tx, ordered_rx) =
            channel::sync::new_unbounded_sync(Some("Ordered Pre Processor"));
        let (unordered_tx, unordered_rx) =
            channel::sync::new_unbounded_sync(Some("Unordered Pre Processor"));

        let worker =
            RequestPreProcessingWorker::new(0, message_channel_rx, ordered_tx, unordered_tx);

        (
            worker,
            PreProcessorHandles {
                message_tx: message_channel_tx,
                ordered_rx,
                unordered_rx,
            },
        )
    }

    static HANDLES: OnceLock<PreProcessorHandles<MockAppData>> = OnceLock::new();
    static RUNNING: OnceLock<Arc<AtomicBool>> = OnceLock::new();

    #[allow(dead_code)]
    fn setup() {
        if RUNNING.get().is_some() {
            return;
        }

        let (worker, handles) = setup_worker::<MockAppData>();

        let running_flag = Arc::new(AtomicBool::new(true));

        RUNNING
            .set(running_flag.clone())
            .expect("Failed to set running flag");

        thread::spawn(move || {
            let worker = worker;

            //TODO: Make this use the running flag
            worker.run();
        });

        HANDLES
            .set(handles)
            .map_err(|_| ())
            .expect("Failed to set handles");
    }

    #[allow(dead_code)]
    #[divan::bench(args = [(1000, 100), (10000, 1000), (100_000, 10000)])]
    #[ignore]
    fn benchmark_pre_processor(_bencher: Bencher, requests: (u32, u32)) {
        //TODO: Actually write relevant code for the pre-processor worker
        let handles = HANDLES.get().unwrap();

        let (requests, _requests_per_batch) = requests;

        let req = SMRReq::<MockAppData>::new(SeqNo::ZERO, SeqNo::ZERO, MockAppData);

        let wire_message = WireMessage::new(
            NodeId(1000),
            NodeId(0),
            MessageModule::Application,
            Buf::new(),
            0,
            Some(Digest::blank()),
            None,
        );

        let work_msg =
            PreProcessorWorkMessage::ClientPoolRequestsReceived(vec![StoredMessage::new(
                *wire_message.header(),
                OrderableMessage::OrderedRequest(req),
            )]);

        let work_msg = PreProcessorWorkMessageOuter::new(work_msg);

        for _ in 0..requests {
            handles.message_tx.send(work_msg.clone()).unwrap();
        }
        for _ in 0..requests {
            let _ = handles.ordered_rx.recv().unwrap();
        }
    }
}

fn main() {
    divan::main();
}
