//! Application-owned policy and effect transaction for folder-aware resume.

use std::future::Future;
use std::pin::Pin;

use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;
use crate::domain::session_scope::{ResumeDisposition, SessionHomeScope, SessionScopeMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumePlan { ResumeHere, LaunchOriginal, ForkTranscriptOnly, LocateAndReassociate, Cancel }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecisionError { ExplicitChoiceRequired, OriginalUnavailable, ActionNotEligible }

pub struct ResumeDecisionPlanner;
impl ResumeDecisionPlanner {
    pub fn plan(home:&SessionHomeScope,current:&SessionHomeScope,original_available:bool,action:ResumeDisposition)->Result<ResumePlan,ResumeDecisionError>{
        match action {
            ResumeDisposition::Cancel=>Ok(ResumePlan::Cancel),
            ResumeDisposition::ForkCurrent=>Ok(ResumePlan::ForkTranscriptOnly),
            ResumeDisposition::Locate=>Ok(ResumePlan::LocateAndReassociate),
            ResumeDisposition::SameScope if same_scope(home,current)=>Ok(ResumePlan::ResumeHere),
            ResumeDisposition::SameScope=>Err(ResumeDecisionError::ExplicitChoiceRequired),
            ResumeDisposition::OpenOriginal if original_available && scoped(home)=>Ok(ResumePlan::LaunchOriginal),
            ResumeDisposition::OpenOriginal if scoped(home)=>Err(ResumeDecisionError::OriginalUnavailable),
            ResumeDisposition::OpenOriginal=>Err(ResumeDecisionError::ActionNotEligible),
        }
    }
}
fn scoped(value:&SessionHomeScope)->bool { matches!(value,SessionHomeScope::Scoped{..}) }
fn same_scope(a:&SessionHomeScope,b:&SessionHomeScope)->bool { matches!((a.execution_location(),b.execution_location()),(Some(x),Some(y)) if x==y) }

/// Only safe fork payload: no workflow, operational children, or persisted handles.
pub fn transcript_only_fork(source:&Session,fresh:SessionIdentity)->Session {
    assert_ne!(source.key,fresh,"a fork requires a fresh identity");
    Session{key:fresh,messages:source.messages.clone(),workflow_run:None,subagent_roster:Vec::new()}
}

pub type ResumeEffect<'a,T> = Pin<Box<dyn Future<Output=Result<T,String>>+Send+'a>>;
/// Request-owned mutable runtime boundary needed only for same-scope restoration.
pub trait SameScopeRestoreContext {
    fn restore<'a>(&'a mut self,key:&'a SessionIdentity)->ResumeEffect<'a,()>;
}
/// Atomic adapter boundary. Implementations stage transcript+scope together and expose
/// success only after commit. Rollback removes the claim and every staged projection.
pub trait ResumeDecisionEffects: Send+Sync {
    fn claim(&self,key:&SessionIdentity)->Result<(),String>;
    fn release(&self,key:&SessionIdentity);
    fn load<'a>(&'a self,key:&'a SessionIdentity)->ResumeEffect<'a,Session>;
    fn commit<'a>(&'a self,session:&'a Session,scope:&'a SessionScopeMetadata)->ResumeEffect<'a,()>;
    fn rollback(&self,key:&SessionIdentity);
    /// Spawn a new process rooted at `location`; its composition reloads config/tools.
    fn launch_fresh<'a>(&'a self,location:&'a str,key:&'a SessionIdentity)->ResumeEffect<'a,()>;
    /// Publish a committed fork as the active session. Must be infallible: all fallible
    /// work belongs to claim/commit before this commit point.
    fn publish_fork(&self,session:Session);
    /// Publish catalogue reassociation only after transcript+metadata commit.
    fn publish_reassociation(&self,key:&SessionIdentity,scope:&SessionScopeMetadata);
}
#[derive(Debug,Clone,PartialEq,Eq)] pub enum ResumeOutcome { Resume(SessionIdentity), Launched, Forked(SessionIdentity), Reassociated, Cancelled }

/// Executes without mutating the current runtime. Same-scope returns the identity for
/// the established resume transaction; open launches a fresh target runtime; fork and
/// locate publish only after durable transcript+scope commit.
pub async fn execute_resume_decision(
    effects:&dyn ResumeDecisionEffects, plan:ResumePlan, source:&Session,
    target_scope:&SessionScopeMetadata, fresh:Option<SessionIdentity>,
    same_scope:&mut dyn SameScopeRestoreContext,
)->Result<ResumeOutcome,String>{
    match plan {
        ResumePlan::Cancel=>Ok(ResumeOutcome::Cancelled),
        ResumePlan::ResumeHere=>{
            same_scope.restore(&source.key).await?;
            Ok(ResumeOutcome::Resume(source.key.clone()))
        },
        ResumePlan::LaunchOriginal=>{
            let location=target_scope.home().execution_location().ok_or("original scope unavailable")?;
            effects.launch_fresh(location.as_str(),&source.key).await?;
            Ok(ResumeOutcome::Launched)
        }
        ResumePlan::ForkTranscriptOnly=>{
            let key=fresh.ok_or("fresh fork identity required")?;
            effects.claim(&key)?;
            let fork=transcript_only_fork(source,key.clone());
            if let Err(error)=effects.commit(&fork,target_scope).await { effects.rollback(&key); effects.release(&key); return Err(error); }
            effects.publish_fork(fork);
            Ok(ResumeOutcome::Forked(key))
        }
        ResumePlan::LocateAndReassociate=>{
            effects.claim(&source.key)?;
            let loaded=match effects.load(&source.key).await { Ok(v)=>v,Err(e)=>{effects.release(&source.key);return Err(e)} };
            if let Err(error)=effects.commit(&loaded,target_scope).await { effects.rollback(&source.key); effects.release(&source.key); return Err(error); }
            effects.publish_reassociation(&source.key,target_scope);
            effects.release(&source.key);
            Ok(ResumeOutcome::Reassociated)
        }
    }
}

#[cfg(test)] #[path="resume_decision_tests.rs"] mod tests;
