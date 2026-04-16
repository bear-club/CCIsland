use std::collections::HashMap;

use tokio::sync::{oneshot, Mutex};

use crate::shared_types::{ApprovalDecision, ApprovalRequestData};

struct PendingApproval {
  request: ApprovalRequestData,
  sender: oneshot::Sender<ApprovalDecision>,
}

#[derive(Default)]
pub struct ApprovalManager {
  pending: Mutex<HashMap<String, PendingApproval>>,
}

impl ApprovalManager {
  pub async fn wait_for_decision(&self, request: ApprovalRequestData) -> oneshot::Receiver<ApprovalDecision> {
    let request_id = request.id.clone();
    let (sender, receiver) = oneshot::channel();
    let mut pending = self.pending.lock().await;
    pending.insert(
      request.id.clone(),
      PendingApproval {
        request,
        sender,
      },
    );
    eprintln!("[ApprovalManager] wait_for_decision inserted id={} pending_count={}", request_id, pending.len());
    receiver
  }

  pub async fn resolve(&self, id: &str, decision: ApprovalDecision) -> bool {
    let mut pending = self.pending.lock().await;
    let pending_item = pending.remove(id);
    eprintln!("[ApprovalManager] resolve id={} found={} pending_count_after={}", id, pending_item.is_some(), pending.len());
    if let Some(pending) = pending_item {
      let _ = pending.sender.send(decision);
      true
    } else {
      false
    }
  }

  pub async fn has_pending(&self) -> bool {
    !self.pending.lock().await.is_empty()
  }

  pub async fn pending_requests(&self) -> Vec<ApprovalRequestData> {
    self.pending
      .lock().await
      .values()
      .map(|pending| pending.request.clone())
      .collect()
  }
}
