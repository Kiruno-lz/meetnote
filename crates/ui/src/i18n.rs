use application::{
    AudioSourceError, AudioStatus, DocumentIssueKind, DocumentsRootError, ReservedLocation,
};
use contracts::ComputeDevice;
use domain::ProjectStatus;
use models::{
    BackendCheck, ChecksumResult, ModelFailure, ModelFailureRecord, ModelOperation, ProbeRecord,
    ProbeRecordError,
};

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
        (Locale::ZhCn, "assistant_prompt") => "助手请求",
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
        (Locale::ZhCn, "no_summary") => "还没有总结。可以使用助手整理这个项目。",
        (Locale::ZhCn, "need_process") => "请先处理项目以生成带 speaker 的转录。",
        (Locale::ZhCn, "empty_meetings") => "导入音频以创建项目。",
        (Locale::ZhCn, "project_detail") => "项目详情",
        (Locale::ZhCn, "close_detail") => "关闭详情",
        (Locale::ZhCn, "catalog_description") => "模型目录、安装状态和下载任务",
        (Locale::ZhCn, "download") => "下载",
        (Locale::ZhCn, "downloaded") => "已下载",
        (Locale::ZhCn, "not_downloaded") => "未下载",
        (Locale::ZhCn, "model_size") => "权重大小",
        (Locale::ZhCn, "vram_estimate") => "显存预估",
        (Locale::ZhCn, "applicable_backends") => "适用后端",
        (Locale::ZhCn, "no_models") => "模型目录中没有可用模型。",
        (Locale::ZhCn, "model_task") => "任务",
        (Locale::ZhCn, "task_asr") => "语音识别",
        (Locale::ZhCn, "task_diarization") => "说话人分离",
        (Locale::ZhCn, "catalog_status") => "目录状态",
        (Locale::ZhCn, "catalog_ready") => "可下载",
        (Locale::ZhCn, "catalog_candidate_source") => "候选来源，暂不可安装",
        (Locale::ZhCn, "catalog_requires_native_adapter") => "需要原生适配",
        (Locale::ZhCn, "model_license") => "许可",
        (Locale::ZhCn, "model_source") => "来源",
        (Locale::ZhCn, "model_checksum") => "校验和",
        (Locale::ZhCn, "show_model_details") => "展开详情",
        (Locale::ZhCn, "hide_model_details") => "收起详情",
        (Locale::ZhCn, "declared_checksums") => "目录声明的校验和",
        (Locale::ZhCn, "probe_backends") => "后端检查",
        (Locale::ZhCn, "probe_artifacts") => "文件校验",
        (Locale::ZhCn, "expected_checksum") => "应为",
        (Locale::ZhCn, "no_probe_record") => "尚无校验记录",
        (Locale::ZhCn, "probe_passed") => "通过",
        (Locale::ZhCn, "probe_failed") => "未通过",
        (Locale::ZhCn, "probe_kind_download") => "下载后校验",
        (Locale::ZhCn, "probe_kind_reprobe") => "重新校验",
        (Locale::ZhCn, "backend_not_registered") => "未检查：没有注册后端检查",
        (Locale::ZhCn, "checksum_matched") => "一致",
        (Locale::ZhCn, "checksum_archive_matched") => "归档一致",
        (Locale::ZhCn, "checksum_archive_not_rechecked") => "归档未保留，未重新校验",
        (Locale::ZhCn, "checksum_not_declared") => "未声明",
        (Locale::ZhCn, "reprobe") => "重新校验",
        (Locale::ZhCn, "reprobing") => "正在重新校验……",
        (Locale::ZhCn, "model_reprobe_progress") => "重新校验进度",
        (Locale::ZhCn, "retry_download") => "重试",
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
        (Locale::ZhCn, "speaker_library") => "声纹库",
        (Locale::ZhCn, "speaker_library_loading") => "正在读取声纹库……",
        (Locale::ZhCn, "speaker_library_empty") => "声纹库中还没有说话人档案。",
        (Locale::ZhCn, "speaker_profile_name_placeholder") => "档案名称",
        (Locale::ZhCn, "speakers") => "说话人",
        (Locale::ZhCn, "speakers_hint") => {
            "重命名只改变显示名称；合并把一位说话人的全部片段归给另一位。转录文字、时间和处理记录保持不变。"
        }
        (Locale::ZhCn, "no_speakers") => "这个文档没有说话人。",
        (Locale::ZhCn, "speaker_name_unchanged") => "名称没有变化。",
        (Locale::ZhCn, "rename") => "重命名",
        (Locale::ZhCn, "merge") => "合并",
        (Locale::ZhCn, "delete") => "删除",
        (Locale::ZhCn, "merge_into_placeholder") => "合并到……",
        (Locale::ZhCn, "merge_document_speaker_description") => {
            "被合并说话人的全部片段将归给目标说话人，并使用目标的名称。转录文字、时间和处理记录不变；正文仍是自动生成的转录时会随之更新。"
        }
        (Locale::ZhCn, "delete_speaker_profile_description") => {
            "档案会从声纹库文件中移除，之后的处理不再用它识别这位说话人。已处理文档中的说话人名称保持不变。"
        }
        (Locale::ZhCn, "merge_speaker_profiles_description") => {
            "两份档案的声纹样本合并到目标档案，被合并档案的名称保留为别名，该档案随后从声纹库移除。已处理文档中的说话人名称保持不变。"
        }
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
        (Locale::ZhCn, "assistant_pending_paused") => "任务在这项写回前暂停：接受才会写入，拒绝则丢弃这项变更。",
        (Locale::ZhCn, "assistant_accepting") => "正在写回……",
        (Locale::ZhCn, "assistant_rejecting") => "正在拒绝……",
        (Locale::ZhCn, "assistant_retry") => "重试",
        (Locale::ZhCn, "assistant_field_diff") => "字段差异",
        (Locale::ZhCn, "assistant_tree_change") => "树结构变化",
        (Locale::ZhCn, "assistant_range") => "语义范围与证据",
        (Locale::ZhCn, "assistant_accept") => "接受并写回",
        (Locale::ZhCn, "assistant_reject") => "拒绝",
        (Locale::ZhCn, "assistant_rejected") => "已拒绝本次写回。",
        (Locale::ZhCn, "assistant_expired") => "确认已过期，请重新运行任务。",
        (Locale::ZhCn, "assistant_deleted") => "已删除",
        (Locale::ZhCn, "assistant_tree_root") => "根层级",
        (Locale::ZhCn, "assistant_new_document") => "新文档",
        (Locale::ZhCn, "assistant_probe_readiness") => "连接测试",
        (Locale::ZhCn, "assistant_probe_model") => "模型能力探测",
        (Locale::ZhCn, "assistant_probe_running") => "正在检测连接……",
        (Locale::ZhCn, "assistant_probe_cancelled") => "检测已取消。",
        (Locale::ZhCn, "assistant_api_key_kept") => "留空则保留已保存的密钥；修改协议、地址、模型或认证方式时须重新输入。",
        (Locale::ZhCn, "assistant_credential_target_conflict") => "协议、地址、模型或认证方式已修改：请为新的连接目标重新输入 API Key 或密钥文件。本次没有保存。",
        (Locale::ZhCn, "assistant_write_automatic_notice") => "自动写回：变更立即写入文档，助手页为每项变更提供撤销。",
        (Locale::ZhCn, "assistant_write_confirm_each_notice") => "自动写回已关闭：Assistant 在每次写入前暂停，在助手页列出变更等你接受或拒绝。接受前不会写入文档，过期的确认不会写入。",
        (Locale::ZhCn, "assistant_write_mode_unsaved") => "尚未保存，点击保存后生效。",
        (Locale::ZhCn, "projection_metadata") => "元数据",
        (Locale::ZhCn, "projection_markdown") => "Markdown 正文",
        (Locale::ZhCn, "projection_transcript") => "转录文本",
        (Locale::ZhCn, "projection_speaker_segments") => "说话人片段",
        (Locale::ZhCn, "projection_complete") => "完整文档",
        (Locale::ZhCn, "capability_yes") => "支持",
        (Locale::ZhCn, "capability_no") => "不支持",
        (Locale::ZhCn, "capability_unknown") => "未知",
        (Locale::ZhCn, "select_placeholder") => "请选择",
        (Locale::ZhCn, "model_download_progress") => "模型下载进度",
        (Locale::ZhCn, "edit") => "编辑",
        (Locale::ZhCn, "document_title") => "标题",
        (Locale::ZhCn, "document_tags") => "标签",
        (Locale::ZhCn, "document_tags_placeholder") => "多个标签用逗号分隔",
        (Locale::ZhCn, "document_summary") => "总结",
        (Locale::ZhCn, "document_body") => "正文（Markdown）",
        (Locale::ZhCn, "document_status") => "文档状态",
        (Locale::ZhCn, "save_document") => "保存更改",
        (Locale::ZhCn, "document_loading") => "正在读取文档……",
        (Locale::ZhCn, "no_document_changes") => "没有需要保存的更改。",
        (Locale::ZhCn, "title_required") => "标题不能为空。",
        (Locale::ZhCn, "document_conflict_hint") => {
            "重新加载会读取磁盘上的最新版本，并丢弃该文档未保存的编辑。"
        }
        (Locale::ZhCn, "reload") => "重新加载",
        (Locale::ZhCn, "status_inbox") => "收件箱",
        (Locale::ZhCn, "status_in_progress") => "整理中",
        (Locale::ZhCn, "status_review") => "待复核",
        (Locale::ZhCn, "status_final") => "定稿",
        (Locale::ZhCn, "status_archived") => "归档",
        (Locale::ZhCn, "detail_identity") => "身份",
        (Locale::ZhCn, "detail_source") => "音频来源",
        (Locale::ZhCn, "detail_hierarchy") => "层级",
        (Locale::ZhCn, "detail_state") => "状态",
        (Locale::ZhCn, "detail_processing") => "处理记录",
        (Locale::ZhCn, "detail_statistics") => "结果统计",
        (Locale::ZhCn, "detail_transcript") => "转录",
        (Locale::ZhCn, "detail_title") => "标题",
        (Locale::ZhCn, "detail_id") => "ID",
        (Locale::ZhCn, "detail_audio_path") => "音频路径",
        (Locale::ZhCn, "detail_audio_range") => "音频范围",
        (Locale::ZhCn, "detail_parent") => "父文档",
        (Locale::ZhCn, "detail_root") => "无（根文档）",
        (Locale::ZhCn, "detail_depth") => "深度",
        (Locale::ZhCn, "detail_children") => "子文档",
        (Locale::ZhCn, "detail_document_status") => "文档状态",
        (Locale::ZhCn, "detail_revision") => "版本",
        (Locale::ZhCn, "detail_processing_record") => "处理记录",
        (Locale::ZhCn, "detail_not_processed") => "未处理",
        (Locale::ZhCn, "detail_asr") => "ASR",
        (Locale::ZhCn, "detail_diarization") => "说话人分离",
        (Locale::ZhCn, "detail_device") => "设备",
        (Locale::ZhCn, "detail_language") => "语言",
        (Locale::ZhCn, "detail_input_checksum") => "输入音频校验和",
        (Locale::ZhCn, "detail_last_error") => "最近错误",
        (Locale::ZhCn, "detail_transcript_segments") => "转录片段",
        (Locale::ZhCn, "detail_diarization_segments") => "分离片段",
        (Locale::ZhCn, "detail_speakers") => "说话人",
        (Locale::ZhCn, "detail_speech_duration") => "语音时长",
        (Locale::ZhCn, "detail_characters") => "字符",
        (Locale::ZhCn, "detail_words") => "词",
        (Locale::ZhCn, "detail_provenance_gaps") => "缺失的来源信息",
        (Locale::ZhCn, "detail_none") => "无",
        (Locale::ZhCn, "gap_no_processing_record") => "无处理记录",
        (Locale::ZhCn, "gap_completion_time") => "完成时间",
        (Locale::ZhCn, "gap_input_audio_checksum") => "输入音频校验和",
        (Locale::ZhCn, "gap_asr_model") => "ASR 模型",
        (Locale::ZhCn, "gap_asr_model_revision") => "ASR 模型版本",
        (Locale::ZhCn, "gap_asr_artifact_checksum") => "ASR 权重校验和",
        (Locale::ZhCn, "gap_diarization_backend") => "分离后端",
        (Locale::ZhCn, "gap_diarization_model_revision") => "分离模型版本",
        (Locale::ZhCn, "gap_clustering") => "聚类算法",
        (Locale::ZhCn, "gap_alignment") => "对齐算法",
        (Locale::ZhCn, "gap_runtime_device") => "运行设备",
        (Locale::ZhCn, "workspace") => "工作区",
        (Locale::ZhCn, "documents_root") => "文档目录",
        (Locale::ZhCn, "documents_root_switching") => "正在切换文档目录……",
        (Locale::ZhCn, "detail_audio_status") => "音频状态",
        (Locale::ZhCn, "audio_checking") => "正在检查……",
        (Locale::ZhCn, "audio_missing") => "音频缺失",
        (Locale::ZhCn, "audio_mismatch") => "音频不一致",
        (Locale::ZhCn, "relink_audio") => "重新关联音频",
        (Locale::ZhCn, "relinking_audio") => "正在重新关联音频……",
        (Locale::ZhCn, "audio_missing_cannot_process") => {
            "音频文件缺失；重新关联音频后才能处理。"
        }
        (Locale::EnUs, "assistant") => "Assistant",
        (Locale::EnUs, "meetings") => "Meetings",
        (Locale::EnUs, "models") => "Models",
        (Locale::EnUs, "settings") => "Settings",
        (Locale::EnUs, "local_intelligence") => "local voice library",
        (Locale::EnUs, "ask_library") => "Ask your voice library",
        (Locale::EnUs, "ask_placeholder") => "Ask about your recordings…",
        (Locale::EnUs, "assistant_prompt") => "Assistant request",
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
        (Locale::EnUs, "no_summary") => {
            "No summary yet. Use the Assistant to organize this project."
        }
        (Locale::EnUs, "need_process") => {
            "Run processing to generate a speaker-labelled transcript."
        }
        (Locale::EnUs, "empty_meetings") => "Import audio to create a project.",
        (Locale::EnUs, "project_detail") => "PROJECT DETAIL",
        (Locale::EnUs, "close_detail") => "Close detail",
        (Locale::EnUs, "catalog_description") => "Catalog, installation state, and download tasks",
        (Locale::EnUs, "download") => "Download",
        (Locale::EnUs, "downloaded") => "Downloaded",
        (Locale::EnUs, "not_downloaded") => "Not downloaded",
        (Locale::EnUs, "model_size") => "Weight size",
        (Locale::EnUs, "vram_estimate") => "VRAM estimate",
        (Locale::EnUs, "applicable_backends") => "Applicable backends",
        (Locale::EnUs, "no_models") => "No models are available in the catalog.",
        (Locale::EnUs, "model_task") => "Task",
        (Locale::EnUs, "task_asr") => "Speech recognition",
        (Locale::EnUs, "task_diarization") => "Speaker diarization",
        (Locale::EnUs, "catalog_status") => "Catalog status",
        (Locale::EnUs, "catalog_ready") => "Downloadable",
        (Locale::EnUs, "catalog_candidate_source") => "Candidate source, not installable",
        (Locale::EnUs, "catalog_requires_native_adapter") => "Needs a native adapter",
        (Locale::EnUs, "model_license") => "License",
        (Locale::EnUs, "model_source") => "Source",
        (Locale::EnUs, "model_checksum") => "Checksum",
        (Locale::EnUs, "show_model_details") => "Show details",
        (Locale::EnUs, "hide_model_details") => "Hide details",
        (Locale::EnUs, "declared_checksums") => "Checksums in the catalog",
        (Locale::EnUs, "probe_backends") => "Backend checks",
        (Locale::EnUs, "probe_artifacts") => "Artifact checksums",
        (Locale::EnUs, "expected_checksum") => "expected",
        (Locale::EnUs, "no_probe_record") => "No probe record yet",
        (Locale::EnUs, "probe_passed") => "passed",
        (Locale::EnUs, "probe_failed") => "failed",
        (Locale::EnUs, "probe_kind_download") => "after download",
        (Locale::EnUs, "probe_kind_reprobe") => "re-probe",
        (Locale::EnUs, "backend_not_registered") => "not checked: no backend probe registered",
        (Locale::EnUs, "checksum_matched") => "matches",
        (Locale::EnUs, "checksum_archive_matched") => "archive matches",
        (Locale::EnUs, "checksum_archive_not_rechecked") => "archive not kept, not rechecked",
        (Locale::EnUs, "checksum_not_declared") => "not declared",
        (Locale::EnUs, "reprobe") => "Re-probe",
        (Locale::EnUs, "reprobing") => "Re-probing…",
        (Locale::EnUs, "model_reprobe_progress") => "Re-probe progress",
        (Locale::EnUs, "retry_download") => "Retry",
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
        (Locale::EnUs, "speaker_library") => "Speaker Library",
        (Locale::EnUs, "speaker_library_loading") => "Reading the speaker library…",
        (Locale::EnUs, "speaker_library_empty") => "The speaker library has no profiles yet.",
        (Locale::EnUs, "speaker_profile_name_placeholder") => "Profile name",
        (Locale::EnUs, "speakers") => "Speakers",
        (Locale::EnUs, "speakers_hint") => {
            "Renaming changes only the displayed name; merging gives every segment of one speaker to another. Transcript text, timings and the processing record stay as they are."
        }
        (Locale::EnUs, "no_speakers") => "This document has no speakers.",
        (Locale::EnUs, "speaker_name_unchanged") => "The name did not change.",
        (Locale::EnUs, "rename") => "Rename",
        (Locale::EnUs, "merge") => "Merge",
        (Locale::EnUs, "delete") => "Delete",
        (Locale::EnUs, "merge_into_placeholder") => "Merge into…",
        (Locale::EnUs, "merge_document_speaker_description") => {
            "Every segment of the merged speaker goes to the target speaker and takes its name. Transcript text, timings and the processing record do not change; a body that is still the generated transcript is regenerated."
        }
        (Locale::EnUs, "delete_speaker_profile_description") => {
            "The profile is removed from the library file, and later processing no longer recognizes this speaker by it. Speaker names in processed documents stay as they are."
        }
        (Locale::EnUs, "merge_speaker_profiles_description") => {
            "The voice samples of both profiles join the target profile, the merged profile's name is kept as an alias, and the merged profile leaves the library. Speaker names in processed documents stay as they are."
        }
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
        (Locale::EnUs, "assistant_pending_paused") => "The run paused before this write: accepting writes it, rejecting discards it.",
        (Locale::EnUs, "assistant_accepting") => "Writing…",
        (Locale::EnUs, "assistant_rejecting") => "Rejecting…",
        (Locale::EnUs, "assistant_retry") => "Retry",
        (Locale::EnUs, "assistant_field_diff") => "Field differences",
        (Locale::EnUs, "assistant_tree_change") => "Tree changes",
        (Locale::EnUs, "assistant_range") => "Semantic ranges and evidence",
        (Locale::EnUs, "assistant_accept") => "Accept and write",
        (Locale::EnUs, "assistant_reject") => "Reject",
        (Locale::EnUs, "assistant_rejected") => "This write was rejected.",
        (Locale::EnUs, "assistant_expired") => "Confirmation expired; run the task again.",
        (Locale::EnUs, "assistant_deleted") => "deleted",
        (Locale::EnUs, "assistant_tree_root") => "top level",
        (Locale::EnUs, "assistant_new_document") => "new document",
        (Locale::EnUs, "assistant_probe_readiness") => "connection test",
        (Locale::EnUs, "assistant_probe_model") => "model probe",
        (Locale::EnUs, "assistant_probe_running") => "Checking the connection…",
        (Locale::EnUs, "assistant_probe_cancelled") => "The check was cancelled.",
        (Locale::EnUs, "assistant_api_key_kept") => "Leave empty to keep the stored key; changing the protocol, URL, model or authentication requires entering it again.",
        (Locale::EnUs, "assistant_credential_target_conflict") => "The protocol, URL, model or authentication changed: enter the API key or key file again for the new target. Nothing was saved.",
        (Locale::EnUs, "assistant_write_automatic_notice") => "Automatic write-back: changes are written at once, and the Assistant page offers an undo for each.",
        (Locale::EnUs, "assistant_write_confirm_each_notice") => "Automatic write-back is off: the Assistant pauses before every write and lists it on the Assistant page for you to accept or reject. Nothing is written until you accept, and an expired confirmation is never written.",
        (Locale::EnUs, "assistant_write_mode_unsaved") => "Not saved yet; takes effect after Save.",
        (Locale::EnUs, "projection_metadata") => "Metadata",
        (Locale::EnUs, "projection_markdown") => "Markdown body",
        (Locale::EnUs, "projection_transcript") => "Transcript",
        (Locale::EnUs, "projection_speaker_segments") => "Speaker segments",
        (Locale::EnUs, "projection_complete") => "Complete document",
        (Locale::EnUs, "capability_yes") => "yes",
        (Locale::EnUs, "capability_no") => "no",
        (Locale::EnUs, "capability_unknown") => "unknown",
        (Locale::EnUs, "select_placeholder") => "Select…",
        (Locale::EnUs, "model_download_progress") => "Model download progress",
        (Locale::EnUs, "edit") => "Edit",
        (Locale::EnUs, "document_title") => "Title",
        (Locale::EnUs, "document_tags") => "Tags",
        (Locale::EnUs, "document_tags_placeholder") => "Separate tags with commas",
        (Locale::EnUs, "document_summary") => "Summary",
        (Locale::EnUs, "document_body") => "Body (Markdown)",
        (Locale::EnUs, "document_status") => "Document status",
        (Locale::EnUs, "save_document") => "Save changes",
        (Locale::EnUs, "document_loading") => "Reading the document…",
        (Locale::EnUs, "no_document_changes") => "Nothing to save.",
        (Locale::EnUs, "title_required") => "The title cannot be empty.",
        (Locale::EnUs, "document_conflict_hint") => {
            "Reload reads the latest version from disk and discards this document's unsaved edits."
        }
        (Locale::EnUs, "reload") => "Reload",
        (Locale::EnUs, "status_inbox") => "Inbox",
        (Locale::EnUs, "status_in_progress") => "In progress",
        (Locale::EnUs, "status_review") => "Review",
        (Locale::EnUs, "status_final") => "Final",
        (Locale::EnUs, "status_archived") => "Archived",
        (Locale::EnUs, "detail_identity") => "Identity",
        (Locale::EnUs, "detail_source") => "Audio source",
        (Locale::EnUs, "detail_hierarchy") => "Hierarchy",
        (Locale::EnUs, "detail_state") => "State",
        (Locale::EnUs, "detail_processing") => "Processing",
        (Locale::EnUs, "detail_statistics") => "Result statistics",
        (Locale::EnUs, "detail_transcript") => "Transcript",
        (Locale::EnUs, "detail_title") => "Title",
        (Locale::EnUs, "detail_id") => "ID",
        (Locale::EnUs, "detail_audio_path") => "Audio path",
        (Locale::EnUs, "detail_audio_range") => "Audio range",
        (Locale::EnUs, "detail_parent") => "Parent",
        (Locale::EnUs, "detail_root") => "None (root document)",
        (Locale::EnUs, "detail_depth") => "Depth",
        (Locale::EnUs, "detail_children") => "Children",
        (Locale::EnUs, "detail_document_status") => "Document status",
        (Locale::EnUs, "detail_revision") => "Revision",
        (Locale::EnUs, "detail_processing_record") => "Processing record",
        (Locale::EnUs, "detail_not_processed") => "Not processed",
        (Locale::EnUs, "detail_asr") => "ASR",
        (Locale::EnUs, "detail_diarization") => "Diarization",
        (Locale::EnUs, "detail_device") => "Device",
        (Locale::EnUs, "detail_language") => "Language",
        (Locale::EnUs, "detail_input_checksum") => "Input audio checksum",
        (Locale::EnUs, "detail_last_error") => "Last error",
        (Locale::EnUs, "detail_transcript_segments") => "Transcript segments",
        (Locale::EnUs, "detail_diarization_segments") => "Diarization turns",
        (Locale::EnUs, "detail_speakers") => "Speakers",
        (Locale::EnUs, "detail_speech_duration") => "Speech duration",
        (Locale::EnUs, "detail_characters") => "Characters",
        (Locale::EnUs, "detail_words") => "Words",
        (Locale::EnUs, "detail_provenance_gaps") => "Missing provenance",
        (Locale::EnUs, "detail_none") => "None",
        (Locale::EnUs, "gap_no_processing_record") => "No processing record",
        (Locale::EnUs, "gap_completion_time") => "Completion time",
        (Locale::EnUs, "gap_input_audio_checksum") => "Input audio checksum",
        (Locale::EnUs, "gap_asr_model") => "ASR model",
        (Locale::EnUs, "gap_asr_model_revision") => "ASR model revision",
        (Locale::EnUs, "gap_asr_artifact_checksum") => "ASR artifact checksum",
        (Locale::EnUs, "gap_diarization_backend") => "Diarization backend",
        (Locale::EnUs, "gap_diarization_model_revision") => "Diarization model revision",
        (Locale::EnUs, "gap_clustering") => "Clustering algorithm",
        (Locale::EnUs, "gap_alignment") => "Alignment algorithm",
        (Locale::EnUs, "gap_runtime_device") => "Runtime device",
        (Locale::EnUs, "workspace") => "Workspace",
        (Locale::EnUs, "documents_root") => "Documents folder",
        (Locale::EnUs, "documents_root_switching") => "Switching the documents folder…",
        (Locale::EnUs, "detail_audio_status") => "Audio status",
        (Locale::EnUs, "audio_checking") => "Checking…",
        (Locale::EnUs, "audio_missing") => "Audio missing",
        (Locale::EnUs, "audio_mismatch") => "Audio differs",
        (Locale::EnUs, "relink_audio") => "Relink audio",
        (Locale::EnUs, "relinking_audio") => "Relinking the audio…",
        (Locale::EnUs, "audio_missing_cannot_process") => {
            "The audio file is missing; relink the audio before processing."
        }
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
        (Locale::ZhCn, "reprobing_model") => format!("正在重新校验 {value}……"),
        (Locale::EnUs, "reprobing_model") => format!("Re-probing {value}…"),
        (Locale::ZhCn, "reprobed") => format!("已重新校验 {value}"),
        (Locale::EnUs, "reprobed") => format!("Re-probed {value}"),
        (Locale::ZhCn, "reprobe_failed") => format!("重新校验失败：{value}"),
        (Locale::EnUs, "reprobe_failed") => format!("Re-probe failed: {value}"),
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
        (Locale::ZhCn, "speaker_renamed") => format!("已将说话人重命名为「{value}」。"),
        (Locale::EnUs, "speaker_renamed") => format!("Renamed the speaker to “{value}”."),
        (Locale::ZhCn, "speaker_name_for") => format!("{value} 的名称"),
        (Locale::EnUs, "speaker_name_for") => format!("Name of {value}"),
        (Locale::ZhCn, "merge_speaker_for") => format!("把 {value} 合并到"),
        (Locale::EnUs, "merge_speaker_for") => format!("Merge {value} into"),
        (Locale::ZhCn, "speaker_segments") => format!("{value} 段"),
        (Locale::EnUs, "speaker_segments") => format!("Segments: {value}"),
        (Locale::ZhCn, "speaker_profile_renamed") => format!("已将档案重命名为「{value}」。"),
        (Locale::EnUs, "speaker_profile_renamed") => {
            format!("Renamed the profile to “{value}”.")
        }
        (Locale::ZhCn, "speaker_profile_deleted") => format!("已删除档案「{value}」。"),
        (Locale::EnUs, "speaker_profile_deleted") => format!("Deleted the profile “{value}”."),
        (Locale::ZhCn, "speaker_profile_name_for") => format!("档案 {value} 的名称"),
        (Locale::EnUs, "speaker_profile_name_for") => format!("Name of the profile {value}"),
        (Locale::ZhCn, "merge_speaker_profile_for") => format!("把档案 {value} 合并到"),
        (Locale::EnUs, "merge_speaker_profile_for") => format!("Merge the profile {value} into"),
        (Locale::ZhCn, "speaker_profile_samples") => format!("{value} 个样本"),
        (Locale::EnUs, "speaker_profile_samples") => format!("Samples: {value}"),
        (Locale::ZhCn, "speaker_profile_aliases") => format!("别名：{value}"),
        (Locale::EnUs, "speaker_profile_aliases") => format!("Also: {value}"),
        (Locale::ZhCn, "speaker_library_failed") => format!("声纹库没有更改：{value}"),
        (Locale::EnUs, "speaker_library_failed") => {
            format!("The speaker library was not changed: {value}")
        }
        (Locale::ZhCn, "speaker_library_read_failed") => format!("无法读取声纹库：{value}"),
        (Locale::EnUs, "speaker_library_read_failed") => {
            format!("Cannot read the speaker library: {value}")
        }
        (Locale::ZhCn, "delete_speaker_profile_title") => format!("删除档案「{value}」？"),
        (Locale::EnUs, "delete_speaker_profile_title") => {
            format!("Delete the profile “{value}”?")
        }
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
        (Locale::ZhCn, "document_saved") => format!("已保存「{value}」。"),
        (Locale::EnUs, "document_saved") => format!("Saved “{value}”."),
        (Locale::ZhCn, "document_read_failed") => format!("无法读取文档：{value}"),
        (Locale::EnUs, "document_read_failed") => format!("Cannot read the document: {value}"),
        (Locale::ZhCn, "document_save_failed") => format!("保存失败：{value}"),
        (Locale::EnUs, "document_save_failed") => format!("Save failed: {value}"),
        (Locale::ZhCn, "document_conflict") => {
            format!("「{value}」已在别处修改，本次更改没有写入。")
        }
        (Locale::EnUs, "document_conflict") => {
            format!("“{value}” changed elsewhere; this change was not written.")
        }
        (Locale::ZhCn, "document_reloaded") => format!("已重新加载「{value}」。"),
        (Locale::EnUs, "document_reloaded") => format!("Reloaded “{value}”."),
        (Locale::ZhCn, "document_issues") => format!("文档目录中有 {value} 项无法载入"),
        (Locale::EnUs, "document_issues") => {
            format!("{value} entries in the documents folder could not be loaded")
        }
        (Locale::ZhCn, "documents_root_switched") => format!("文档目录已切换到 {value}"),
        (Locale::EnUs, "documents_root_switched") => {
            format!("Switched the documents folder to {value}")
        }
        (Locale::ZhCn, "documents_root_not_saved") => {
            format!("文档目录保持不变：设置保存失败：{value}")
        }
        (Locale::EnUs, "documents_root_not_saved") => {
            format!("The documents folder stays unchanged: saving the setting failed: {value}")
        }
        (Locale::ZhCn, "documents_root_restore_failed") => {
            format!("设置保存失败，且无法切回原文档目录：{value}")
        }
        (Locale::EnUs, "documents_root_restore_failed") => format!(
            "Saving the setting failed and the documents folder could not be switched back: {value}"
        ),
        (Locale::ZhCn, "audio_relinked") => format!("已重新关联「{value}」的音频。"),
        (Locale::EnUs, "audio_relinked") => format!("Relinked the audio of “{value}”."),
        (Locale::ZhCn, "assistant_pending_position") => {
            let (position, total) = value.split_once('|').unwrap_or((value, value));
            format!("第 {position} / {total} 项")
        }
        (Locale::EnUs, "assistant_pending_position") => {
            let (position, total) = value.split_once('|').unwrap_or((value, value));
            format!("Review {position} of {total}")
        }
        (Locale::ZhCn, "assistant_receipts") => format!("已写入 {value} 项变更，可逐项撤销"),
        (Locale::EnUs, "assistant_receipts") => {
            format!("{value} writes applied; each can be undone")
        }
        (Locale::ZhCn, "assistant_pending_expires") => format!("{value} 过期"),
        (Locale::EnUs, "assistant_pending_expires") => format!("Expires {value}"),
        (Locale::ZhCn, "assistant_pending_overflow") => format!(
            "另有 {value} 项确认超出队列上限，未列出，也不会写入；处理完当前队列后请重新运行任务。"
        ),
        (Locale::EnUs, "assistant_pending_overflow") => format!(
            "{value} more confirmations exceed the queue limit; they are not listed and will not be written. Run the task again after this queue."
        ),
        (Locale::ZhCn, "assistant_probe_failed") => format!("检测失败：{value}"),
        (Locale::EnUs, "assistant_probe_failed") => format!("Check failed: {value}"),
        (Locale::ZhCn, "documents_load_failed") => format!("无法读取文档目录：{value}"),
        (Locale::EnUs, "documents_load_failed") => {
            format!("Cannot read the documents folder: {value}")
        }
        _ => value.to_string(),
    }
}

/// A message naming two things, such as the speaker merged and the one it
/// is merged into.
pub fn message_pair(locale: Locale, key: &str, first: &str, second: &str) -> String {
    match (locale, key) {
        (Locale::ZhCn, "merge_document_speaker_title") => {
            format!("将「{first}」合并到「{second}」？")
        }
        (Locale::EnUs, "merge_document_speaker_title") => {
            format!("Merge “{first}” into “{second}”?")
        }
        (Locale::ZhCn, "speakers_merged") => format!("已将「{first}」合并到「{second}」。"),
        (Locale::EnUs, "speakers_merged") => format!("Merged “{first}” into “{second}”."),
        (Locale::ZhCn, "merge_speaker_profiles_title") => {
            format!("将档案「{first}」合并到「{second}」？")
        }
        (Locale::EnUs, "merge_speaker_profiles_title") => {
            format!("Merge the profile “{first}” into “{second}”?")
        }
        (Locale::ZhCn, "speaker_profiles_merged") => {
            format!("已将档案「{first}」合并到「{second}」。")
        }
        (Locale::EnUs, "speaker_profiles_merged") => {
            format!("Merged the profile “{first}” into “{second}”.")
        }
        _ => format!("{first} → {second}"),
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

/// The distinct SHA-256 values a catalog entry declares, in short form: the
/// first in full detail is one expansion away on the card.
pub fn declared_checksums(locale: Locale, checksums: &[&str]) -> String {
    match (locale, checksums) {
        (_, []) => text(locale, "checksum_not_declared").into(),
        (_, [checksum]) => format!("{}…", short_checksum(checksum)),
        (Locale::ZhCn, [first, ..]) => {
            format!("{}… 等 {} 个", short_checksum(first), checksums.len())
        }
        (Locale::EnUs, [first, rest @ ..]) => {
            format!("{}… and {} more", short_checksum(first), rest.len())
        }
    }
}

/// The last failed download or re-probe: the operation, when it failed and
/// why.
pub fn model_failure(locale: Locale, record: &ModelFailureRecord) -> String {
    let time = crate::components::utc_time(record.failed_at_ms);
    let reason = model_failure_reason(locale, &record.reason);
    match (locale, record.operation) {
        (Locale::ZhCn, ModelOperation::Download) => format!("下载失败（{time}）：{reason}"),
        (Locale::ZhCn, ModelOperation::Reprobe) => format!("重新校验失败（{time}）：{reason}"),
        (Locale::EnUs, ModelOperation::Download) => format!("Download failed ({time}): {reason}"),
        (Locale::EnUs, ModelOperation::Reprobe) => format!("Re-probe failed ({time}): {reason}"),
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
        (Locale::ZhCn, BackendCheck::Failed { message }) => format!("未通过：{message}"),
        (Locale::EnUs, BackendCheck::Failed { message }) => format!("failed: {message}"),
    }
}

/// The outcome, kind and time of a model's last probe.
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
        Locale::ZhCn => format!("最近校验：{outcome} · {kind} · {time}"),
        Locale::EnUs => format!("Last probe: {outcome} · {kind} · {time}"),
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
            BackendCheck, ChecksumResult, ModelFailure, ModelFailureRecord, ModelOperation,
            ProbeOutcome, ProbeRecord, ProbeRecordError,
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

        // The failed operation and its time head the reason.
        let (reason, _) = &failures[6];
        let record = |operation| ModelFailureRecord {
            operation,
            failed_at_ms: 1_788_000_000_000,
            revision: None,
            reason: reason.clone(),
        };
        let [download, reprobe] = [ModelOperation::Download, ModelOperation::Reprobe]
            .map(|operation| model_failure(Locale::ZhCn, &record(operation)));
        assert_ne!(download, reprobe, "the operation is named");
        for sentence in [&download, &reprobe] {
            assert!(sentence.contains("2026-08-29 10:40 UTC"), "{sentence}");
            assert!(
                sentence.ends_with(&model_failure_reason(Locale::ZhCn, reason)),
                "{sentence}"
            );
        }
        assert_localized(
            &download,
            &model_failure(Locale::EnUs, &record(ModelOperation::Download)),
        );

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
