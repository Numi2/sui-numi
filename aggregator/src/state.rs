// Checkpoint streaming and state reconciliation
//
// Consumes gRPC SubscriptionService checkpoint stream and maintains a simple
// in-memory reconciliation cursor. Broadcasts new checkpoints to subscribers.
//
// Numan Thabit 2025 Nov

use crate::transport::grpc::{sui, GrpcClients};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use futures::StreamExt;
use tracing::{debug, info, warn};

#[derive(Clone)]
pub struct CheckpointUpdate {
	pub cursor: u64,
	pub checkpoint: Option<sui::rpc::v2::Checkpoint>,
}

#[derive(Clone)]
pub struct CheckpointState {
	last_cursor: Arc<RwLock<Option<u64>>>,
	tx: broadcast::Sender<CheckpointUpdate>,
}

impl CheckpointState {
	pub fn new(buffer: usize) -> Self {
		let (tx, _) = broadcast::channel(buffer);
		Self { last_cursor: Arc::new(RwLock::new(None)), tx }
	}

	pub fn subscribe(&self) -> broadcast::Receiver<CheckpointUpdate> {
		self.tx.subscribe()
	}

	pub async fn last_cursor(&self) -> Option<u64> {
		*self.last_cursor.read().await
	}
}

/// Start the checkpoint streaming task.
/// Spawns a background task that consumes the gRPC stream and updates state.
pub async fn start_checkpoint_streaming(
	mut grpc: GrpcClients,
	state: CheckpointState,
) -> Result<tokio::task::JoinHandle<()>> {
	let handle = tokio::spawn(async move {
		loop {
			match grpc.subscribe_checkpoints().await {
				Ok(mut stream) => {
					info!("checkpoint stream connected");
					while let Some(msg) = stream.next().await {
						match msg {
							Ok(resp) => {
								let cursor = resp.cursor.unwrap_or_default();
								{
									let mut guard = state.last_cursor.write().await;
									*guard = Some(cursor);
								}
								let update = CheckpointUpdate { cursor, checkpoint: resp.checkpoint };
								let _ = state.tx.send(update);
								debug!(cursor = cursor, "checkpoint advanced");
							}
							Err(err) => {
								warn!(error = %err, "checkpoint stream item error; reconnecting");
								break;
							}
						}
					}
					warn!("checkpoint stream ended; reconnecting shortly");
				}
				Err(err) => {
					warn!(error = %err, "failed to connect checkpoint stream; retrying");
				}
			}
			tokio::time::sleep(std::time::Duration::from_secs(2)).await;
		}
	});
	Ok(handle)
}


