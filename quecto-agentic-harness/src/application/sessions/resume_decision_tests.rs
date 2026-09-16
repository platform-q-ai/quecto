use super::{execute_resume_decision, transcript_only_fork, ResumeDecisionEffects, SameScopeRestoreContext, ResumeDecisionError, ResumeDecisionPlanner, ResumeOutcome, ResumePlan};
use std::sync::Mutex;
use crate::domain::session_scope::{AssociationProvenance, CanonicalExecutionLocation, ResumeDisposition, SessionHomeScope, SessionScopeMetadata};
use crate::domain::session::{Session, PersistedSubagentRosterEntry, SubagentLiveness, SubagentRestoreReason};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::message::Message;
use crate::domain::workflow::WorkflowRunPersisted;

fn scope(path: &str) -> SessionHomeScope {
    SessionHomeScope::scoped(CanonicalExecutionLocation::new(path).unwrap(), None, AssociationProvenance::Discovered)
}

#[test]
fn same_scope_is_the_only_implicit_resume() {
    assert_eq!(ResumeDecisionPlanner::plan(&scope("/a"), &scope("/a"), true, ResumeDisposition::SameScope).unwrap(), ResumePlan::ResumeHere);
    assert_eq!(ResumeDecisionPlanner::plan(&scope("/a"), &scope("/b"), true, ResumeDisposition::SameScope), Err(ResumeDecisionError::ExplicitChoiceRequired));
}

#[test]
fn cross_scope_actions_are_explicit_and_folder_availability_is_allowlisted() {
    assert_eq!(ResumeDecisionPlanner::plan(&scope("/a"), &scope("/b"), true, ResumeDisposition::OpenOriginal).unwrap(), ResumePlan::LaunchOriginal);
    assert_eq!(ResumeDecisionPlanner::plan(&scope("/a"), &scope("/b"), false, ResumeDisposition::OpenOriginal), Err(ResumeDecisionError::OriginalUnavailable));
    assert_eq!(ResumeDecisionPlanner::plan(&scope("/a"), &scope("/b"), false, ResumeDisposition::Locate).unwrap(), ResumePlan::LocateAndReassociate);
    assert_eq!(ResumeDecisionPlanner::plan(&scope("/a"), &scope("/b"), false, ResumeDisposition::ForkCurrent).unwrap(), ResumePlan::ForkTranscriptOnly);
}

#[test]
fn legacy_records_require_explicit_association_or_fork() {
    assert_eq!(ResumeDecisionPlanner::plan(&SessionHomeScope::LegacyUnscoped, &scope("/b"), true, ResumeDisposition::SameScope), Err(ResumeDecisionError::ExplicitChoiceRequired));
    assert_eq!(ResumeDecisionPlanner::plan(&SessionHomeScope::LegacyUnscoped, &scope("/b"), true, ResumeDisposition::Locate).unwrap(), ResumePlan::LocateAndReassociate);
}

#[test]
fn cancel_has_no_effect_in_every_state() {
    assert_eq!(ResumeDecisionPlanner::plan(&scope("/a"), &scope("/a"), true, ResumeDisposition::Cancel).unwrap(), ResumePlan::Cancel);
}

#[test]
fn fork_imports_transcript_only_under_a_fresh_identity() {
    let source = Session {
        key: SessionIdentity::from_persisted_key("cli:source"),
        messages: vec![Message::user("safe transcript")],
        workflow_run: Some(WorkflowRunPersisted { template_id: Some("unsafe".into()), done: vec![true], active_issue: None }),
        subagent_roster: vec![PersistedSubagentRosterEntry {
            agent_uuid: "a".into(), display_name: "child".into(), session_key: "child".into(),
            liveness: SubagentLiveness::Live, restore_reason: SubagentRestoreReason::LegacyUnspecified,
            parent_id: None, read_only: false, delivered_message_ordinal: None,
            pending_message_reports: Default::default(), status: None,
        }],
    };
    let forked = transcript_only_fork(&source, SessionIdentity::from_persisted_key("cli:fresh"));
    assert_eq!(forked.key.runtime_key(), "cli:fresh");
    assert_eq!(forked.messages.len(), 1);
    assert_eq!(forked.messages[0].content, "safe transcript");
    assert!(forked.workflow_run.is_none());
    assert!(forked.subagent_roster.is_empty());
}

#[derive(Default)]
struct Effects { log: Mutex<Vec<String>>, fail_commit: bool, fail_launch: bool }
impl SameScopeRestoreContext for Effects { fn restore<'a>(&'a mut self,key:&'a SessionIdentity)->super::ResumeEffect<'a,()>{Box::pin(async move {self.log.lock().unwrap().push(format!("restore:{}",key.runtime_key()));Ok(())})} }
impl ResumeDecisionEffects for Effects {
    fn claim(&self,key:&SessionIdentity)->Result<(),String>{self.log.lock().unwrap().push(format!("claim:{}",key.runtime_key()));Ok(())}
    fn release(&self,key:&SessionIdentity){self.log.lock().unwrap().push(format!("release:{}",key.runtime_key()));}
    fn load<'a>(&'a self,key:&'a SessionIdentity)->super::ResumeEffect<'a,Session>{Box::pin(async move {Ok(Session::new(key.clone()))})}
    fn commit<'a>(&'a self,session:&'a Session,scope:&'a SessionScopeMetadata)->super::ResumeEffect<'a,()>{Box::pin(async move { self.log.lock().unwrap().push(format!("commit:{}:{}",session.key.runtime_key(),scope.home().execution_location().unwrap().as_str())); if self.fail_commit{Err("commit failed".into())}else{Ok(())} })}
    fn rollback(&self,key:&SessionIdentity){self.log.lock().unwrap().push(format!("rollback:{}",key.runtime_key()));}
    fn launch_fresh<'a>(&'a self,location:&'a str,_:&'a SessionIdentity)->super::ResumeEffect<'a,()>{Box::pin(async move {self.log.lock().unwrap().push(format!("launch:{location}"));if self.fail_launch{Err("launch failed".into())}else{Ok(())}})}
    fn publish_fork(&self,session:Session){self.log.lock().unwrap().push(format!("publish-fork:{}",session.key.runtime_key()));}
    fn publish_reassociation(&self,key:&SessionIdentity,_:&SessionScopeMetadata){self.log.lock().unwrap().push(format!("publish-locate:{}",key.runtime_key()));}
}
fn metadata(path:&str)->SessionScopeMetadata { SessionScopeMetadata::current(scope(path)) }
fn source()->Session { Session::new(SessionIdentity::from_persisted_key("cli:source")) }

#[tokio::test]
async fn fork_commits_transcript_and_scope_atomically_and_rolls_back_claim_on_failure(){
    let effects=Effects{fail_commit:true,..Default::default()};
    let result=execute_resume_decision(&effects,ResumePlan::ForkTranscriptOnly,&source(),&metadata("/current"),Some(SessionIdentity::from_persisted_key("cli:fresh")),&mut Effects::default()).await;
    assert_eq!(result,Err("commit failed".into()));
    assert_eq!(effects.log.lock().unwrap().as_slice(),["claim:cli:fresh","commit:cli:fresh:/current","rollback:cli:fresh","release:cli:fresh"]);
}

#[tokio::test]
async fn open_original_uses_fresh_launch_and_failure_does_not_publish_success(){
    let effects=Effects{fail_launch:true,..Default::default()};
    let result=execute_resume_decision(&effects,ResumePlan::LaunchOriginal,&source(),&metadata("/original"),None,&mut Effects::default()).await;
    assert_eq!(result,Err("launch failed".into()));
    assert_eq!(effects.log.lock().unwrap().as_slice(),["launch:/original"]);
}

#[tokio::test]
async fn same_scope_delegates_to_durable_restore_transaction(){
    let effects=Effects::default();
    let mut restore=Effects::default();
    let result=execute_resume_decision(&effects,ResumePlan::ResumeHere,&source(),&metadata("/current"),None,&mut restore).await.unwrap();
    assert_eq!(result,ResumeOutcome::Resume(SessionIdentity::from_persisted_key("cli:source")));
    assert_eq!(effects.log.lock().unwrap().as_slice(),[] as [&str;0]);
    assert_eq!(restore.log.lock().unwrap().as_slice(),["restore:cli:source"]);
}

#[tokio::test]
async fn successful_fork_publishes_only_after_durable_scope_commit(){
    let effects=Effects::default();
    execute_resume_decision(&effects,ResumePlan::ForkTranscriptOnly,&source(),&metadata("/current"),Some(SessionIdentity::from_persisted_key("cli:fresh")),&mut Effects::default()).await.unwrap();
    assert_eq!(effects.log.lock().unwrap().as_slice(),["claim:cli:fresh","commit:cli:fresh:/current","publish-fork:cli:fresh"]);
}

#[tokio::test]
async fn locate_commits_supplied_scope_before_releasing_ownership(){
    let effects=Effects::default();
    let result=execute_resume_decision(&effects,ResumePlan::LocateAndReassociate,&source(),&metadata("/located"),None,&mut Effects::default()).await.unwrap();
    assert_eq!(result,ResumeOutcome::Reassociated);
    assert_eq!(effects.log.lock().unwrap().as_slice(),["claim:cli:source","commit:cli:source:/located","publish-locate:cli:source","release:cli:source"]);
}
