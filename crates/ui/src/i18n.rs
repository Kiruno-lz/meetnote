use application::{
    AudioSourceError, AudioStatus, DocumentIssueKind, DocumentsRootError, ReservedLocation,
};
use contracts::ComputeDevice;
use domain::ProjectStatus;
use models::{
    BackendCheck, ChecksumResult, ModelFailure, ModelOperation, ProbeRecord, ProbeRecordError,
};

pub use ui_text::{message, message_pair, text, Locale};

pub fn status(locale: Locale, status: ProjectStatus) -> &'static str {
    text(
        locale,
        match status {
            ProjectStatus::Imported => "ready_to_process",
            ProjectStatus::Processing => "processing",
            ProjectStatus::Ready => "ready",
            ProjectStatus::Failed => "failed",
        },
    )
}

pub fn device(locale: Locale, device: ComputeDevice) -> &'static str {
    match (locale, device) {
        (Locale::ZhCn, ComputeDevice::Auto) => "自动",
        (Locale::ZhCn, ComputeDevice::Cpu) => "CPU",
        (Locale::ZhCn, ComputeDevice::Metal) => "Metal",
        (Locale::ZhCn, ComputeDevice::CoreMl) => "Core ML",
        (Locale::EnUs, ComputeDevice::Auto) => "Auto",
        (Locale::EnUs, ComputeDevice::Cpu) => "CPU",
        (Locale::EnUs, ComputeDevice::Metal) => "Metal",
        (Locale::EnUs, ComputeDevice::CoreMl) => "Core ML",
    }
}

/// The sentence the issues panel shows for a scanner issue, with the ids
/// and depths it names. The English sentence is the application's own.
pub fn document_issue(locale: Locale, kind: &DocumentIssueKind) -> String {
    use DocumentIssueKind as Kind;
    match (locale, kind) {
        (Locale::EnUs, kind) => kind.to_string(),
        (Locale::ZhCn, Kind::InvalidPackage { message, .. }) => {
            format!("无法读取这个文档包：{message}。")
        }
        (Locale::ZhCn, Kind::DuplicateDocumentId { document_id, .. }) => {
            format!("另一个文档包已经使用文档 ID {document_id}。")
        }
        (
            Locale::ZhCn,
            Kind::MissingParent {
                document_id,
                parent_id,
            },
        ) => format!("文档 {document_id} 的父文档 {parent_id} 不存在。"),
        (Locale::ZhCn, Kind::Cycle { document_id }) => {
            format!("文档 {document_id} 经父文档链接成为了自己的祖先。")
        }
        (Locale::ZhCn, Kind::MaximumDepth { document_id }) => {
            format!("文档 {document_id} 超出了三层树的深度上限。")
        }
        (Locale::ZhCn, Kind::CrossAudioAsset { document_id }) => {
            format!("文档 {document_id} 的父文档来自另一段录音。")
        }
        (
            Locale::ZhCn,
            Kind::DepthMismatch {
                document_id,
                declared,
                actual,
            },
        ) => format!("文档 {document_id} 声明深度为 {declared}，但按父文档应为 {actual}。"),
        (Locale::ZhCn, Kind::UnexpectedFile { .. }) => {
            "这个文件不是文档包；每个文档都是包含 document.md 和 state.json 的文件夹。".into()
        }
    }
}

/// Why the documents root was not switched.
pub fn documents_root_error(locale: Locale, error: &DocumentsRootError) -> String {
    use DocumentsRootError as Error;
    let reserved_name = |location: &ReservedLocation| match (locale, location) {
        (Locale::ZhCn, ReservedLocation::SpeakerLibrary) => "声纹库",
        (Locale::ZhCn, ReservedLocation::ModelDirectory) => "模型目录",
        (Locale::EnUs, ReservedLocation::SpeakerLibrary) => "Speaker Library",
        (Locale::EnUs, ReservedLocation::ModelDirectory) => "model directory",
    };
    match (locale, error) {
        (Locale::ZhCn, Error::Unsupported) => "这个工作区不能切换文档目录。".into(),
        (Locale::EnUs, Error::Unsupported) => {
            "This workspace cannot switch its documents folder.".into()
        }
        (Locale::ZhCn, Error::Empty) => "文档目录不能为空。".into(),
        (Locale::EnUs, Error::Empty) => "The documents folder must not be empty.".into(),
        (Locale::ZhCn, Error::NotADirectory { path }) => {
            format!("{} 不是文件夹。", path.display())
        }
        (Locale::EnUs, Error::NotADirectory { path }) => {
            format!("{} is not a folder.", path.display())
        }
        (
            Locale::ZhCn,
            Error::Reserved {
                path,
                location,
                reserved,
            },
        ) => format!(
            "{} 与{}（{}）重叠，不能存放文档。",
            path.display(),
            reserved_name(location),
            reserved.display()
        ),
        (
            Locale::EnUs,
            Error::Reserved {
                path,
                location,
                reserved,
            },
        ) => format!(
            "{} overlaps the {} at {}; it cannot hold the documents.",
            path.display(),
            reserved_name(location),
            reserved.display()
        ),
        (Locale::ZhCn, Error::WorkspaceBusy { job_ids }) => format!(
            "有 {} 个处理或助手任务正在运行；等它们结束或取消后再切换文档目录。",
            job_ids.len()
        ),
        (Locale::EnUs, Error::WorkspaceBusy { job_ids }) => format!(
            "{} processing or Assistant job(s) are running; switch the documents folder once they finish or are cancelled.",
            job_ids.len()
        ),
        (Locale::ZhCn, Error::Create { path, message }) => {
            format!("无法创建文档目录 {}：{message}", path.display())
        }
        (Locale::EnUs, Error::Create { path, message }) => {
            format!("Cannot create the documents folder {}: {message}", path.display())
        }
        (Locale::ZhCn, Error::Open { path, message }) => {
            format!("无法打开文档目录 {}：{message}", path.display())
        }
        (Locale::EnUs, Error::Open { path, message }) => {
            format!("Cannot open the documents folder {}: {message}", path.display())
        }
        (Locale::ZhCn, Error::Jobs(message)) => format!("无法读取任务状态：{message}"),
        (Locale::EnUs, Error::Jobs(message)) => format!("Cannot read the jobs: {message}"),
    }
}

/// The first characters of a checksum, enough to tell two apart.
fn short_checksum(checksum: &str) -> &str {
    checksum.get(..12).unwrap_or(checksum)
}

/// What the last check found about a document's source audio.
pub fn audio_status(locale: Locale, status: &AudioStatus) -> String {
    match (locale, status) {
        (Locale::ZhCn, AudioStatus::Present { .. }) => "与处理时的音频一致".into(),
        (Locale::EnUs, AudioStatus::Present { .. }) => "Matches the processed audio".into(),
        (Locale::ZhCn, AudioStatus::Missing { .. }) => "音频文件缺失；重新关联后才能处理".into(),
        (Locale::EnUs, AudioStatus::Missing { .. }) => {
            "The audio file is missing; relink it to process".into()
        }
        (
            Locale::ZhCn,
            AudioStatus::ChecksumMismatch {
                expected, actual, ..
            },
        ) => format!(
            "与处理时的音频不一致（记录 {}…，现为 {}…）",
            short_checksum(expected),
            short_checksum(actual)
        ),
        (
            Locale::EnUs,
            AudioStatus::ChecksumMismatch {
                expected, actual, ..
            },
        ) => format!(
            "Differs from the processed audio (recorded {}…, now {}…)",
            short_checksum(expected),
            short_checksum(actual)
        ),
        (Locale::ZhCn, AudioStatus::Unverifiable { .. }) => {
            "文件存在；没有处理时记录的校验和可供核对".into()
        }
        (Locale::EnUs, AudioStatus::Unverifiable { .. }) => {
            "The file exists; no recorded checksum to verify it against".into()
        }
    }
}

/// Why an audio check or relink failed; a refused relink wrote nothing.
pub fn audio_source_error(locale: Locale, error: &AudioSourceError) -> String {
    use AudioSourceError as Error;
    match (locale, error) {
        (Locale::ZhCn, Error::NotFound(document_id)) => format!("文档 {document_id} 不存在。"),
        (Locale::EnUs, Error::NotFound(document_id)) => {
            format!("Document {document_id} does not exist.")
        }
        (
            Locale::ZhCn,
            Error::RevisionConflict {
                expected, actual, ..
            },
        ) => format!("文档已在别处修改（版本 {expected} → {actual}），音频没有重新关联。"),
        (
            Locale::EnUs,
            Error::RevisionConflict {
                expected, actual, ..
            },
        ) => format!(
            "The document changed elsewhere (revision {expected} → {actual}); the audio was not relinked."
        ),
        (Locale::ZhCn, Error::FileMissing { path }) => format!("所选音频文件不存在：{path}"),
        (Locale::EnUs, Error::FileMissing { path }) => {
            format!("The chosen audio file does not exist: {path}")
        }
        (
            Locale::ZhCn,
            Error::ChecksumMismatch {
                path,
                expected,
                actual,
            },
        ) => format!(
            "{path} 不是处理时的音频（校验和 {}…，应为 {}…），没有写入。",
            short_checksum(actual),
            short_checksum(expected)
        ),
        (
            Locale::EnUs,
            Error::ChecksumMismatch {
                path,
                expected,
                actual,
            },
        ) => format!(
            "{path} is not the processed audio (checksum {}…, expected {}…); nothing was written.",
            short_checksum(actual),
            short_checksum(expected)
        ),
        (Locale::ZhCn, Error::Read { path, message }) => {
            format!("无法读取音频文件 {path}：{message}")
        }
        (Locale::EnUs, Error::Read { path, message }) => {
            format!("Cannot read the audio file {path}: {message}")
        }
        (Locale::ZhCn, Error::Service(message)) => format!("文档服务出错：{message}"),
        (Locale::EnUs, Error::Service(message)) => format!("Document service error: {message}"),
    }
}

/// Why a model download or probe failed, per reason kind, with the facts
/// the reason names.
pub fn model_failure_reason(locale: Locale, failure: &ModelFailure) -> String {
    match (locale, failure) {
        (Locale::ZhCn, ModelFailure::UnsupportedSource { url }) => {
            format!("模型来源不是 HTTP 地址：{url}")
        }
        (Locale::EnUs, ModelFailure::UnsupportedSource { url }) => {
            format!("The model source is not an HTTP URL: {url}")
        }
        (Locale::ZhCn, ModelFailure::Network { message }) => {
            format!("网络请求在自动重试后仍然失败：{message}")
        }
        (Locale::EnUs, ModelFailure::Network { message }) => {
            format!("The download request failed after the automatic retries: {message}")
        }
        (Locale::ZhCn, ModelFailure::Io { message }) => format!("读写模型文件失败：{message}"),
        (Locale::EnUs, ModelFailure::Io { message }) => {
            format!("Reading or writing the model files failed: {message}")
        }
        (
            Locale::ZhCn,
            ModelFailure::SizeMismatch {
                path,
                expected,
                actual,
            },
        ) => format!("{path} 的大小不符：应为 {expected} 字节，实际为 {actual} 字节"),
        (
            Locale::EnUs,
            ModelFailure::SizeMismatch {
                path,
                expected,
                actual,
            },
        ) => format!("{path} has the wrong size: expected {expected} bytes, got {actual}"),
        (
            Locale::ZhCn,
            ModelFailure::ChecksumMismatch {
                path,
                expected,
                actual,
            },
        ) => format!(
            "{path} 的校验和不符：应为 {}…，实际为 {}…",
            short_checksum(expected),
            short_checksum(actual)
        ),
        (
            Locale::EnUs,
            ModelFailure::ChecksumMismatch {
                path,
                expected,
                actual,
            },
        ) => format!(
            "{path} has the wrong checksum: expected {}…, got {}…",
            short_checksum(expected),
            short_checksum(actual)
        ),
        (Locale::ZhCn, ModelFailure::Archive { message }) => {
            format!("无法解压模型归档：{message}")
        }
        (Locale::EnUs, ModelFailure::Archive { message }) => {
            format!("Cannot extract the model archive: {message}")
        }
        (
            Locale::ZhCn,
            ModelFailure::BackendProbe {
                backend_id,
                message,
            },
        ) => format!("后端 {backend_id} 未通过检查：{message}"),
        (
            Locale::EnUs,
            ModelFailure::BackendProbe {
                backend_id,
                message,
            },
        ) => format!("Backend {backend_id} rejected the model: {message}"),
    }
}

/// What failed, in a few words: a backend's load check, the check of the
/// files against the catalog, or the download. A re-probe downloads
/// nothing, so its other failures are failed checks too.
pub fn model_failure_headline(
    locale: Locale,
    operation: ModelOperation,
    failure: &ModelFailure,
) -> &'static str {
    use ModelFailure as Failure;
    match (locale, failure, operation) {
        (Locale::ZhCn, Failure::BackendProbe { .. }, _) => "加载检查未通过",
        (Locale::EnUs, Failure::BackendProbe { .. }, _) => "Load check failed",
        (Locale::ZhCn, Failure::SizeMismatch { .. } | Failure::ChecksumMismatch { .. }, _)
        | (Locale::ZhCn, _, ModelOperation::Reprobe) => "校验未通过",
        (Locale::EnUs, Failure::SizeMismatch { .. } | Failure::ChecksumMismatch { .. }, _)
        | (Locale::EnUs, _, ModelOperation::Reprobe) => "Verification failed",
        (Locale::ZhCn, _, ModelOperation::Download) => "下载失败",
        (Locale::EnUs, _, ModelOperation::Download) => "Download failed",
    }
}

/// Why a model failed, in plain words per reason kind; the paths and the
/// backend's own message are left to `model_failure_reason`.
pub fn model_failure_explanation(locale: Locale, failure: &ModelFailure) -> String {
    use ModelFailure as Failure;
    match (locale, failure) {
        (Locale::ZhCn, Failure::UnsupportedSource { .. }) => "这个来源不支持直接下载".into(),
        (Locale::EnUs, Failure::UnsupportedSource { .. }) => {
            "This source cannot be downloaded directly".into()
        }
        (Locale::ZhCn, Failure::Network { .. }) => "网络连接出错，可稍后重试".into(),
        (Locale::EnUs, Failure::Network { .. }) => "The connection failed; try again later".into(),
        (Locale::ZhCn, Failure::Io { .. }) => "无法读写模型文件".into(),
        (Locale::EnUs, Failure::Io { .. }) => "The model files could not be read or written".into(),
        (Locale::ZhCn, Failure::SizeMismatch { .. }) => "文件大小与目录记录不符".into(),
        (Locale::EnUs, Failure::SizeMismatch { .. }) => {
            "A file's size does not match the catalog".into()
        }
        (Locale::ZhCn, Failure::ChecksumMismatch { .. }) => "文件内容与目录记录不符".into(),
        (Locale::EnUs, Failure::ChecksumMismatch { .. }) => {
            "A file's contents do not match the catalog".into()
        }
        (Locale::ZhCn, Failure::Archive { .. }) => "下载的压缩包无法解压".into(),
        (Locale::EnUs, Failure::Archive { .. }) => {
            "The downloaded archive could not be unpacked".into()
        }
        (Locale::ZhCn, Failure::BackendProbe { backend_id, .. }) => {
            format!("{backend_id} 无法加载这个模型")
        }
        (Locale::EnUs, Failure::BackendProbe { backend_id, .. }) => {
            format!("{backend_id} could not load this model")
        }
    }
}

/// One artifact's checksum check; a mismatch names the checksum found.
pub fn checksum_result(locale: Locale, result: &ChecksumResult) -> String {
    match (locale, result) {
        (_, ChecksumResult::Matched) => text(locale, "checksum_matched").into(),
        (_, ChecksumResult::ArchiveMatched) => text(locale, "checksum_archive_matched").into(),
        (_, ChecksumResult::ArchiveNotRechecked) => {
            text(locale, "checksum_archive_not_rechecked").into()
        }
        (_, ChecksumResult::NotDeclared) => text(locale, "checksum_not_declared").into(),
        (Locale::ZhCn, ChecksumResult::Mismatched { actual_sha256 }) => {
            format!("不一致，实际为 {actual_sha256}")
        }
        (Locale::EnUs, ChecksumResult::Mismatched { actual_sha256 }) => {
            format!("mismatch, found {actual_sha256}")
        }
        (Locale::ZhCn, ChecksumResult::Unreadable { message }) => format!("无法读取：{message}"),
        (Locale::EnUs, ChecksumResult::Unreadable { message }) => format!("unreadable: {message}"),
    }
}

/// One candidate backend's probe.
pub fn backend_check(locale: Locale, check: &BackendCheck) -> String {
    match (locale, check) {
        (_, BackendCheck::Passed) => text(locale, "probe_passed").into(),
        (_, BackendCheck::NotRegistered) => text(locale, "backend_not_registered").into(),
        (Locale::ZhCn, BackendCheck::SkippedChecksumFailure) => "未执行：文件校验失败".into(),
        (Locale::EnUs, BackendCheck::SkippedChecksumFailure) => {
            "not run: file verification failed".into()
        }
        (Locale::ZhCn, BackendCheck::Failed { message }) => format!("未通过：{message}"),
        (Locale::EnUs, BackendCheck::Failed { message }) => format!("failed: {message}"),
    }
}

/// A model's estimated VRAM, `size` already formatted.
pub fn model_vram(locale: Locale, size: &str) -> String {
    match locale {
        Locale::ZhCn => format!("显存约 {size}"),
        Locale::EnUs => format!("~{size} VRAM"),
    }
}

/// The kind, outcome and time of a model's last probe.
pub fn probe_summary(locale: Locale, record: &ProbeRecord) -> String {
    let outcome = text(
        locale,
        if record.passed() {
            "probe_passed"
        } else {
            "probe_failed"
        },
    );
    let kind = text(
        locale,
        match record.kind {
            ModelOperation::Download => "probe_kind_download",
            ModelOperation::Reprobe => "probe_kind_reprobe",
        },
    );
    let time = crate::components::utc_time(record.probed_at_ms);
    match locale {
        Locale::ZhCn => format!("{kind}{outcome} · {time}"),
        Locale::EnUs => format!("{kind} {outcome} · {time}"),
    }
}

/// A probe record file that exists but cannot be read; the next probe
/// replaces it.
pub fn probe_record_error(locale: Locale, error: &ProbeRecordError) -> String {
    let path = error.path.display();
    let reason = &error.reason;
    match locale {
        Locale::ZhCn => format!("无法读取校验记录 {path}：{reason}。下次校验会覆盖它。"),
        Locale::EnUs => {
            format!("Cannot read the probe record {path}: {reason}. The next probe replaces it.")
        }
    }
}

/// How many of a model family's models are downloaded.
pub fn model_family_installed(locale: Locale, installed: usize, total: usize) -> String {
    match locale {
        Locale::ZhCn => format!("已下载 {installed} / {total}"),
        Locale::EnUs => format!("{installed} of {total} downloaded"),
    }
}

/// Which of a download's files is downloading, when it has several.
pub fn download_artifact(locale: Locale, index: usize, count: usize) -> String {
    match locale {
        Locale::ZhCn => format!("文件 {index}/{count}"),
        Locale::EnUs => format!("File {index} of {count}"),
    }
}

/// The time a download has left: seconds under a minute, then minutes and
/// hours rounded to the minute.
pub fn download_eta(locale: Locale, seconds: u64) -> String {
    if seconds < 60 {
        return match locale {
            Locale::ZhCn => format!("约 {seconds} 秒"),
            Locale::EnUs => format!("about {seconds} s left"),
        };
    }
    let minutes = (seconds + 30) / 60;
    match (locale, minutes / 60, minutes % 60) {
        (Locale::ZhCn, 0, minutes) => format!("约 {minutes} 分钟"),
        (Locale::ZhCn, hours, 0) => format!("约 {hours} 小时"),
        (Locale::ZhCn, hours, minutes) => format!("约 {hours} 小时 {minutes} 分钟"),
        (Locale::EnUs, 0, minutes) => format!("about {minutes} min left"),
        (Locale::EnUs, hours, 0) => format!("about {hours} h left"),
        (Locale::EnUs, hours, minutes) => format!("about {hours} h {minutes} min left"),
    }
}

/// A queued download the manager starts again after a dropped connection.
pub fn download_retrying(locale: Locale, attempt: u32) -> String {
    match locale {
        Locale::ZhCn => format!("连接中断，正在第 {attempt} 次重试"),
        Locale::EnUs => format!("Connection lost, retrying (attempt {attempt})"),
    }
}

pub fn download_progress(
    locale: Locale,
    model_id: &str,
    artifact_index: usize,
    artifact_count: usize,
    completed_bytes: u64,
    speed_bytes_per_second: Option<u64>,
) -> String {
    let completed_mb = completed_bytes as f64 / 1_000_000.0;
    let speed = speed_bytes_per_second
        .map(|bytes| format!("{:.1} MB/s", bytes as f64 / 1_000_000.0))
        .unwrap_or_else(|| "— MB/s".into());
    match locale {
        Locale::ZhCn => format!(
            "{model_id} · 文件 {artifact_index}/{artifact_count}：已下载 {completed_mb:.1} MB · 速度 {speed}"
        ),
        Locale::EnUs => format!(
            "{model_id} · artifact {artifact_index}/{artifact_count}: {completed_mb:.1} MB · speed {speed}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_files::SOURCES;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Token<'a> {
        Ident(&'a str),
        Str(&'a str),
        Punct(u8),
    }

    /// Minimal Rust lexer: identifiers, (raw) string literals and ASCII
    /// punctuation. Line comments, char literals, lifetimes and numbers are skipped.
    fn tokens(source: &str) -> Vec<Token<'_>> {
        let bytes = source.as_bytes();
        let mut tokens = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            } else if byte == b'"' {
                let start = index + 1;
                index = start;
                while bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                tokens.push(Token::Str(&source[start..index]));
                index += 1;
            } else if byte == b'\'' {
                let escaped = bytes.get(index + 1) == Some(&b'\\');
                let width = source[index + 1..].chars().next().map_or(1, char::len_utf8);
                if escaped {
                    index += 3;
                    while bytes[index] != b'\'' {
                        index += 1;
                    }
                    index += 1;
                } else if bytes.get(index + 1 + width) == Some(&b'\'') {
                    index += width + 2;
                } else {
                    index += 1;
                }
            } else if byte.is_ascii_alphabetic() || byte == b'_' {
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                let ident = &source[start..index];
                if matches!(ident, "r" | "br") && matches!(bytes.get(index), Some(b'"' | b'#')) {
                    let hashes = bytes[index..]
                        .iter()
                        .take_while(|byte| **byte == b'#')
                        .count();
                    let content = index + hashes + 1;
                    let terminator = format!("\"{}", "#".repeat(hashes));
                    let end = content + source[content..].find(&terminator).unwrap();
                    tokens.push(Token::Str(&source[content..end]));
                    index = end + terminator.len();
                } else {
                    tokens.push(Token::Ident(ident));
                }
            } else if byte.is_ascii_digit() {
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
            } else {
                if byte.is_ascii_punctuation() {
                    tokens.push(Token::Punct(byte));
                }
                index += 1;
            }
        }
        tokens
    }

    /// Key-shaped string literals inside the delimited group that starts at the
    /// first `open` at or after `start`.
    fn group_keys<'a>(tokens: &[Token<'a>], start: usize, open: u8, close: u8) -> Vec<&'a str> {
        let mut keys = Vec::new();
        let mut depth = 0usize;
        for token in tokens[start..]
            .iter()
            .skip_while(|token| **token != Token::Punct(open))
        {
            match *token {
                Token::Punct(byte) if byte == open => depth += 1,
                Token::Punct(byte) if byte == close => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Token::Str(value)
                    if !value.is_empty()
                        && value
                            .bytes()
                            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_')) =>
                {
                    keys.push(value)
                }
                _ => {}
            }
        }
        keys
    }

    /// Keys passed to `text(`/`tr(` (including literals in nested `match`/`if`
    /// arms) plus literals returned by local `fn *_key(..)` helpers.
    fn translation_keys(source: &str) -> Vec<&str> {
        let tokens = tokens(source);
        let mut keys = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            let previous = index.checked_sub(1).map(|previous| tokens[previous]);
            let call = matches!(token, Token::Ident("text" | "tr"))
                && tokens.get(index + 1) == Some(&Token::Punct(b'('))
                && !matches!(previous, Some(Token::Ident("fn") | Token::Punct(b'.')));
            let key_helper = matches!(token, Token::Ident(name) if name.ends_with("_key"))
                && previous == Some(Token::Ident("fn"));
            if call {
                keys.extend(group_keys(&tokens, index, b'(', b')'));
            } else if key_helper {
                keys.extend(group_keys(&tokens, index, b'{', b'}'));
            }
        }
        keys
    }

    #[test]
    fn scanner_finds_direct_nested_and_helper_keys_only() {
        let source = r##"
            fn text(locale: Locale, key: &str) {}
            fn scene_key() -> &'static str { match x { A => "helper_key", _ => "other_helper" } }
            // tr(locale, "commented_out")
            let c = '"'; let q = '\''; let l: &'static str = "not_a_call";
            let raw = r#"tr(locale, "raw_hidden")"#;
            tr(locale, "direct"); i18n::text(locale, match m { A => "arm_a", B => "arm_b" });
            value.text("method_call"); message(locale, "message_key", "Value");
        "##;
        assert_eq!(
            translation_keys(source),
            ["helper_key", "other_helper", "direct", "arm_a", "arm_b"]
        );
    }

    fn every_issue_kind() -> Vec<DocumentIssueKind> {
        use domain::DocumentId;
        let id = DocumentId::new;
        vec![
            DocumentIssueKind::InvalidPackage {
                path: "/work/documents/broken".into(),
                message: "cannot parse /work/documents/broken/state.json".into(),
            },
            DocumentIssueKind::DuplicateDocumentId {
                document_id: id("doc-1"),
                path: "/work/documents/copy".into(),
            },
            DocumentIssueKind::MissingParent {
                document_id: id("doc-2"),
                parent_id: id("gone"),
            },
            DocumentIssueKind::Cycle {
                document_id: id("doc-3"),
            },
            DocumentIssueKind::MaximumDepth {
                document_id: id("doc-4"),
            },
            DocumentIssueKind::CrossAudioAsset {
                document_id: id("doc-5"),
            },
            DocumentIssueKind::DepthMismatch {
                document_id: id("doc-6"),
                declared: 2,
                actual: 1,
            },
            DocumentIssueKind::UnexpectedFile {
                path: "/work/documents/notes.txt".into(),
            },
        ]
    }

    /// Whether `text` has a CJK character, as a Chinese sentence does.
    fn has_chinese(text: &str) -> bool {
        text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
    }

    #[test]
    fn every_scanner_issue_kind_reads_as_a_sentence_in_both_locales() {
        for kind in every_issue_kind() {
            let chinese = document_issue(Locale::ZhCn, &kind);
            let english = document_issue(Locale::EnUs, &kind);
            assert!(has_chinese(&chinese), "{kind:?}: {chinese}");
            assert_eq!(english, kind.to_string(), "{kind:?}");
            let named: Vec<String> = match &kind {
                DocumentIssueKind::InvalidPackage { message, .. } => vec![message.clone()],
                DocumentIssueKind::DuplicateDocumentId { document_id, .. }
                | DocumentIssueKind::Cycle { document_id }
                | DocumentIssueKind::MaximumDepth { document_id }
                | DocumentIssueKind::CrossAudioAsset { document_id } => {
                    vec![document_id.to_string()]
                }
                DocumentIssueKind::MissingParent {
                    document_id,
                    parent_id,
                } => vec![document_id.to_string(), parent_id.to_string()],
                DocumentIssueKind::DepthMismatch {
                    document_id,
                    declared,
                    actual,
                } => vec![
                    document_id.to_string(),
                    declared.to_string(),
                    actual.to_string(),
                ],
                DocumentIssueKind::UnexpectedFile { .. } => vec!["document.md".into()],
            };
            for fact in named {
                assert!(chinese.contains(&fact), "{chinese} names {fact}");
            }
        }
    }

    #[test]
    fn documents_root_errors_and_audio_states_read_in_both_locales() {
        use application::ReservedLocation;
        use std::path::PathBuf;
        let path = PathBuf::from("/work/elsewhere");
        let root_errors = [
            DocumentsRootError::Unsupported,
            DocumentsRootError::Empty,
            DocumentsRootError::NotADirectory { path: path.clone() },
            DocumentsRootError::Reserved {
                path: path.clone(),
                location: ReservedLocation::SpeakerLibrary,
                reserved: "/work/speakers".into(),
            },
            DocumentsRootError::Reserved {
                path: path.clone(),
                location: ReservedLocation::ModelDirectory,
                reserved: "/work/models".into(),
            },
            DocumentsRootError::WorkspaceBusy {
                job_ids: vec![runtime::JobId(4), runtime::JobId(5)],
            },
            DocumentsRootError::Create {
                path: path.clone(),
                message: "permission denied".into(),
            },
            DocumentsRootError::Open {
                path: path.clone(),
                message: "cannot watch".into(),
            },
            DocumentsRootError::Jobs("poisoned".into()),
        ];
        for error in &root_errors {
            let chinese = documents_root_error(Locale::ZhCn, error);
            let english = documents_root_error(Locale::EnUs, error);
            assert!(has_chinese(&chinese), "{error:?}: {chinese}");
            assert!(!english.is_empty() && !has_chinese(&english), "{error:?}");
        }
        let busy = &root_errors[5];
        assert!(documents_root_error(Locale::ZhCn, busy).contains('2'));
        let reserved = [&root_errors[3], &root_errors[4]]
            .map(|error| documents_root_error(Locale::ZhCn, error));
        assert_ne!(reserved[0], reserved[1], "the reserved location is named");
        assert!(reserved[0].contains("/work/speakers"));

        let statuses = [
            AudioStatus::Present {
                path: "a.wav".into(),
                checksum: "ab".repeat(32),
            },
            AudioStatus::Missing {
                path: "a.wav".into(),
            },
            AudioStatus::ChecksumMismatch {
                path: "a.wav".into(),
                expected: "ab".repeat(32),
                actual: "cd".repeat(32),
            },
            AudioStatus::Unverifiable {
                path: "a.wav".into(),
            },
        ];
        let errors = [
            AudioSourceError::NotFound(domain::DocumentId::new("doc")),
            AudioSourceError::RevisionConflict {
                document_id: domain::DocumentId::new("doc"),
                expected: 3,
                actual: 4,
            },
            AudioSourceError::FileMissing {
                path: "b.wav".into(),
            },
            AudioSourceError::ChecksumMismatch {
                path: "b.wav".into(),
                expected: "ab".repeat(32),
                actual: "cd".repeat(32),
            },
            AudioSourceError::Read {
                path: "b.wav".into(),
                message: "denied".into(),
            },
            AudioSourceError::Service("offline".into()),
        ];
        let mut sentences = std::collections::BTreeSet::new();
        for (chinese, english) in statuses
            .iter()
            .map(|status| {
                (
                    audio_status(Locale::ZhCn, status),
                    audio_status(Locale::EnUs, status),
                )
            })
            .chain(errors.iter().map(|error| {
                (
                    audio_source_error(Locale::ZhCn, error),
                    audio_source_error(Locale::EnUs, error),
                )
            }))
        {
            assert!(has_chinese(&chinese), "{chinese}");
            assert!(!english.is_empty() && !has_chinese(&english), "{english}");
            assert!(sentences.insert(chinese), "each state reads differently");
        }
        assert!(audio_source_error(Locale::ZhCn, &errors[1]).contains("3"));
        assert!(audio_source_error(Locale::EnUs, &errors[2]).contains("b.wav"));
    }

    /// Whether a Chinese and an English sentence read as their locales.
    #[track_caller]
    fn assert_localized(chinese: &str, english: &str) {
        assert!(has_chinese(chinese), "{chinese}");
        assert!(!english.is_empty() && !has_chinese(english), "{english}");
    }

    #[test]
    fn every_model_failure_and_probe_result_reads_in_both_locales() {
        use models::{
            BackendCheck, ChecksumResult, ModelFailure, ModelOperation, ProbeOutcome, ProbeRecord,
            ProbeRecordError,
        };
        let failures = [
            (
                ModelFailure::UnsupportedSource {
                    url: "local/model.bin".into(),
                },
                vec!["local/model.bin"],
            ),
            (
                ModelFailure::Network {
                    message: "connection refused".into(),
                },
                vec!["connection refused"],
            ),
            (
                ModelFailure::Io {
                    message: "disk full".into(),
                },
                vec!["disk full"],
            ),
            (
                ModelFailure::SizeMismatch {
                    path: "/models/asr/a/model.bin".into(),
                    expected: 1_024,
                    actual: 512,
                },
                vec!["/models/asr/a/model.bin", "1024", "512"],
            ),
            (
                ModelFailure::ChecksumMismatch {
                    path: "/models/asr/a/model.bin".into(),
                    expected: "ab".repeat(32),
                    actual: "cd".repeat(32),
                },
                vec!["/models/asr/a/model.bin", "abababababab", "cdcdcdcdcdcd"],
            ),
            (
                ModelFailure::Archive {
                    message: "bad tar header".into(),
                },
                vec!["bad tar header"],
            ),
            (
                ModelFailure::BackendProbe {
                    backend_id: "sherpa-onnx".into(),
                    message: "tokens.txt is missing".into(),
                },
                vec!["sherpa-onnx", "tokens.txt is missing"],
            ),
        ];
        let mut reasons = std::collections::BTreeSet::new();
        for (failure, named) in &failures {
            let chinese = model_failure_reason(Locale::ZhCn, failure);
            let english = model_failure_reason(Locale::EnUs, failure);
            assert_localized(&chinese, &english);
            for fact in named {
                assert!(chinese.contains(fact), "{chinese} names {fact}");
                assert!(english.contains(fact), "{english} names {fact}");
            }
            assert!(
                reasons.insert(chinese),
                "each reason kind reads differently"
            );
        }

        // A failure heads its banner with what failed: the download, the
        // check of its files, or a backend's load check; a re-probe never
        // fails as a download.
        let mut explanations = std::collections::BTreeSet::new();
        for (failure, named) in &failures {
            let expected = match failure {
                ModelFailure::BackendProbe { .. } => ["加载检查未通过"; 2],
                ModelFailure::SizeMismatch { .. } | ModelFailure::ChecksumMismatch { .. } => {
                    ["校验未通过"; 2]
                }
                _ => ["下载失败", "校验未通过"],
            };
            for (operation, headline) in [ModelOperation::Download, ModelOperation::Reprobe]
                .into_iter()
                .zip(expected)
            {
                assert_eq!(
                    model_failure_headline(Locale::ZhCn, operation, failure),
                    headline,
                    "{failure:?} {operation:?}"
                );
                assert_localized(
                    headline,
                    model_failure_headline(Locale::EnUs, operation, failure),
                );
            }
            // Plain words for each kind, without the paths or the backend's
            // own message.
            let chinese = model_failure_explanation(Locale::ZhCn, failure);
            let english = model_failure_explanation(Locale::EnUs, failure);
            assert_localized(&chinese, &english);
            assert!(explanations.insert(chinese.clone()), "{chinese}");
            for fact in named {
                for sentence in [&chinese, &english] {
                    // Only the backend is named.
                    let named = *fact == "sherpa-onnx";
                    assert_eq!(sentence.contains(fact), named, "{sentence} and {fact}");
                }
            }
        }
        let (reason, _) = &failures[6];

        let checksums = [
            ChecksumResult::Matched,
            ChecksumResult::ArchiveMatched,
            ChecksumResult::ArchiveNotRechecked,
            ChecksumResult::NotDeclared,
            ChecksumResult::Mismatched {
                actual_sha256: "cd".repeat(32),
            },
            ChecksumResult::Unreadable {
                message: "permission denied".into(),
            },
        ];
        let backends = [
            BackendCheck::Passed,
            BackendCheck::Failed {
                message: "graph does not load".into(),
            },
            BackendCheck::NotRegistered,
            BackendCheck::SkippedChecksumFailure,
        ];
        let mut results = std::collections::BTreeSet::new();
        for (chinese, english) in checksums
            .iter()
            .map(|result| {
                (
                    checksum_result(Locale::ZhCn, result),
                    checksum_result(Locale::EnUs, result),
                )
            })
            .chain(backends.iter().map(|check| {
                (
                    backend_check(Locale::ZhCn, check),
                    backend_check(Locale::EnUs, check),
                )
            }))
        {
            assert_localized(&chinese, &english);
            assert!(results.insert(chinese), "each result reads differently");
        }
        assert!(checksum_result(Locale::ZhCn, &checksums[4]).contains(&"cd".repeat(32)));
        assert!(backend_check(Locale::EnUs, &backends[1]).contains("graph does not load"));

        let summaries = [
            ProbeOutcome::Passed,
            ProbeOutcome::Failed {
                reason: reason.clone(),
            },
        ]
        .map(|outcome| {
            let record = ProbeRecord {
                kind: ModelOperation::Reprobe,
                probed_at_ms: 1_788_000_000_000,
                revision: None,
                checksums: Vec::new(),
                backends: Vec::new(),
                outcome,
            };
            (
                probe_summary(Locale::ZhCn, &record),
                probe_summary(Locale::EnUs, &record),
            )
        });
        assert_ne!(summaries[0], summaries[1], "the outcome is named");
        for (chinese, english) in &summaries {
            assert_localized(chinese, english);
            assert!(chinese.contains("2026-08-29 10:40 UTC"), "{chinese}");
        }

        let error = ProbeRecordError {
            path: "/models/asr/a.probe.json".into(),
            reason: "record belongs to model b".into(),
        };
        let (chinese, english) = (
            probe_record_error(Locale::ZhCn, &error),
            probe_record_error(Locale::EnUs, &error),
        );
        assert_localized(&chinese, &english);
        for sentence in [&chinese, &english] {
            assert!(sentence.contains("/models/asr/a.probe.json"), "{sentence}");
            assert!(sentence.contains("record belongs to model b"), "{sentence}");
        }
    }

    #[test]
    fn every_ui_key_in_source_has_chinese_and_english_text() {
        let mut scanned = std::collections::BTreeSet::new();
        let mut missing = Vec::new();
        for (file, source) in SOURCES {
            for key in translation_keys(source) {
                if !scanned.insert(key) {
                    continue;
                }
                for locale in [Locale::ZhCn, Locale::EnUs] {
                    if text(locale, key).is_empty() {
                        missing.push(format!("{file}: {key} ({})", locale.code()));
                    }
                }
            }
        }
        assert!(
            scanned.len() > 100,
            "source scan found only {} keys",
            scanned.len()
        );
        assert!(missing.is_empty(), "missing translations: {missing:#?}");
    }
}
