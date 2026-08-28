use contracts::ComputeDevice;
use domain::ProjectStatus;

#[cfg(test)]
pub const KEYS: &[&str] = &[
    "assistant",
    "meetings",
    "models",
    "settings",
    "local_intelligence",
    "ask_library",
    "ask_placeholder",
    "send",
    "organize_library",
    "find_project",
    "split_project",
    "audio_projects",
    "import_audio",
    "no_projects",
    "open_detail",
    "process",
    "cancel",
    "summary",
    "original",
    "merge_speaker",
    "no_summary",
    "need_process",
    "empty_meetings",
    "project_detail",
    "catalog_description",
    "download",
    "use_settings",
    "no_models",
    "interface",
    "language",
    "transcription",
    "asr_backend",
    "asr_model",
    "transcription_language",
    "speaker_separation",
    "diarization_backend",
    "diarization_model",
    "expected_speakers",
    "runtime",
    "compute_device",
    "auto",
    "not_selected",
    "ready",
    "processing",
    "failed",
    "ready_to_process",
    "models_active",
    "model_status",
    "ask_notice",
    "choose_audio",
    "missing",
    "interface_description",
    "transcription_description",
    "speaker_description",
    "runtime_description",
    "settings_description",
    "saved_automatically",
    "models_count",
    "download_queue",
    "pause",
    "resume",
    "empty_request",
    "language_changed",
    "hierarchy_updated",
    "order_updated",
    "download_updated",
    "speaker_merged",
    "cancellation_requested",
    "select_project_first",
    "project_missing",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    ZhCn,
    EnUs,
}

impl Locale {
    pub fn from_code(code: &str) -> Self {
        if code.to_ascii_lowercase().starts_with("en") {
            Self::EnUs
        } else {
            Self::ZhCn
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }
}

pub fn text(locale: Locale, key: &str) -> &'static str {
    match (locale, key) {
        (Locale::ZhCn, "assistant") => "助手",
        (Locale::ZhCn, "meetings") => "项目",
        (Locale::ZhCn, "models") => "模型",
        (Locale::ZhCn, "settings") => "设置",
        (Locale::ZhCn, "local_intelligence") => "本地语音资料整理",
        (Locale::ZhCn, "ask_library") => "询问你的语音库",
        (Locale::ZhCn, "ask_placeholder") => "询问你的录音……",
        (Locale::ZhCn, "send") => "发送",
        (Locale::ZhCn, "organize_library") => "整理语音库",
        (Locale::ZhCn, "find_project") => "查找项目",
        (Locale::ZhCn, "split_project") => "拆分项目",
        (Locale::ZhCn, "audio_projects") => "音频项目",
        (Locale::ZhCn, "import_audio") => "导入音频",
        (Locale::ZhCn, "no_projects") => "还没有项目。导入音频开始整理。",
        (Locale::ZhCn, "open_detail") => "打开详情",
        (Locale::ZhCn, "process") => "处理项目",
        (Locale::ZhCn, "cancel") => "取消",
        (Locale::ZhCn, "summary") => "总结文",
        (Locale::ZhCn, "original") => "原始文",
        (Locale::ZhCn, "merge_speaker") => "合并 speaker-1 到 speaker-0",
        (Locale::ZhCn, "no_summary") => "还没有总结。可以使用助手整理这个项目。",
        (Locale::ZhCn, "need_process") => "请先处理项目以生成带 speaker 的转录。",
        (Locale::ZhCn, "empty_meetings") => "导入音频以创建项目。",
        (Locale::ZhCn, "project_detail") => "项目详情",
        (Locale::ZhCn, "catalog_description") => "模型目录、安装状态和下载任务",
        (Locale::ZhCn, "download") => "下载",
        (Locale::ZhCn, "use_settings") => "用于设置",
        (Locale::ZhCn, "no_models") => "模型目录中没有可用模型。",
        (Locale::ZhCn, "interface") => "界面",
        (Locale::ZhCn, "language") => "界面语言",
        (Locale::ZhCn, "transcription") => "转录",
        (Locale::ZhCn, "asr_backend") => "ASR 后端",
        (Locale::ZhCn, "asr_model") => "ASR 模型",
        (Locale::ZhCn, "transcription_language") => "转录语言",
        (Locale::ZhCn, "speaker_separation") => "说话人分离",
        (Locale::ZhCn, "diarization_backend") => "Diarization 后端",
        (Locale::ZhCn, "diarization_model") => "Diarization 模型",
        (Locale::ZhCn, "expected_speakers") => "预期说话人数",
        (Locale::ZhCn, "runtime") => "运行时",
        (Locale::ZhCn, "compute_device") => "计算设备",
        (Locale::ZhCn, "auto") => "自动",
        (Locale::ZhCn, "not_selected") => "未选择",
        (Locale::ZhCn, "ready") => "就绪",
        (Locale::ZhCn, "processing") => "处理中",
        (Locale::ZhCn, "failed") => "失败",
        (Locale::ZhCn, "ready_to_process") => "等待处理",
        (Locale::ZhCn, "models_active") => "下载中",
        (Locale::ZhCn, "model_status") => "模型状态",
        (Locale::ZhCn, "ask_notice") => "助手输入已就绪；不会保存对话历史。",
        (Locale::ZhCn, "choose_audio") => "选择音频",
        (Locale::ZhCn, "missing") => "缺失",
        (Locale::ZhCn, "interface_description") => "界面偏好与本地化选项",
        (Locale::ZhCn, "transcription_description") => "选择语音识别后端、模型和转录语言",
        (Locale::ZhCn, "speaker_description") => "选择说话人分离后端、模型和人数提示",
        (Locale::ZhCn, "runtime_description") => "控制本地推理使用的计算设备",
        (Locale::ZhCn, "settings_description") => "后端、模型、运行设备和界面语言",
        (Locale::ZhCn, "saved_automatically") => "设置会在选择后自动保存",
        (Locale::ZhCn, "models_count") => "个模型",
        (Locale::ZhCn, "download_queue") => "下载任务",
        (Locale::ZhCn, "pause") => "暂停",
        (Locale::ZhCn, "resume") => "继续",
        (Locale::ZhCn, "empty_request") => "发送前请输入内容。",
        (Locale::ZhCn, "language_changed") => "界面语言已切换为中文。",
        (Locale::ZhCn, "hierarchy_updated") => "项目层级已更新。",
        (Locale::ZhCn, "order_updated") => "项目顺序已更新。",
        (Locale::ZhCn, "download_updated") => "下载任务已更新。",
        (Locale::ZhCn, "speaker_merged") => "已将 speaker-1 合并到 speaker-0，并更新 Markdown。",
        (Locale::ZhCn, "cancellation_requested") => "已请求取消……",
        (Locale::ZhCn, "select_project_first") => "请先选择一个项目。",
        (Locale::ZhCn, "project_missing") => "选中的项目已不存在。",
        (Locale::EnUs, "assistant") => "Assistant",
        (Locale::EnUs, "meetings") => "Meetings",
        (Locale::EnUs, "models") => "Models",
        (Locale::EnUs, "settings") => "Settings",
        (Locale::EnUs, "local_intelligence") => "local voice library",
        (Locale::EnUs, "ask_library") => "Ask your voice library",
        (Locale::EnUs, "ask_placeholder") => "Ask about your recordings…",
        (Locale::EnUs, "send") => "Send",
        (Locale::EnUs, "organize_library") => "Organize voice library",
        (Locale::EnUs, "find_project") => "Find a project",
        (Locale::EnUs, "split_project") => "Split a project",
        (Locale::EnUs, "audio_projects") => "AUDIO PROJECTS",
        (Locale::EnUs, "import_audio") => "Import audio",
        (Locale::EnUs, "no_projects") => "No projects yet. Import audio to begin.",
        (Locale::EnUs, "open_detail") => "Open detail",
        (Locale::EnUs, "process") => "Process project",
        (Locale::EnUs, "cancel") => "Cancel",
        (Locale::EnUs, "summary") => "Summary",
        (Locale::EnUs, "original") => "Original",
        (Locale::EnUs, "merge_speaker") => "Merge speaker-1 into speaker-0",
        (Locale::EnUs, "no_summary") => {
            "No summary yet. Use the Assistant to organize this project."
        }
        (Locale::EnUs, "need_process") => {
            "Run processing to generate a speaker-labelled transcript."
        }
        (Locale::EnUs, "empty_meetings") => "Import audio to create a project.",
        (Locale::EnUs, "project_detail") => "PROJECT DETAIL",
        (Locale::EnUs, "catalog_description") => "Catalog, installation state, and download tasks",
        (Locale::EnUs, "download") => "Download",
        (Locale::EnUs, "use_settings") => "Use in settings",
        (Locale::EnUs, "no_models") => "No models are available in the catalog.",
        (Locale::EnUs, "interface") => "Interface",
        (Locale::EnUs, "language") => "Language / Interface locale",
        (Locale::EnUs, "transcription") => "Transcription",
        (Locale::EnUs, "asr_backend") => "ASR backend",
        (Locale::EnUs, "asr_model") => "ASR model",
        (Locale::EnUs, "transcription_language") => "Transcription language",
        (Locale::EnUs, "speaker_separation") => "Speaker separation",
        (Locale::EnUs, "diarization_backend") => "Diarization backend",
        (Locale::EnUs, "diarization_model") => "Diarization model",
        (Locale::EnUs, "expected_speakers") => "Expected speakers",
        (Locale::EnUs, "runtime") => "Runtime",
        (Locale::EnUs, "compute_device") => "Compute device",
        (Locale::EnUs, "auto") => "Auto",
        (Locale::EnUs, "not_selected") => "Not selected",
        (Locale::EnUs, "ready") => "Ready",
        (Locale::EnUs, "processing") => "Processing",
        (Locale::EnUs, "failed") => "Failed",
        (Locale::EnUs, "ready_to_process") => "Ready to process",
        (Locale::EnUs, "models_active") => "Downloads active",
        (Locale::EnUs, "model_status") => "Model status",
        (Locale::EnUs, "ask_notice") => {
            "Assistant input is ready; no conversation history is stored."
        }
        (Locale::EnUs, "choose_audio") => "Choose audio",
        (Locale::EnUs, "missing") => "Missing",
        (Locale::EnUs, "interface_description") => "Interface preferences and localization",
        (Locale::EnUs, "transcription_description") => {
            "Choose the speech recognition backend, model, and language"
        }
        (Locale::EnUs, "speaker_description") => {
            "Choose the speaker separation backend, model, and speaker hint"
        }
        (Locale::EnUs, "runtime_description") => {
            "Control the compute device used for local inference"
        }
        (Locale::EnUs, "settings_description") => {
            "Backends, models, compute device, and interface language"
        }
        (Locale::EnUs, "saved_automatically") => "Settings are saved automatically after selection",
        (Locale::EnUs, "models_count") => "models",
        (Locale::EnUs, "download_queue") => "Download tasks",
        (Locale::EnUs, "pause") => "Pause",
        (Locale::EnUs, "resume") => "Resume",
        (Locale::EnUs, "empty_request") => "Type a request before sending it.",
        (Locale::EnUs, "language_changed") => "Interface language changed to English.",
        (Locale::EnUs, "hierarchy_updated") => "Project hierarchy updated.",
        (Locale::EnUs, "order_updated") => "Project order updated.",
        (Locale::EnUs, "download_updated") => "Download task updated.",
        (Locale::EnUs, "speaker_merged") => "Merged speaker-1 into speaker-0 and updated Markdown.",
        (Locale::EnUs, "cancellation_requested") => "Cancellation requested…",
        (Locale::EnUs, "select_project_first") => "Select a project first.",
        (Locale::EnUs, "project_missing") => "Selected project no longer exists.",
        _ => "",
    }
}

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

pub fn install_state(locale: Locale, state: models::ModelInstallState) -> &'static str {
    match state {
        models::ModelInstallState::Missing => match locale {
            Locale::ZhCn => "缺失",
            Locale::EnUs => "Missing",
        },
        models::ModelInstallState::Partial => match locale {
            Locale::ZhCn => "部分安装",
            Locale::EnUs => "Partial",
        },
        models::ModelInstallState::Installed => match locale {
            Locale::ZhCn => "已安装",
            Locale::EnUs => "Installed",
        },
    }
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

pub fn message(locale: Locale, key: &str, value: &str) -> String {
    match (locale, key) {
        (Locale::ZhCn, "imported") => format!("已导入 {value}"),
        (Locale::EnUs, "imported") => format!("Imported {value}"),
        (Locale::ZhCn, "import_failed") => format!("导入失败：{value}"),
        (Locale::EnUs, "import_failed") => format!("Import failed: {value}"),
        (Locale::ZhCn, "processing") => format!("正在处理 {value}……"),
        (Locale::EnUs, "processing") => format!("Processing {value}…"),
        (Locale::ZhCn, "finished") => format!("已完成 {value}"),
        (Locale::EnUs, "finished") => format!("Finished {value}"),
        (Locale::ZhCn, "processing_failed") => format!("处理失败：{value}"),
        (Locale::EnUs, "processing_failed") => format!("Processing failed: {value}"),
        (Locale::ZhCn, "backend_selected") => format!("已选择 {value} 后端"),
        (Locale::EnUs, "backend_selected") => format!("Backend selected: {value}"),
        (Locale::ZhCn, "settings_failed") => format!("设置更新失败：{value}"),
        (Locale::EnUs, "settings_failed") => format!("Settings update failed: {value}"),
        (Locale::ZhCn, "download_started") => format!("正在下载 {value}……"),
        (Locale::EnUs, "download_started") => format!("Downloading {value}…"),
        (Locale::ZhCn, "download_failed") => format!("下载失败：{value}"),
        (Locale::EnUs, "download_failed") => format!("Download failed: {value}"),
        (Locale::ZhCn, "installed") => format!("已安装 {value}"),
        (Locale::EnUs, "installed") => format!("Installed {value}"),
        (Locale::ZhCn, "cancelled") => format!("已取消 {value}"),
        (Locale::EnUs, "cancelled") => format!("Cancelled {value}"),
        (Locale::ZhCn, "request_received") => format!("已收到请求：{value}（LLM 服务尚未接入。）"),
        (Locale::EnUs, "request_received") => {
            format!("Request received: {value} (LLM service is not connected yet.)")
        }
        (Locale::ZhCn, "model_not_ready") => format!("模型尚未就绪：{value}"),
        (Locale::EnUs, "model_not_ready") => format!("Model is not ready: {value}"),
        (Locale::ZhCn, "move_failed") => format!("无法移动项目：{value}"),
        (Locale::EnUs, "move_failed") => format!("Cannot move project: {value}"),
        (Locale::ZhCn, "reorder_failed") => format!("无法调整项目顺序：{value}"),
        (Locale::EnUs, "reorder_failed") => format!("Cannot reorder project: {value}"),
        (Locale::ZhCn, "speaker_edit_failed") => format!("说话人编辑失败：{value}"),
        (Locale::EnUs, "speaker_edit_failed") => format!("Speaker edit failed: {value}"),
        (Locale::ZhCn, "asr_model_required") => {
            format!("请先为 {value} 选择已安装的 ASR 模型。")
        }
        (Locale::EnUs, "asr_model_required") => {
            format!("Select an installed ASR model for {value} first.")
        }
        (Locale::ZhCn, "diarization_model_required") => {
            format!("请先为 {value} 选择已安装的说话人分离模型。")
        }
        (Locale::EnUs, "diarization_model_required") => {
            format!("Select an installed diarization model for {value} first.")
        }
        (Locale::ZhCn, "job_started") => format!("{value} 已开始……"),
        (Locale::EnUs, "job_started") => format!("{value} started…"),
        (Locale::ZhCn, "job_completed") => format!("{value} 已完成……"),
        (Locale::EnUs, "job_completed") => format!("{value} completed…"),
        (Locale::ZhCn, "job_cancelled") => format!("{value} 已取消……"),
        (Locale::EnUs, "job_cancelled") => format!("{value} cancelled…"),
        (Locale::ZhCn, "job_failed") => {
            let mut parts = value.splitn(2, '|');
            format!(
                "{} 失败：{}",
                parts.next().unwrap_or("Job"),
                parts.next().unwrap_or(value)
            )
        }
        (Locale::EnUs, "job_failed") => {
            let mut parts = value.splitn(2, '|');
            format!(
                "{} failed: {}",
                parts.next().unwrap_or("Job"),
                parts.next().unwrap_or(value)
            )
        }
        (Locale::ZhCn, "job_progress_total") => {
            let mut parts = value.splitn(3, '|');
            format!(
                "{} 进度：{}/{}",
                parts.next().unwrap_or("Job"),
                parts.next().unwrap_or("0"),
                parts.next().unwrap_or("0")
            )
        }
        (Locale::EnUs, "job_progress_total") => {
            let mut parts = value.splitn(3, '|');
            format!(
                "{} progress: {}/{}",
                parts.next().unwrap_or("Job"),
                parts.next().unwrap_or("0"),
                parts.next().unwrap_or("0")
            )
        }
        (Locale::ZhCn, "job_progress") => {
            let mut parts = value.splitn(2, '|');
            format!(
                "{} 进度：{}",
                parts.next().unwrap_or("Job"),
                parts.next().unwrap_or("0")
            )
        }
        (Locale::EnUs, "job_progress") => {
            let mut parts = value.splitn(2, '|');
            format!(
                "{} progress: {}",
                parts.next().unwrap_or("Job"),
                parts.next().unwrap_or("0")
            )
        }
        _ => value.to_string(),
    }
}

pub fn download_progress(
    locale: Locale,
    model_id: &str,
    artifact_index: usize,
    artifact_count: usize,
    completed_bytes: u64,
) -> String {
    match locale {
        Locale::ZhCn => format!(
            "{model_id} · 文件 {artifact_index}/{artifact_count}：已下载 {completed_bytes} 字节"
        ),
        Locale::EnUs => format!(
            "{model_id} · artifact {artifact_index}/{artifact_count}: {completed_bytes} bytes"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ui_key_has_chinese_and_english_text() {
        for key in KEYS {
            assert!(!text(Locale::ZhCn, key).is_empty(), "missing zh key: {key}");
            assert!(!text(Locale::EnUs, key).is_empty(), "missing en key: {key}");
        }
    }
}
