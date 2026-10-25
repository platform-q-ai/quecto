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
#[cfg(target_os = "linux")]
#[path = "backup_unix.rs"]
pub(super) mod anchored;
#[cfg(target_os = "linux")]
impl BackupFileOps for SystemBackupFileOps {
    fn create_partial(&self, directory: &File, name: &str) -> io::Result<File> {
        anchored::open(
            directory,
            name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
    }
    fn sync_file(&self, file: &File) -> io::Result<()> {
        file.sync_all()
    }
    fn publish_noreplace(
        &self,
        directory: &File,
        partial: &AdmittedBackupFile,
        final_name: &str,
    ) -> io::Result<()> {
        anchored::check(directory, partial)?;
        anchored::link(directory, &partial.name, final_name)?;
        // Both names now refer to the same inode. Remove only our admitted partial.
        let mut linked = partial.clone();
        linked.identity.links = 2;
        anchored::remove(directory, &linked)
    }
    fn sync_directory(&self, directory: &File) -> io::Result<()> {
        directory.sync_all()
    }
    fn prune_admitted(&self, directory: &File, candidate: &AdmittedBackupFile) -> io::Result<()> {
        anchored::remove(directory, candidate)
    }
}
#[cfg(not(target_os = "linux"))]
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
#[cfg(not(target_os = "linux"))]
fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "anchored backup primitives unavailable",
    ))
}

#[cfg(test)]
#[cfg(target_os = "linux")]
#[path = "backup_files_tests.rs"]
mod tests;
