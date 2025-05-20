use std::path::{Path, PathBuf};

use metrics::{gauge, Label};
use sysinfo::{Disk, Disks};

use crate::Report;

#[derive(Debug, thiserror::Error)]
#[error("Disk not found for path: {path}")]
pub struct DiskNotFoundError {
    path: PathBuf,
}

/// A reporter for system disk metrics, focusing on the disk hosting a specific path.
#[derive(Debug)]
pub struct DiskReporter {
    disk: Disk,
    labels: Vec<Label>,
}

impl DiskReporter {
    /// Creates a new [`StorageMetrics`] reporter for the disk containing the given path.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, DiskNotFoundError> {
        let path = path.as_ref();

        if let Some(disk) = Self::get_disk_for_path(path) {
            let mount_point_str = disk.mount_point().to_string_lossy().into_owned();

            // Use disk.name() for a more specific identifier if available (e.g. device name or
            // volume label) Fallback to mount_point if name is empty or too generic
            // like "HarddiskVolumeX" on Windows for root.
            let disk_id = disk.name().to_string_lossy();
            let disk_label = if disk_id.is_empty()
                || disk_id.starts_with("HarddiskVolume") && mount_point_str == "/"
            {
                mount_point_str.clone()
            } else {
                disk_id.into_owned()
            };

            let labels = vec![
                Label::new("mount_point", mount_point_str.clone()),
                Label::new("disk_label", disk_label.clone()),
            ];

            Ok(Self { disk, labels })
        } else {
            Err(DiskNotFoundError { path: path.to_owned() })
        }
    }

    /// Finds the disk that contains the given path by looking for the longest
    /// mount point that is a prefix of the path.
    fn get_disk_for_path(path: &Path) -> Option<Disk> {
        let disks: Vec<Disk> = Disks::new_with_refreshed_list().into();
        let mut best_match: Option<Disk> = None;
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        for disk in disks {
            let mount_point = disk.mount_point();
            if canonical_path.starts_with(mount_point)
                && best_match.as_ref().is_none_or(|current_best| {
                    mount_point.as_os_str().len() > current_best.mount_point().as_os_str().len()
                })
            {
                best_match = Some(disk);
            }
        }

        best_match
    }
}

impl Report for DiskReporter {
    fn report(&self) {
        let total_space = self.disk.total_space();
        let available_space = self.disk.available_space();
        let used_space = total_space - available_space;

        gauge!("system.storage_total_bytes", self.labels.clone()).set(total_space as f64);
        gauge!("system.storage_available_bytes", self.labels.clone()).set(available_space as f64);
        gauge!("system.storage_used_bytes", self.labels.clone()).set(used_space as f64);
    }
}

/// Describes the storage metrics.
pub fn describe_storage_metrics() {
    use metrics::describe_gauge;

    describe_gauge!(
        "system.storage.total_space_bytes",
        metrics::Unit::Bytes,
        "Total storage space on the disk."
    );
    describe_gauge!(
        "system.storage.available_space_bytes",
        metrics::Unit::Bytes,
        "Available storage space on the disk."
    );
}
