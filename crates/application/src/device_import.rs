//! Copies new recordings from an attached device into the workspace: each
//! copy is verified before it becomes a document, and the ledger records it
//! at once so an interrupted import resumes instead of repeating.

use std::fs;
use std::path::{Path, PathBuf};

use devices::ingest::{self, SourceFile};
use devices::{ImportSummary, ImportedFile, PairedDevice, RegistryStore};
use domain::DocumentState;
use runtime::CancellationToken;

use crate::DocumentService;

/// Hidden staging beside the packages: the scanner skips dot directories and
/// a rename from here into a package stays on one filesystem.
pub const STAGING_DIRECTORY: &str = ".meetnote-staging";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceImportProgress {
    /// `total` recordings are new.
    Planned { total: usize },
    Copying {
        index: usize,
        total: usize,
        name: String,
        copied: u64,
        size: u64,
    },
}

#[derive(Debug, Default)]
pub struct DeviceImportReport {
    pub imported: Vec<DocumentState>,
    /// Recording name and reason.
    pub failed: Vec<(String, String)>,
    pub deleted: usize,
    pub cancelled: bool,
}

pub struct DeviceImport<'a> {
    pub documents: &'a DocumentService,
    pub workspace_root: &'a Path,
    pub registry: &'a RegistryStore,
    pub device: &'a PairedDevice,
    pub volumes: &'a [PathBuf],
}

impl DeviceImport<'_> {
    pub fn run(
        &self,
        progress: &mut dyn FnMut(DeviceImportProgress),
        cancellation: &CancellationToken,
    ) -> Result<DeviceImportReport, String> {
        self.clear_partial_copies();
        // The paired model's layout; a model since dropped from the catalog
        // imports every recording.
        let layout = self
            .device
            .identity
            .model_code
            .as_deref()
            .and_then(devices::catalog::by_model_code)
            .map(|model| model.recordings)
            .unwrap_or_default();
        let mut files = Vec::new();
        for volume in self.volumes {
            files.extend(ingest::list_recordings(volume, layout).map_err(|error| {
                format!("Cannot read recordings on {}: {error}", volume.display())
            })?);
        }
        let ledger = self
            .registry
            .load()
            .map_err(|error| error.to_string())?
            .imported;
        let planned = ingest::plan(&self.device.key, files, &ledger);
        let total = planned.len();
        progress(DeviceImportProgress::Planned { total });
        let mut report = DeviceImportReport::default();
        for (index, file) in planned.iter().enumerate() {
            if cancellation.is_cancelled() {
                report.cancelled = true;
                break;
            }
            let name = file.file_name().to_owned();
            let mut copying = |copied| {
                progress(DeviceImportProgress::Copying {
                    index,
                    total,
                    name: name.clone(),
                    copied,
                    size: file.size,
                })
            };
            copying(0);
            match self.import_one(file, &mut copying) {
                Ok((document, sha256)) => {
                    if self.device.delete_after_import
                        && ingest::delete_source(&file.path, &sha256).unwrap_or(false)
                    {
                        report.deleted += 1;
                    }
                    report.imported.push(document);
                }
                Err(error) => report.failed.push((name, error)),
            }
        }
        let summary = ImportSummary {
            finished_at_ms: now_ms(),
            imported: report.imported.len(),
            failed: report.failed.len(),
        };
        let key = self.device.key.clone();
        self.registry
            .update(|registry| {
                if let Some(device) = registry.device_mut(&key) {
                    device.last_import = Some(summary);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(report)
    }

    fn import_one(
        &self,
        file: &SourceFile,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(DocumentState, String), String> {
        let (document, sha256) = match &self.device.import_dir {
            None => {
                let staged = self
                    .workspace_root
                    .join(STAGING_DIRECTORY)
                    .join(format!("{}.part", uuid::Uuid::new_v4().simple()));
                let sha256 = ingest::copy_verified(&file.path, &staged, progress)
                    .map_err(|error| error.to_string())?;
                let document = self
                    .documents
                    .import_packaged_audio(self.workspace_root, &staged, file.file_name())
                    .inspect_err(|_| {
                        let _ = fs::remove_file(&staged);
                    })?;
                (document, sha256)
            }
            Some(directory) => {
                let destination =
                    ingest::unique_destination(directory, file.file_name(), |path| path.exists());
                let mut partial = destination.clone().into_os_string();
                partial.push(".part");
                let partial = PathBuf::from(partial);
                let sha256 = ingest::copy_verified(&file.path, &partial, progress)
                    .map_err(|error| error.to_string())?;
                fs::rename(&partial, &destination).map_err(|error| {
                    let _ = fs::remove_file(&partial);
                    format!("Cannot finish {}: {error}", destination.display())
                })?;
                let path = destination
                    .to_str()
                    .ok_or("The import directory is not valid Unicode")?;
                (self.documents.import_audio(path)?, sha256)
            }
        };
        let record = ImportedFile {
            device: self.device.key.clone(),
            path: file.relative.clone(),
            size: file.size,
            modified_s: file.modified_s,
            document: Some(document.id().as_str().to_owned()),
        };
        self.registry
            .update(|registry| registry.imported.push(record))
            .map_err(|error| error.to_string())?;
        Ok((document, sha256))
    }

    /// Partial copies left by an interrupted import; never a finished file.
    fn clear_partial_copies(&self) {
        let staging = self.workspace_root.join(STAGING_DIRECTORY);
        let custom = self.device.import_dir.as_deref();
        for directory in std::iter::once(staging.as_path()).chain(custom) {
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "part")
                {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
