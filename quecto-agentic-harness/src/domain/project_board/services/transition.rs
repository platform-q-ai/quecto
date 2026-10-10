use super::super::entities::task::Task;
use super::super::value_objects::status::{Claim, Identity, TaskStatus, CLAIM_SECONDS};

pub fn claim(task: &Task, who: &Identity, now: u64) -> Result<Task, String> {
    let free = match &task.claim { None => true, Some(c) => c.expires <= now || &c.holder == who };
    let claimable = matches!(task.status, TaskStatus::Ready | TaskStatus::Claimed | TaskStatus::InProgress);
    if !(free && claimable) { return Err(format!("{} not claimable", task.id)); }
    let mut next = task.clone();
    next.claim = Some(Claim { holder: who.clone(), since: now, expires: now + CLAIM_SECONDS });
    if task.status == TaskStatus::Ready { next.status = TaskStatus::Claimed; }
    next.updated = now;
    Ok(next)
}
