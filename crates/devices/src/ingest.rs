//! Which recordings on a device are new, and copying them so that a copy
//! is only ever used once its bytes match the source.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub use crate::catalog::Recordings;
use crate::registry::ImportedFile;

const AUDIO_EXTENSIONS: [&str; 5] = ["wav", "mp3", "m4a", "aac", "flac"];
const SKIPPED_DIRECTORIES: [&str; 2] = ["System Volume Information", "$RECYCLE.BIN"];
/// Device volumes keep recordings a few folders down at most.
const MAX_DEPTH: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    /// Path inside the volume with `/` separators: the ledger's identity.
    pub relative: String,
    pub path: PathBuf,
    pub size: u64,
    pub modified_s: i64,
}

impl SourceFile {
    pub fn file_name(&self) -> &str {
        self.relative.rsplit('/').next().unwrap_or(&self.relative)
    }
}

pub fn is_recording(name: &str) -> bool {
    !name.starts_with('.')
        && Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                AUDIO_EXTENSIONS
                    .iter()
                    .any(|known| extension.eq_ignore_ascii_case(known))
            })
}

/// Every recording on `volume` the model's layout imports, skipping hidden
/// entries and system folders.
pub fn list_recordings(volume: &Path, layout: Recordings) -> io::Result<Vec<SourceFile>> {
    let mut files = Vec::new();
    collect(volume, volume, 0, &mut files)?;
    Ok(match layout {
        Recordings::All => files,
        Recordings::OriginalsOverEdits => prefer_originals(files),
    })
}

/// [`Recordings::OriginalsOverEdits`]: DJI recorders write each take twice, `…_orig.wav` untouched and
/// `…_edit.wav` with on-device processing; only the original is imported.
/// An `_edit` file without its original is kept.
pub fn prefer_originals(files: Vec<SourceFile>) -> Vec<SourceFile> {
    let original_of = |file: &SourceFile| {
        let (stem, extension) = file.relative.rsplit_once('.')?;
        let take = stem.strip_suffix("_edit")?;
        Some(format!("{take}_orig.{extension}"))
    };
    let keep: Vec<bool> = files
        .iter()
        .map(|file| {
            original_of(file).is_none_or(|original| {
                !files
                    .iter()
                    .any(|other| other.relative.eq_ignore_ascii_case(&original))
            })
        })
        .collect();
    files
        .into_iter()
        .zip(keep)
        .filter_map(|(file, keep)| keep.then_some(file))
        .collect()
}

fn collect(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<SourceFile>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || SKIPPED_DIRECTORIES.contains(&name) {
            continue;
        }
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                collect(root, &path, depth + 1, files)?;
            }
        } else if kind.is_file() && is_recording(name) {
            let metadata = entry.metadata()?;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            files.push(SourceFile {
                relative,
                path,
                size: metadata.len(),
                modified_s: modified_seconds(&metadata),
            });
        }
    }
    Ok(())
}

fn modified_seconds(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs() as i64)
}

/// The recordings in `files` the ledger has not seen, oldest first.
pub fn plan<'a>(
    device: &str,
    files: Vec<SourceFile>,
    ledger: impl IntoIterator<Item = &'a ImportedFile>,
) -> Vec<SourceFile> {
    let seen: Vec<&ImportedFile> = ledger
        .into_iter()
        .filter(|file| file.device == device)
        .collect();
    let mut new: Vec<SourceFile> = files
        .into_iter()
        .filter(|file| {
            !seen.iter().any(|known| {
                known.path == file.relative
                    && known.size == file.size
                    && known.modified_s == file.modified_s
            })
        })
        .collect();
    new.sort_by(|a, b| {
        a.modified_s
            .cmp(&b.modified_s)
            .then(a.relative.cmp(&b.relative))
    });
    new
}

/// `name` in `directory`, or `stem (n).ext` for the first free `n`.
pub fn unique_destination(directory: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let candidate = directory.join(name);
    if !taken(&candidate) {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name);
    let extension = path.extension().and_then(|extension| extension.to_str());
    (2..)
        .map(|n| {
            directory.join(match extension {
                Some(extension) => format!("{stem} ({n}).{extension}"),
                None => format!("{stem} ({n})"),
            })
        })
        .find(|candidate| !taken(candidate))
        .expect("an unbounded range always finds a free name")
}

#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("the copy of {} does not match the recording on the device", path.display())]
    Mismatch { path: PathBuf },
}

/// Copies `source` to `destination` (replacing a stale partial copy), syncs
/// it, then reads the copy back. Returns the SHA-256 of the bytes, which the
/// source and the copy share.
pub fn copy_verified(
    source: &Path,
    destination: &Path,
    mut progress: impl FnMut(u64),
) -> Result<String, CopyError> {
    let read_error = |source_error| CopyError::Read {
        path: source.to_path_buf(),
        source: source_error,
    };
    let write_error = |source_error| CopyError::Write {
        path: destination.to_path_buf(),
        source: source_error,
    };
    let mut input = File::open(source).map_err(read_error)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(write_error)?;
    }
    let mut output = File::create(destination).map_err(write_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    let mut copied = 0;
    loop {
        let read = input.read(&mut buffer).map_err(read_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read]).map_err(write_error)?;
        copied += read as u64;
        progress(copied);
    }
    output.sync_all().map_err(write_error)?;
    drop(output);
    let expected = hex(&hasher.finalize());
    if sha256_file(destination).map_err(read_error)? != expected {
        return Err(CopyError::Mismatch {
            path: source.to_path_buf(),
        });
    }
    Ok(expected)
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Deletes the device's copy once a second read of it still matches the
/// imported bytes; a recording that changed meanwhile stays.
pub fn delete_source(source: &Path, sha256: &str) -> io::Result<bool> {
    if sha256_file(source)? != sha256 {
        return Ok(false);
    }
    fs::remove_file(source)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(relative: &str, size: u64, modified_s: i64) -> SourceFile {
        SourceFile {
            relative: relative.into(),
            path: PathBuf::from("/Volumes/DJI").join(relative),
            size,
            modified_s,
        }
    }

    #[test]
    fn keeps_only_the_original_of_each_dji_take() {
        let kept = prefer_originals(vec![
            file(
                "TX_MIC001_20261120_101010/TX00_MIC001_20261120_101010_orig.wav",
                1,
                1,
            ),
            file(
                "TX_MIC001_20261120_101010/TX00_MIC001_20261120_101010_edit.wav",
                1,
                1,
            ),
            file(
                "TX_MIC002_20261120_111111/TX00_MIC002_20261120_111111_edit.wav",
                1,
                1,
            ),
            file("memo.wav", 1, 1),
        ]);
        let names: Vec<_> = kept.iter().map(SourceFile::file_name).collect();
        assert_eq!(
            names,
            [
                "TX00_MIC001_20261120_101010_orig.wav",
                "TX00_MIC002_20261120_111111_edit.wav",
                "memo.wav"
            ],
            "an edit without its original stays"
        );
    }

    fn ledger(device: &str, relative: &str, size: u64, modified_s: i64) -> ImportedFile {
        ImportedFile {
            device: device.into(),
            path: relative.into(),
            size,
            modified_s,
            document: None,
        }
    }

    #[test]
    fn plans_new_and_changed_recordings_oldest_first() {
        let files = vec![
            file("DJI_Audio/DJI_03.WAV", 30, 300),
            file("DJI_Audio/DJI_01.WAV", 10, 100),
            file("DJI_Audio/DJI_02.WAV", 20, 200),
        ];
        let known = [
            ledger("sn:A", "DJI_Audio/DJI_01.WAV", 10, 100),
            ledger("sn:A", "DJI_Audio/DJI_02.WAV", 21, 200),
            ledger("sn:B", "DJI_Audio/DJI_03.WAV", 30, 300),
        ];
        let planned: Vec<_> = plan("sn:A", files, &known)
            .into_iter()
            .map(|file| file.relative)
            .collect();
        assert_eq!(planned, ["DJI_Audio/DJI_02.WAV", "DJI_Audio/DJI_03.WAV"]);
    }

    #[test]
    fn recognises_audio_and_skips_hidden_and_system_entries() {
        assert!(is_recording("DJI_01.WAV") && is_recording("a.m4a"));
        assert!(!is_recording("._DJI_01.WAV") && !is_recording("notes.txt"));
        let volume = tempfile::tempdir().unwrap();
        let root = volume.path();
        for path in [
            "DJI_Audio/DJI_01.WAV",
            "DJI_Audio/.hidden.wav",
            ".Spotlight-V100/x.wav",
            "System Volume Information/y.wav",
            "MISC/readme.txt",
            "top.mp3",
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        let mut found: Vec<_> = list_recordings(root, Recordings::All)
            .unwrap()
            .into_iter()
            .map(|file| file.relative)
            .collect();
        found.sort();
        assert_eq!(found, ["DJI_Audio/DJI_01.WAV", "top.mp3"]);
    }

    #[test]
    fn the_model_layout_decides_whether_edits_are_imported() {
        let volume = tempfile::tempdir().unwrap();
        for name in ["T_orig.WAV", "T_edit.WAV"] {
            fs::write(volume.path().join(name), b"x").unwrap();
        }
        let count = |layout| list_recordings(volume.path(), layout).unwrap().len();
        assert_eq!(count(Recordings::All), 2);
        assert_eq!(count(Recordings::OriginalsOverEdits), 1);
    }

    #[test]
    fn unique_destination_numbers_taken_names() {
        let taken = |path: &Path| {
            ["/in/DJI_01.WAV", "/in/DJI_01 (2).WAV"]
                .iter()
                .any(|known| Path::new(known) == path)
        };
        assert_eq!(
            unique_destination(Path::new("/in"), "DJI_01.WAV", taken),
            PathBuf::from("/in/DJI_01 (3).WAV")
        );
        assert_eq!(
            unique_destination(Path::new("/in"), "DJI_02.WAV", taken),
            PathBuf::from("/in/DJI_02.WAV")
        );
    }

    #[test]
    fn copies_with_a_verified_hash_and_deletes_only_unchanged_sources() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("DJI_01.WAV");
        let copy = directory.path().join("out/DJI_01.WAV.part");
        fs::write(&source, b"recording bytes").unwrap();
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        fs::write(&copy, b"stale partial copy that is longer").unwrap();
        let mut reported = 0;
        let hash = copy_verified(&source, &copy, |bytes| reported = bytes).unwrap();
        assert_eq!(reported, 15);
        assert_eq!(fs::read(&copy).unwrap(), b"recording bytes");
        assert_eq!(hash, sha256_file(&source).unwrap());

        fs::write(&source, b"changed on device").unwrap();
        assert!(!delete_source(&source, &hash).unwrap());
        assert!(source.exists());
        fs::write(&source, b"recording bytes").unwrap();
        assert!(delete_source(&source, &hash).unwrap());
        assert!(!source.exists());
    }

    #[test]
    fn a_missing_source_is_a_read_error_naming_it() {
        let directory = tempfile::tempdir().unwrap();
        let error = copy_verified(
            &directory.path().join("gone.wav"),
            &directory.path().join("copy.part"),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(error, CopyError::Read { .. }));
        assert!(error.to_string().contains("gone.wav"));
    }
}
