//! Local host process adapters (#2024 S3, #2615): the systemd *user* unit
//! installer for the admission broker, and the detached start behind the
//! broker started on demand. Effects only — the install/uninstall and
//! start-on-demand policy is the `application/admission` use cases'; here
//! we write the unit file, run `systemctl --user`, and start a process.

pub mod detached_process;
pub mod systemd_service_manager;

pub use systemd_service_manager::SystemdUserServiceManager;
