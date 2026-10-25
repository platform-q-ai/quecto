//! Directory-anchored no-follow publication for cooperating private-directory producers.
use std::{fs::File, io};
#[derive(Debug, Clone)]
pub struct BackupFileIdentity {
    pub device: u64,
    pub inode: u64,
    pub links: u64,
}
#[derive(Debug, Clone)]
pub struct AdmittedBackupFile {
    pub name: String,
    pub identity: BackupFileIdentity,
}
pub trait BackupFileOps: Send + Sync {
    fn create_partial(&self, directory: &File, name: &str) -> io::Result<File>;
    fn sync_file(&self, file: &File) -> io::Result<()>;
    fn publish_noreplace(
        &self,
        directory: &File,
        partial: &AdmittedBackupFile,
        final_name: &str,
    ) -> io::Result<()>;
    fn sync_directory(&self, directory: &File) -> io::Result<()>;
    fn prune_admitted(&self, directory: &File, candidate: &AdmittedBackupFile) -> io::Result<()>;
}
pub struct SystemBackupFileOps;
impl BackupFileOps for SystemBackupFileOps {
    fn create_partial(&self, _: &File, _: &str) -> io::Result<File> {
        unsupported()
    }
    fn sync_file(&self, _: &File) -> io::Result<()> {
        unsupported()
    }
    fn publish_noreplace(&self, _: &File, _: &AdmittedBackupFile, _: &str) -> io::Result<()> {
        unsupported()
    }
    fn sync_directory(&self, _: &File) -> io::Result<()> {
        unsupported()
    }
    fn prune_admitted(&self, _: &File, _: &AdmittedBackupFile) -> io::Result<()> {
        unsupported()
    }
}
fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "B13-lite not implemented",
    ))
}
