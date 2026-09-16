//! Atomic durability boundary for folder-aware resume decisions.
use std::{future::Future,pin::Pin};
use crate::domain::{session::Session,session_identity::SessionIdentity,session_scope::SessionScopeMetadata};

pub type ResumeTransactionResult<'a>=Pin<Box<dyn Future<Output=Result<(),String>>+Send+'a>>;

/// Infrastructure stages authoritative transcript and derived scope projection and
/// makes both visible at one commit point. Failure/cancellation must leave neither.
pub trait AtomicResumePersistence: Send+Sync {
    fn claim(&self,key:&SessionIdentity)->Result<(),String>;
    fn release(&self,key:&SessionIdentity);
    fn commit<'a>(&'a self,session:&'a Session,scope:&'a SessionScopeMetadata)->ResumeTransactionResult<'a>;
    fn rollback(&self,key:&SessionIdentity);
}

/// Affirmative launch capability. `Ready` means canonical target exists and a fresh
/// process can reload configuration/tools there.
#[derive(Debug,Clone,PartialEq,Eq)]
pub enum LaunchCapability { Ready{canonical_directory:String}, Unavailable{reason:String} }
pub trait FreshRuntimeLaunch: Send+Sync {
    fn capability(&self,target:&str)->LaunchCapability;
    fn launch<'a>(&'a self,canonical_directory:&'a str,key:&'a SessionIdentity)->ResumeTransactionResult<'a>;
}
