//! Local host process adapters (#2024 S3): the systemd *user* unit installer
//! for the admission broker. Effects only — the install/uninstall policy is
//! the `application/admission` use cases; here we write the unit file and run
//! `systemctl --user`.

pub mod systemd_service_manager;

pub use systemd_service_manager::SystemdUserServiceManager;
