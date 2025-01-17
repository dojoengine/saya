use anyhow::Result;
use celestia_rpc::{BlobClient, Client};
use celestia_types::{nmt::Namespace, AppVersion, Blob, TxConfig};
use log::{debug, info};
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

use crate::{
    data_availability::{
        DataAvailabilityBackend, DataAvailabilityBackendBuilder, DataAvailabilityContent,
        DataAvailabilityCursor, DataAvailabilityPacket, DataAvailabilityPointer,
    },
    prover::Proof,
    service::{Daemon, FinishHandle, ShutdownHandle},
};

// TODO: make namespace configurable?
const NAMESPACE: Namespace = Namespace::const_v0(*b"sayaproofs");

#[derive(Debug)]
pub struct CelestiaDataAvailabilityBackend {
    rpc_url: Url,
    auth_token: String,
    last_pointer: Option<DataAvailabilityPointer>,
    proof_channel: Receiver<Proof>,
    cursor_channel: Sender<DataAvailabilityCursor>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct CelestiaDataAvailabilityBackendBuilder {
    rpc_url: Url,
    auth_token: String,
    last_pointer: Option<Option<DataAvailabilityPointer>>,
    proof_channel: Option<Receiver<Proof>>,
    cursor_channel: Option<Sender<DataAvailabilityCursor>>,
}

impl CelestiaDataAvailabilityBackend {
    async fn run(mut self) {
        loop {
            let new_proof = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_proof = self.proof_channel.recv() => new_proof,
            };

            // This should be fine for now as provers wouldn't drop senders. This might change in
            // the future.
            let new_proof = new_proof.unwrap();
            debug!("Received new proof");

            // TODO: error handling
            let client = Client::new(self.rpc_url.as_ref(), Some(&self.auth_token))
                .await
                .unwrap();

            let packet = DataAvailabilityPacket {
                prev: self.last_pointer,
                content: DataAvailabilityContent {
                    from_block_number: new_proof.block_number,
                    to_block_number: new_proof.block_number,
                    proof: new_proof.proof,
                },
            };

            // TODO: error handling
            let mut serialized_packet: Vec<u8> = Vec::new();
            ciborium::into_writer(&packet, &mut serialized_packet).unwrap();
            debug!(
                "Celestia DA blob size for block #{}: {} bytes",
                new_proof.block_number,
                serialized_packet.len()
            );

            // TODO: error handling
            let blob = Blob::new(NAMESPACE, serialized_packet, AppVersion::V3).unwrap();
            let commitment = blob.commitment.0;

            // TODO: error handling
            let celestia_block = client
                .blob_submit(&[blob], TxConfig::default())
                .await
                .unwrap();
            self.last_pointer = Some(DataAvailabilityPointer {
                height: celestia_block,
                commitment,
            });

            info!(
                "Proof made availalbe on Celestia block #{}. Commitment: {}",
                celestia_block,
                hex::encode(commitment)
            );

            let new_cursor = DataAvailabilityCursor {
                from_block_number: new_proof.block_number,
                to_block_number: new_proof.block_number,
                pointer: DataAvailabilityPointer {
                    height: celestia_block,
                    commitment,
                },
            };

            // Since the channel is bounded, it's possible
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.cursor_channel.send(new_cursor) => {},
            }
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl CelestiaDataAvailabilityBackendBuilder {
    pub fn new(rpc_url: Url, auth_token: String) -> Self {
        Self {
            rpc_url,
            auth_token,
            last_pointer: None,
            proof_channel: None,
            cursor_channel: None,
        }
    }
}

impl DataAvailabilityBackendBuilder for CelestiaDataAvailabilityBackendBuilder {
    type Backend = CelestiaDataAvailabilityBackend;

    fn build(self) -> Result<Self::Backend> {
        Ok(CelestiaDataAvailabilityBackend {
            rpc_url: self.rpc_url,
            auth_token: self.auth_token,
            last_pointer: self
                .last_pointer
                .ok_or_else(|| anyhow::anyhow!("`last_pointer` not set"))?,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            cursor_channel: self
                .cursor_channel
                .ok_or_else(|| anyhow::anyhow!("`cursor_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn last_pointer(mut self, last_pointer: Option<DataAvailabilityPointer>) -> Self {
        self.last_pointer = Some(last_pointer);
        self
    }

    fn proof_channel(mut self, proof_channel: Receiver<Proof>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }

    fn cursor_channel(mut self, cursor_channel: Sender<DataAvailabilityCursor>) -> Self {
        self.cursor_channel = Some(cursor_channel);
        self
    }
}

impl DataAvailabilityBackend for CelestiaDataAvailabilityBackend {}

impl Daemon for CelestiaDataAvailabilityBackend {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
