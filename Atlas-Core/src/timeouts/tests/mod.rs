#![cfg(test)]
pub mod timeouts_tests {

    pub use crate::timeouts::timeout::*;
    pub use crate::timeouts::worker::*;
    pub use crate::timeouts::*;

    use std::sync::Arc;
    use std::sync::LazyLock;
    use std::time::Duration;
    use tracing_appender::non_blocking::WorkerGuard;

    use atlas_common::channel::sync::{new_bounded_sync, ChannelSyncRx, ChannelSyncTx};
    use atlas_common::node_id::NodeId;
    use atlas_common::ordering::SeqNo;

    const OUR_ID: NodeId = NodeId(0);
    const ID_1: NodeId = NodeId(1);

    static MOD_NAME: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("TestMod"));
    static IDS: LazyLock<Vec<NodeId>> = LazyLock::new(|| vec![OUR_ID, ID_1]);
    static TIMEOUT_IDS: LazyLock<Vec<TimeoutIdentification>> = LazyLock::new(|| {
        vec![TimeoutIdentification {
            mod_id: MOD_NAME.clone(),
            timeout_id: TimeoutID::SessionBased {
                session: SeqNo::ZERO,
                seq_no: SeqNo::ZERO,
                from: OUR_ID,
            },
        }]
    });

    fn setup_tracing_subscriber() -> WorkerGuard {
        let (console_nb, guard_3) = tracing_appender::non_blocking(std::io::stdout());

        let _ = tracing_subscriber::fmt::fmt()
            .with_writer(console_nb)
            .try_init();

        guard_3
    }

    #[derive(Clone)]
    struct TimeoutTx {
        tx: ChannelSyncTx<Vec<Timeout>>,
    }

    impl TimeoutWorkerResponder for TimeoutTx {
        fn report_timeouts(&self, timeouts: Vec<Timeout>) -> atlas_common::error::Result<()> {
            self.tx
                .send(timeouts)
                .context("Failed to send timeouts to the test receiver")
        }
    }

    fn init_timeout_rx() -> (TimeoutTx, ChannelSyncRx<Vec<Timeout>>) {
        let (tx, rx) = new_bounded_sync(128, Some("TimeoutRx"));
        (TimeoutTx { tx }, rx)
    }

    fn setup_timeouts() -> (TimeoutsHandle, ChannelSyncRx<Vec<Timeout>>, WorkerGuard) {
        let guard = setup_tracing_subscriber();

        let (tx, rx) = init_timeout_rx();

        let handle = initialize_timeouts(OUR_ID, 1, 10, tx);

        (handle, rx, guard)
    }

    fn start_timeout_with(
        handle: &TimeoutsHandle,
        timeout_id: TimeoutIdentification,
        needed_acks: usize,
        cumulative: bool,
    ) {
        handle
            .request_timeout(
                timeout_id,
                None,
                Duration::from_secs(1),
                needed_acks,
                cumulative,
            )
            .unwrap();
    }

    fn start_timeout(timeout_id: TimeoutIdentification, handle: &TimeoutsHandle) {
        start_timeout_with(handle, timeout_id, 1, false);
    }

    fn ack_timeout(timeout_id: TimeoutIdentification, from: NodeId, handle: &TimeoutsHandle) {
        handle.ack_received(timeout_id, from).unwrap();
    }

    fn cancel_timeout(timeout_id: TimeoutIdentification, handle: &TimeoutsHandle) {
        handle.cancel_timeout(timeout_id).unwrap();
    }

    #[test]
    fn test_timeout_session_based() {
        let (handle, timeout_rx, _) = setup_timeouts();

        start_timeout(TIMEOUT_IDS[0].clone(), &handle);

        let timeouts = timeout_rx.recv().unwrap();

        assert_eq!(timeouts.len(), 1);

        assert_eq!(timeouts[0].id.timeout_id, TIMEOUT_IDS[0].timeout_id);
    }

    #[test]
    fn test_timeout_ack() {
        let (handle, timeout_rx, _) = setup_timeouts();

        start_timeout(TIMEOUT_IDS[0].clone(), &handle);

        ack_timeout(TIMEOUT_IDS[0].clone(), OUR_ID, &handle);

        std::thread::sleep(Duration::from_secs(2));

        let timeouts = timeout_rx.try_recv();

        assert!(timeouts.is_err());
    }

    #[test]
    fn test_multiple_ack_not_received() {
        let (handle, timeout_rx, _) = setup_timeouts();

        start_timeout_with(&handle, TIMEOUT_IDS[0].clone(), 2, false);

        ack_timeout(TIMEOUT_IDS[0].clone(), OUR_ID, &handle);

        let timeouts = timeout_rx.recv().unwrap();

        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].id.timeout_id, TIMEOUT_IDS[0].timeout_id);
    }

    #[test]
    fn test_multiple_acks_received() {
        let (handle, timeout_rx, _) = setup_timeouts();

        let acks = 2;

        start_timeout_with(&handle, TIMEOUT_IDS[0].clone(), acks, false);

        for id in 0..acks {
            ack_timeout(TIMEOUT_IDS[0].clone(), IDS[id], &handle);
        }

        std::thread::sleep(Duration::from_secs(2));

        let timeouts = timeout_rx.try_recv();

        assert!(timeouts.is_err());
    }

    #[test]
    fn test_duplicate_acks() {
        let (handle, timeout_rx, _) = setup_timeouts();

        let acks = 2;

        start_timeout_with(&handle, TIMEOUT_IDS[0].clone(), acks, false);

        for _ in 0..acks {
            ack_timeout(TIMEOUT_IDS[0].clone(), IDS[0], &handle);
        }

        let timeouts = timeout_rx.recv().unwrap();

        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].id.timeout_id, TIMEOUT_IDS[0].timeout_id);
    }

    #[test]
    fn test_cumulative_timeouts() {
        let (handle, timeout_rx, _) = setup_timeouts();

        start_timeout_with(&handle, TIMEOUT_IDS[0].clone(), 1, true);

        for t_count in 1..=2 {
            let timeouts = timeout_rx.recv().unwrap();

            assert_eq!(timeouts.len(), 1);
            assert_eq!(timeouts[0].id.timeout_id, TIMEOUT_IDS[0].timeout_id);
            assert_eq!(timeouts[0].timeout_count, t_count);
        }
    }

    #[test]
    fn test_cumulative_timeouts_ack() {
        let (handle, timeout_rx, _) = setup_timeouts();

        start_timeout_with(&handle, TIMEOUT_IDS[0].clone(), 1, true);

        // Timeout once.
        let timeouts = timeout_rx.recv().unwrap();

        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].id.timeout_id, TIMEOUT_IDS[0].timeout_id);
        assert_eq!(timeouts[0].timeout_count, 1);

        ack_timeout(TIMEOUT_IDS[0].clone(), OUR_ID, &handle);

        std::thread::sleep(Duration::from_secs(2));

        let timeouts = timeout_rx.try_recv();

        assert!(timeouts.is_err());
    }

    #[test]
    fn test_timeout_cancel() {
        let (handle, timeout_rx, _) = setup_timeouts();

        start_timeout(TIMEOUT_IDS[0].clone(), &handle);

        cancel_timeout(TIMEOUT_IDS[0].clone(), &handle);

        std::thread::sleep(Duration::from_secs(2));

        let timeouts = timeout_rx.try_recv();

        assert!(timeouts.is_err());
    }
}
