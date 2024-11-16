use std::{
    collections::HashMap,
    sync::{
        atomic::{self, AtomicBool},
        Arc,
    },
};

use tokio::{sync::mpsc, task::JoinHandle};

use zcash_client_backend::data_api::scanning::ScanRange;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::{consensus, zip32::AccountId};

use crate::{
    client::FetchRequest,
    primitives::WalletBlock,
    sync,
    traits::{SyncBlocks, SyncWallet},
};

use super::{error::ScanError, scan, ScanResults};

const SCAN_WORKER_POOLSIZE: usize = 2;

pub(crate) struct Scanner<P> {
    workers: Vec<ScanWorker<P>>,
    scan_results_sender: mpsc::UnboundedSender<(ScanRange, Result<ScanResults, ScanError>)>,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    consensus_parameters: P,
    ufvks: HashMap<AccountId, UnifiedFullViewingKey>,
}

// TODO: add fn for checking and handling worker errors
impl<P> Scanner<P>
where
    P: consensus::Parameters + Sync + Send + 'static,
{
    pub(crate) fn new(
        scan_results_sender: mpsc::UnboundedSender<(ScanRange, Result<ScanResults, ScanError>)>,
        fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
        consensus_parameters: P,
        ufvks: HashMap<AccountId, UnifiedFullViewingKey>,
    ) -> Self {
        let workers: Vec<ScanWorker<P>> = Vec::with_capacity(SCAN_WORKER_POOLSIZE);

        Self {
            workers,
            scan_results_sender,
            fetch_request_sender,
            consensus_parameters,
            ufvks,
        }
    }

    pub(crate) fn spawn_worker(&mut self) {
        let mut worker = ScanWorker::new(
            None,
            self.scan_results_sender.clone(),
            self.fetch_request_sender.clone(),
            self.consensus_parameters.clone(),
            self.ufvks.clone(),
        );
        worker.run().unwrap();
        self.workers.push(worker);
    }

    pub(crate) fn spawn_workers(&mut self) {
        for _ in 0..SCAN_WORKER_POOLSIZE {
            self.spawn_worker();
        }
    }

    pub(crate) fn idle_worker(&mut self) -> Option<&mut ScanWorker<P>> {
        if let Some(idle_worker) = self.workers.iter_mut().find(|worker| !worker.is_scanning()) {
            Some(idle_worker)
        } else {
            None
        }
    }

    /// Updates the scanner.
    ///
    /// If there is an idle worker, create a new scan task and send to worker.
    /// If there are no more range available to scan, shutdown the idle worker.
    pub(crate) fn update<W>(&mut self, wallet: &mut W)
    where
        W: SyncWallet + SyncBlocks,
    {
        if let Some(worker) = self.idle_worker() {
            if let Some(scan_task) = ScanTask::create(wallet).unwrap() {
                worker.add_scan_task(scan_task).unwrap();
            } else {
                if let Some(sender) = worker.scan_task_sender.take() {
                    drop(sender);
                }
            }
        }
    }
}

struct ScanWorker<P> {
    handle: Option<JoinHandle<Result<(), ()>>>,
    is_scanning: Arc<AtomicBool>,
    scan_task_sender: Option<mpsc::UnboundedSender<ScanTask>>,
    scan_results_sender: mpsc::UnboundedSender<(ScanRange, Result<ScanResults, ScanError>)>,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    consensus_parameters: P,
    ufvks: HashMap<AccountId, UnifiedFullViewingKey>,
}

impl<P> ScanWorker<P>
where
    P: consensus::Parameters + Sync + Send + 'static,
{
    fn new(
        scan_task_sender: Option<mpsc::UnboundedSender<ScanTask>>,
        scan_results_sender: mpsc::UnboundedSender<(ScanRange, Result<ScanResults, ScanError>)>,
        fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
        consensus_parameters: P,
        ufvks: HashMap<AccountId, UnifiedFullViewingKey>,
    ) -> Self {
        Self {
            handle: None,
            is_scanning: Arc::new(AtomicBool::new(false)),
            scan_task_sender,
            scan_results_sender,
            fetch_request_sender,
            consensus_parameters,
            ufvks,
        }
    }

    fn run(&mut self) -> Result<(), ()> {
        let (scan_task_sender, mut scan_task_receiver) = mpsc::unbounded_channel::<ScanTask>();
        let is_scanning = self.is_scanning.clone();
        let scan_results_sender = self.scan_results_sender.clone();
        let fetch_request_sender = self.fetch_request_sender.clone();
        let consensus_parameters = self.consensus_parameters.clone();
        let ufvks = self.ufvks.clone();
        let handle = tokio::spawn(async move {
            while let Some(scan_task) = scan_task_receiver.recv().await {
                is_scanning.store(true, atomic::Ordering::Release);

                let scan_results = scan(
                    fetch_request_sender.clone(),
                    &consensus_parameters,
                    &ufvks,
                    scan_task.scan_range.clone(),
                    scan_task.previous_wallet_block,
                )
                .await;

                scan_results_sender
                    .send((scan_task.scan_range, scan_results))
                    .unwrap();

                is_scanning.store(false, atomic::Ordering::Release);
            }

            Ok(())
        });

        self.handle = Some(handle);
        self.scan_task_sender = Some(scan_task_sender);

        Ok(())
    }

    fn is_scanning(&self) -> bool {
        self.is_scanning.load(atomic::Ordering::Acquire)
    }

    fn add_scan_task(&self, scan_task: ScanTask) -> Result<(), ()> {
        self.scan_task_sender
            .clone()
            .unwrap()
            .send(scan_task)
            .unwrap();

        Ok(())
    }
}

struct ScanTask {
    scan_range: ScanRange,
    previous_wallet_block: Option<WalletBlock>,
}

impl ScanTask {
    fn from_parts(scan_range: ScanRange, previous_wallet_block: Option<WalletBlock>) -> Self {
        Self {
            scan_range,
            previous_wallet_block,
        }
    }

    fn create<W>(wallet: &mut W) -> Result<Option<ScanTask>, ()>
    where
        W: SyncWallet + SyncBlocks,
    {
        if let Some(scan_range) = sync::select_scan_range(wallet.get_sync_state_mut().unwrap()) {
            let previous_wallet_block = wallet
                .get_wallet_block(scan_range.block_range().start - 1)
                .ok();

            Ok(Some(ScanTask::from_parts(
                scan_range,
                previous_wallet_block,
            )))
        } else {
            Ok(None)
        }
    }
}
