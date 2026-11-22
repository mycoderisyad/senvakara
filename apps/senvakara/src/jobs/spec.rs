//! What a job does: the `JobSpec` that starts (or restarts) a transfer or install.

use crate::files::Source;

/// What a job does, with everything needed to start it (again).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobSpec {
    InstallApk {
        route_id: String,
        serial: String,
        local_path: String,
    },
    /// Copy a file from this computer to a phone. Started by the Files pane, like the rest.
    DevicePush {
        route_id: String,
        serial: String,
        local_path: String,
        remote_dir: String,
        overwrite: bool,
    },
    /// Copy a file from a phone to this computer.
    DevicePull {
        route_id: String,
        remote_path: String,
        local_dir: String,
        overwrite: bool,
    },
    /// Copy a file from this computer to a host over SFTP.
    SftpUpload {
        host_id: String,
        local_path: String,
        remote_dir: String,
        overwrite: bool,
    },
    /// Copy a file from a host to this computer over SFTP.
    SftpDownload {
        host_id: String,
        remote_path: String,
        local_dir: String,
        overwrite: bool,
    },
    /// Copy a file from a host straight to a phone, without a copy on this computer.
    HostToDevice {
        host_id: String,
        remote_path: String,
        route_id: String,
        serial: String,
        device_dir: String,
        overwrite: bool,
    },
    /// Copy a file from a phone straight to a host.
    DeviceToHost {
        route_id: String,
        device_path: String,
        host_id: String,
        remote_dir: String,
        overwrite: bool,
    },
    /// Copy a file or a folder from one host straight to another (or to another folder of
    /// the same host), without a copy on this computer.
    HostToHost {
        from_host: String,
        remote_path: String,
        to_host: String,
        remote_dir: String,
        overwrite: bool,
    },
}

impl JobSpec {
    /// The same job, allowed to replace an existing file. None when the job never overwrites
    /// or already does.
    pub fn replacing(&self) -> Option<JobSpec> {
        let mut spec = self.clone();
        match &mut spec {
            JobSpec::DevicePush { overwrite, .. }
            | JobSpec::DevicePull { overwrite, .. }
            | JobSpec::SftpUpload { overwrite, .. }
            | JobSpec::SftpDownload { overwrite, .. }
            | JobSpec::HostToDevice { overwrite, .. }
            | JobSpec::DeviceToHost { overwrite, .. }
            | JobSpec::HostToHost { overwrite, .. }
                if !*overwrite =>
            {
                *overwrite = true;
                Some(spec)
            }
            _ => None,
        }
    }

    /// (file name, where it already exists) for the "Replace existing file?" question.
    pub(super) fn target(&self) -> (String, String) {
        match self {
            JobSpec::InstallApk { local_path, .. } => (file_name(local_path), String::new()),
            JobSpec::DevicePush {
                local_path,
                remote_dir,
                ..
            } => (file_name(local_path), remote_dir.clone()),
            JobSpec::DevicePull {
                remote_path,
                local_dir,
                ..
            } => (file_name(remote_path), local_dir.clone()),
            JobSpec::SftpUpload {
                local_path,
                remote_dir,
                ..
            } => (file_name(local_path), remote_dir.clone()),
            JobSpec::SftpDownload {
                remote_path,
                local_dir,
                ..
            } => (file_name(remote_path), local_dir.clone()),
            JobSpec::HostToDevice {
                remote_path,
                device_dir,
                ..
            } => (file_name(remote_path), device_dir.clone()),
            JobSpec::DeviceToHost {
                device_path,
                remote_dir,
                ..
            } => (file_name(device_path), remote_dir.clone()),
            JobSpec::HostToHost {
                remote_path,
                remote_dir,
                ..
            } => (file_name(remote_path), remote_dir.clone()),
        }
    }

    /// The place and folder a copy writes to. None for a job that is not a copy.
    pub fn destination(&self) -> Option<(Source, String)> {
        match self {
            JobSpec::InstallApk { .. } => None,
            JobSpec::DevicePush {
                route_id,
                remote_dir,
                ..
            } => Some((Source::Device(route_id.clone()), remote_dir.clone())),
            JobSpec::HostToDevice {
                route_id,
                device_dir,
                ..
            } => Some((Source::Device(route_id.clone()), device_dir.clone())),
            JobSpec::DevicePull { local_dir, .. } | JobSpec::SftpDownload { local_dir, .. } => {
                Some((Source::Local, local_dir.clone()))
            }
            JobSpec::SftpUpload {
                host_id,
                remote_dir,
                ..
            }
            | JobSpec::DeviceToHost {
                host_id,
                remote_dir,
                ..
            } => Some((Source::Host(host_id.clone()), remote_dir.clone())),
            JobSpec::HostToHost {
                to_host,
                remote_dir,
                ..
            } => Some((Source::Host(to_host.clone()), remote_dir.clone())),
        }
    }

    /// What a "conflict" from the backend means for this kind of job.
    pub(super) fn conflict_wording(&self) -> &'static str {
        match self {
            JobSpec::InstallApk { .. } => "The phone did not accept the app.",
            _ => "A file or folder with that name already exists.",
        }
    }
}

/// The last component of a Windows or device path.
pub fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_file_copies_can_be_retried_with_replace() {
        let install = JobSpec::InstallApk {
            route_id: "r".into(),
            serial: "s".into(),
            local_path: "C:\\apps\\demo.apk".into(),
        };
        assert_eq!(install.replacing(), None);

        let pull = JobSpec::DevicePull {
            route_id: "r".into(),
            remote_path: "/sdcard/Download/a.txt".into(),
            local_dir: "C:\\Users\\me".into(),
            overwrite: false,
        };
        let again = pull.replacing().expect("a pull can replace");
        assert!(matches!(
            again,
            JobSpec::DevicePull {
                overwrite: true,
                ..
            }
        ));
        // Already replacing: a second clash is a real failure.
        assert_eq!(again.replacing(), None);
        assert_eq!(
            pull.target(),
            ("a.txt".to_string(), "C:\\Users\\me".to_string())
        );
    }

    #[test]
    fn copies_know_where_they_write() {
        let upload = JobSpec::SftpUpload {
            host_id: "h".into(),
            local_path: "C:\\Temp\\a.txt".into(),
            remote_dir: "/tmp".into(),
            overwrite: false,
        };
        assert_eq!(
            upload.destination(),
            Some((Source::Host("h".into()), "/tmp".to_string()))
        );
        assert_eq!(upload.target(), ("a.txt".to_string(), "/tmp".to_string()));
        assert!(upload.replacing().is_some());
        let across = JobSpec::HostToDevice {
            host_id: "h".into(),
            remote_path: "/tmp/a.txt".into(),
            route_id: "r".into(),
            serial: "s".into(),
            device_dir: "/sdcard/Download".into(),
            overwrite: true,
        };
        assert_eq!(
            across.destination(),
            Some((Source::Device("r".into()), "/sdcard/Download".to_string()))
        );
        assert_eq!(across.replacing(), None);
        let install = JobSpec::InstallApk {
            route_id: "r".into(),
            serial: "s".into(),
            local_path: "C:\\apps\\demo.apk".into(),
        };
        assert_eq!(install.destination(), None);
    }

    #[test]
    fn a_copy_between_hosts_writes_to_the_second_host_and_can_replace() {
        let across = JobSpec::HostToHost {
            from_host: "a".into(),
            remote_path: "/srv/site".into(),
            to_host: "b".into(),
            remote_dir: "/tmp".into(),
            overwrite: false,
        };
        assert_eq!(
            across.destination(),
            Some((Source::Host("b".into()), "/tmp".to_string()))
        );
        assert_eq!(across.target(), ("site".to_string(), "/tmp".to_string()));
        assert!(matches!(
            across.replacing(),
            Some(JobSpec::HostToHost {
                overwrite: true,
                ..
            })
        ));
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name("C:\\apps\\demo.apk"), "demo.apk");
        assert_eq!(file_name("/sdcard/Download/a.txt"), "a.txt");
        assert_eq!(file_name("/sdcard/Download/"), "Download");
        assert_eq!(file_name("plain"), "plain");
    }
}
