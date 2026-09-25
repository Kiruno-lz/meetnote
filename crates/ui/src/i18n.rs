use contracts::ComputeDevice;
use domain::ProjectStatus;

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
        (Locale::ZhCn, "downloaded") => "已下载",
        (Locale::ZhCn, "not_downloaded") => "未下载",
        (Locale::ZhCn, "model_size") => "权重大小",
        (Locale::ZhCn, "vram_estimate") => "显存预估",
        (Locale::ZhCn, "applicable_backends") => "适用后端",
        (Locale::ZhCn, "no_models") => "模型目录中没有可用模型。",
        (Locale::ZhCn, "appearance") => "外观",
        (Locale::ZhCn, "theme") => "主题",
        (Locale::ZhCn, "theme_system") => "跟随系统",
        (Locale::ZhCn, "theme_light") => "浅色",
        (Locale::ZhCn, "theme_dark") => "深色",
        (Locale::ZhCn, "material") => "材质",
        (Locale::ZhCn, "material_glass") => "毛玻璃",
        (Locale::ZhCn, "material_opaque") => "不透明",
        (Locale::ZhCn, "interface") => "界面",
        (Locale::ZhCn, "language") => "界面语言",
        (Locale::ZhCn, "transcription") => "转录",
        (Locale::ZhCn, "asr_backend") => "ASR 后端",
        (Locale::ZhCn, "asr_model") => "ASR 模型",
        (Locale::ZhCn, "transcription_language") => "转录语言",
        (Locale::ZhCn, "speaker_separation") => "说话人分离",
        (Locale::ZhCn, "diarization_backend") => "Diarization 后端",
        (Locale::ZhCn, "diarization_model") => "Diarization 模型",
        (Locale::ZhCn, "diarization_segmentation_model") => "分离 segmentation 模型",
        (Locale::ZhCn, "diarization_embedding_model") => "分离 embedding 模型",
        (Locale::ZhCn, "diarization_clustering") => "分离 clustering 算法",
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
        (Locale::ZhCn, "speaker_library_path") => "声纹库地址",
        (Locale::ZhCn, "choose_directory") => "选择目录",
        (Locale::ZhCn, "speaker_library_migration_title") => "是否迁移现有声纹库？",
        (Locale::ZhCn, "speaker_library_migration_description") => {
            "检测到现有声纹库。迁移会将副本写入新位置，原文件会保留。"
        }
        (Locale::ZhCn, "migrate") => "迁移并使用",
        (Locale::ZhCn, "use_new_path") => "仅使用新位置",
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
        (Locale::ZhCn, "settings_saved") => "设置已保存。",
        (Locale::ZhCn, "settings_conflict") => "设置已在别处修改，已重新载入；请重试。",
        (Locale::ZhCn, "assistant_settings") => "Assistant",
        (Locale::ZhCn, "assistant_enabled") => "启用 Assistant",
        (Locale::ZhCn, "enabled") => "启用",
        (Locale::ZhCn, "disabled") => "关闭",
        (Locale::ZhCn, "assistant_profiles") => "连接配置",
        (Locale::ZhCn, "assistant_new_profile") => "新建连接",
        (Locale::ZhCn, "assistant_connection_id") => "连接 ID",
        (Locale::ZhCn, "assistant_connection_name") => "连接名称",
        (Locale::ZhCn, "assistant_protocol") => "协议",
        (Locale::ZhCn, "assistant_protocol_notice") => "协议差异由运行时 adapter 处理",
        (Locale::ZhCn, "assistant_base_url") => "LiteLLM 地址",
        (Locale::ZhCn, "assistant_model") => "模型别名",
        (Locale::ZhCn, "assistant_auth_mode") => "认证方式",
        (Locale::ZhCn, "assistant_api_key") => "API Key",
        (Locale::ZhCn, "assistant_api_key_file") => "API Key 文件",
        (Locale::ZhCn, "assistant_context_window") => "上下文 token 上限",
        (Locale::ZhCn, "assistant_timeout") => "超时（毫秒）",
        (Locale::ZhCn, "assistant_max_output") => "最大输出 token",
        (Locale::ZhCn, "assistant_write_mode") => "写回模式",
        (Locale::ZhCn, "assistant_automatic") => "自动写回",
        (Locale::ZhCn, "assistant_confirm_each") => "逐次确认",
        (Locale::ZhCn, "assistant_max_rounds") => "最大工具轮次",
        (Locale::ZhCn, "assistant_max_retries") => "最大校验重试",
        (Locale::ZhCn, "assistant_connection_test") => "保存与检测",
        (Locale::ZhCn, "save") => "保存并设为当前连接",
        (Locale::ZhCn, "assistant_readiness") => "连接检测",
        (Locale::ZhCn, "assistant_model_probe") => "模型能力检测",
        (Locale::ZhCn, "assistant_settings_saved") => "Assistant 设置已保存。",
        (Locale::ZhCn, "assistant_connection_required") => "请先配置 Assistant 连接。",
        (Locale::ZhCn, "assistant_scene_ask") => "问答",
        (Locale::ZhCn, "assistant_scene_organize") => "整理",
        (Locale::ZhCn, "assistant_scene_partition") => "完整拆分",
        (Locale::ZhCn, "assistant_scene_extract") => "提取片段",
        (Locale::ZhCn, "assistant_scene_move") => "调整项目树",
        (Locale::ZhCn, "assistant_authorized_document") => "授权文档",
        (Locale::ZhCn, "assistant_current_document") => "仅当前文档",
        (Locale::ZhCn, "assistant_all_visible_documents") => "全部可见文档",
        (Locale::ZhCn, "assistant_no_document") => "未选择文档",
        (Locale::ZhCn, "assistant_projection") => "发送投影",
        (Locale::ZhCn, "assistant_connection") => "连接",
        (Locale::ZhCn, "assistant_local_connection") => "本机连接",
        (Locale::ZhCn, "assistant_remote_connection") => "远程连接",
        (Locale::ZhCn, "assistant_scope_notice") => "只发送当前明确选择的文档；不会发送音频、凭据、本机路径或其他文档。",
        (Locale::ZhCn, "assistant_all_visible_scope_notice") => "发送当前工作区内全部可见文档；不会发送音频、凭据或本机路径。",
        (Locale::ZhCn, "assistant_idle") => "等待任务",
        (Locale::ZhCn, "assistant_running") => "Assistant 正在运行",
        (Locale::ZhCn, "assistant_cancelling") => "正在取消",
        (Locale::ZhCn, "assistant_completed") => "Assistant 任务已完成",
        (Locale::ZhCn, "assistant_partial") => "Assistant 任务部分完成",
        (Locale::ZhCn, "assistant_cancelled") => "Assistant 任务已取消",
        (Locale::ZhCn, "assistant_awaiting_confirmation") => "等待确认写回",
        (Locale::ZhCn, "assistant_no_answer") => "本次任务没有文本回答。",
        (Locale::ZhCn, "assistant_evidence") => "证据 segment",
        (Locale::ZhCn, "assistant_no_evidence") => "无",
        (Locale::ZhCn, "assistant_model_route") => "请求/实际模型",
        (Locale::ZhCn, "assistant_usage") => "输入/输出/总 token",
        (Locale::ZhCn, "assistant_diagnostics") => "执行诊断",
        (Locale::ZhCn, "assistant_warning") => "警告",
        (Locale::ZhCn, "assistant_mutation") => "变更",
        (Locale::ZhCn, "assistant_affected") => "受影响文档",
        (Locale::ZhCn, "assistant_uncovered") => "未覆盖 segment",
        (Locale::ZhCn, "assistant_undo") => "撤销变更",
        (Locale::ZhCn, "assistant_undoing") => "正在撤销",
        (Locale::ZhCn, "assistant_undone") => "已撤销",
        (Locale::ZhCn, "assistant_pending") => "等待写回确认",
        (Locale::ZhCn, "assistant_pending_queue") => "排队中的确认",
        (Locale::ZhCn, "assistant_pending_overflow") => "超出队列上限、未列出的确认",
        (Locale::ZhCn, "assistant_field_diff") => "字段差异",
        (Locale::ZhCn, "assistant_tree_change") => "树结构变化",
        (Locale::ZhCn, "assistant_range") => "语义范围与证据",
        (Locale::ZhCn, "assistant_accept") => "接受并写回",
        (Locale::ZhCn, "assistant_reject") => "拒绝",
        (Locale::ZhCn, "assistant_rejected") => "已拒绝本次写回。",
        (Locale::ZhCn, "assistant_expired") => "确认已过期，请重新运行任务。",
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
        (Locale::EnUs, "downloaded") => "Downloaded",
        (Locale::EnUs, "not_downloaded") => "Not downloaded",
        (Locale::EnUs, "model_size") => "Weight size",
        (Locale::EnUs, "vram_estimate") => "VRAM estimate",
        (Locale::EnUs, "applicable_backends") => "Applicable backends",
        (Locale::EnUs, "no_models") => "No models are available in the catalog.",
        (Locale::EnUs, "appearance") => "Appearance",
        (Locale::EnUs, "theme") => "Theme",
        (Locale::EnUs, "theme_system") => "System",
        (Locale::EnUs, "theme_light") => "Light",
        (Locale::EnUs, "theme_dark") => "Dark",
        (Locale::EnUs, "material") => "Material",
        (Locale::EnUs, "material_glass") => "Glass",
        (Locale::EnUs, "material_opaque") => "Opaque",
        (Locale::EnUs, "interface") => "Interface",
        (Locale::EnUs, "language") => "Language / Interface locale",
        (Locale::EnUs, "transcription") => "Transcription",
        (Locale::EnUs, "asr_backend") => "ASR backend",
        (Locale::EnUs, "asr_model") => "ASR model",
        (Locale::EnUs, "transcription_language") => "Transcription language",
        (Locale::EnUs, "speaker_separation") => "Speaker separation",
        (Locale::EnUs, "diarization_backend") => "Diarization backend",
        (Locale::EnUs, "diarization_model") => "Diarization model",
        (Locale::EnUs, "diarization_segmentation_model") => "Segmentation model",
        (Locale::EnUs, "diarization_embedding_model") => "Embedding model",
        (Locale::EnUs, "diarization_clustering") => "Clustering algorithm",
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
        (Locale::EnUs, "speaker_library_path") => "Speaker library path",
        (Locale::EnUs, "choose_directory") => "Choose directory",
        (Locale::EnUs, "speaker_library_migration_title") => {
            "Migrate the existing speaker library?"
        }
        (Locale::EnUs, "speaker_library_migration_description") => {
            "An existing speaker library was found. Migration copies it to the new location and keeps the original file."
        }
        (Locale::EnUs, "migrate") => "Migrate and use",
        (Locale::EnUs, "use_new_path") => "Use new path only",
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
        (Locale::EnUs, "settings_saved") => "Settings saved.",
        (Locale::EnUs, "settings_conflict") => {
            "Settings changed elsewhere and were reloaded; try again."
        }
        (Locale::EnUs, "assistant_settings") => "Assistant",
        (Locale::EnUs, "assistant_enabled") => "Enable Assistant",
        (Locale::EnUs, "enabled") => "Enabled",
        (Locale::EnUs, "disabled") => "Disabled",
        (Locale::EnUs, "assistant_profiles") => "Connection profiles",
        (Locale::EnUs, "assistant_new_profile") => "New connection",
        (Locale::EnUs, "assistant_connection_id") => "Connection ID",
        (Locale::EnUs, "assistant_connection_name") => "Connection name",
        (Locale::EnUs, "assistant_protocol") => "Protocol",
        (Locale::EnUs, "assistant_protocol_notice") => "Protocol differences stay in runtime adapters",
        (Locale::EnUs, "assistant_base_url") => "LiteLLM URL",
        (Locale::EnUs, "assistant_model") => "Model alias",
        (Locale::EnUs, "assistant_auth_mode") => "Authentication",
        (Locale::EnUs, "assistant_api_key") => "API key",
        (Locale::EnUs, "assistant_api_key_file") => "API key file",
        (Locale::EnUs, "assistant_context_window") => "Context token limit",
        (Locale::EnUs, "assistant_timeout") => "Timeout (ms)",
        (Locale::EnUs, "assistant_max_output") => "Maximum output tokens",
        (Locale::EnUs, "assistant_write_mode") => "Write mode",
        (Locale::EnUs, "assistant_automatic") => "Automatic",
        (Locale::EnUs, "assistant_confirm_each") => "Confirm each",
        (Locale::EnUs, "assistant_max_rounds") => "Maximum tool rounds",
        (Locale::EnUs, "assistant_max_retries") => "Maximum validation retries",
        (Locale::EnUs, "assistant_connection_test") => "Save and probe",
        (Locale::EnUs, "save") => "Save and activate",
        (Locale::EnUs, "assistant_readiness") => "Test connection",
        (Locale::EnUs, "assistant_model_probe") => "Probe model capabilities",
        (Locale::EnUs, "assistant_settings_saved") => "Assistant settings saved.",
        (Locale::EnUs, "assistant_connection_required") => "Configure an Assistant connection first.",
        (Locale::EnUs, "assistant_scene_ask") => "Ask",
        (Locale::EnUs, "assistant_scene_organize") => "Organize",
        (Locale::EnUs, "assistant_scene_partition") => "Partition",
        (Locale::EnUs, "assistant_scene_extract") => "Extract",
        (Locale::EnUs, "assistant_scene_move") => "Move tree",
        (Locale::EnUs, "assistant_authorized_document") => "Authorized document",
        (Locale::EnUs, "assistant_current_document") => "Current document only",
        (Locale::EnUs, "assistant_all_visible_documents") => "All visible documents",
        (Locale::EnUs, "assistant_no_document") => "No document selected",
        (Locale::EnUs, "assistant_projection") => "Sent projection",
        (Locale::EnUs, "assistant_connection") => "Connection",
        (Locale::EnUs, "assistant_local_connection") => "Local connection",
        (Locale::EnUs, "assistant_remote_connection") => "Remote connection",
        (Locale::EnUs, "assistant_scope_notice") => "Only the explicitly selected document is sent; audio, credentials, local paths, and other documents stay local.",
        (Locale::EnUs, "assistant_all_visible_scope_notice") => "All documents visible in the current workspace are sent; audio, credentials, and local paths stay local.",
        (Locale::EnUs, "assistant_idle") => "Idle",
        (Locale::EnUs, "assistant_running") => "Assistant is running",
        (Locale::EnUs, "assistant_cancelling") => "Cancelling",
        (Locale::EnUs, "assistant_completed") => "Assistant task completed",
        (Locale::EnUs, "assistant_partial") => "Assistant task partially completed",
        (Locale::EnUs, "assistant_cancelled") => "Assistant task cancelled",
        (Locale::EnUs, "assistant_awaiting_confirmation") => "Awaiting write confirmation",
        (Locale::EnUs, "assistant_no_answer") => "This task returned no text answer.",
        (Locale::EnUs, "assistant_evidence") => "Evidence segments",
        (Locale::EnUs, "assistant_no_evidence") => "None",
        (Locale::EnUs, "assistant_model_route") => "Requested/resolved model",
        (Locale::EnUs, "assistant_usage") => "Input/output/total tokens",
        (Locale::EnUs, "assistant_diagnostics") => "Execution diagnostics",
        (Locale::EnUs, "assistant_warning") => "Warning",
        (Locale::EnUs, "assistant_mutation") => "Mutation",
        (Locale::EnUs, "assistant_affected") => "Affected documents",
        (Locale::EnUs, "assistant_uncovered") => "Uncovered segments",
        (Locale::EnUs, "assistant_undo") => "Undo mutation",
        (Locale::EnUs, "assistant_undoing") => "Undoing",
        (Locale::EnUs, "assistant_undone") => "Undone",
        (Locale::EnUs, "assistant_pending") => "Write confirmation pending",
        (Locale::EnUs, "assistant_pending_queue") => "Queued confirmations",
        (Locale::EnUs, "assistant_pending_overflow") => "Confirmations over the queue limit, not listed",
        (Locale::EnUs, "assistant_field_diff") => "Field differences",
        (Locale::EnUs, "assistant_tree_change") => "Tree changes",
        (Locale::EnUs, "assistant_range") => "Semantic ranges and evidence",
        (Locale::EnUs, "assistant_accept") => "Accept and write",
        (Locale::EnUs, "assistant_reject") => "Reject",
        (Locale::EnUs, "assistant_rejected") => "This write was rejected.",
        (Locale::EnUs, "assistant_expired") => "Confirmation expired; run the task again.",
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
        (Locale::ZhCn, "speaker_library_updated") => format!("声纹库地址已更新：{value}"),
        (Locale::EnUs, "speaker_library_updated") => {
            format!("Speaker library path updated: {value}")
        }
        (Locale::ZhCn, "speaker_library_migration_failed") => {
            format!("声纹库迁移失败：{value}")
        }
        (Locale::EnUs, "speaker_library_migration_failed") => {
            format!("Speaker library migration failed: {value}")
        }
        (Locale::ZhCn, "speaker_library_migration_title") => {
            "是否迁移现有声纹库？".into()
        }
        (Locale::EnUs, "speaker_library_migration_title") => {
            "Migrate the existing speaker library?".into()
        }
        (Locale::ZhCn, "speaker_library_migration_description") => {
            "检测到现有声纹库。迁移会将副本写入新位置，原文件会保留。".into()
        }
        (Locale::EnUs, "speaker_library_migration_description") => {
            "An existing speaker library was found. Migration copies it to the new location and keeps the original file.".into()
        }
        (Locale::ZhCn, "migrate") => "迁移并使用".into(),
        (Locale::EnUs, "migrate") => "Migrate and use".into(),
        (Locale::ZhCn, "use_new_path") => "仅使用新位置".into(),
        (Locale::EnUs, "use_new_path") => "Use new path only".into(),
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
        (Locale::ZhCn, "model_selected") => format!("已选择模型：{value}。"),
        (Locale::EnUs, "model_selected") => format!("Model selected: {value}."),
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
