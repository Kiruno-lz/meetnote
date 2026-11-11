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
        (Locale::ZhCn, "recordings_empty_title") => "还没有录音",
        (Locale::EnUs, "recordings_empty_title") => "No recordings",
        (Locale::ZhCn, "recordings_empty_hint") => "导入音频以创建文档。",
        (Locale::EnUs, "recordings_empty_hint") => "Import an audio file to create a document.",
        (Locale::ZhCn, "recording_imported") => "已导入",
        (Locale::EnUs, "recording_imported") => "Imported",
        (Locale::ZhCn, "transcript_heading") => "转写原文",
        (Locale::EnUs, "transcript_heading") => "Original transcript",
        (Locale::ZhCn, "transcript_empty") => "此文档没有转写片段。",
        (Locale::EnUs, "transcript_empty") => "No transcript segments in this document.",
        (Locale::ZhCn, "search_match") => "搜索命中",
        (Locale::EnUs, "search_match") => "Search match",
        (Locale::ZhCn, "saved_notes_heading") => "已保存的笔记",
        (Locale::EnUs, "saved_notes_heading") => "Saved notes",
        (Locale::ZhCn, "saved_notes_empty") => "此文档没有已保存的笔记。",
        (Locale::EnUs, "saved_notes_empty") => "This document has no saved notes.",
        (Locale::ZhCn, "edit_document_heading") => "编辑文档",
        (Locale::EnUs, "edit_document_heading") => "Edit document",
        (Locale::ZhCn, "document_details") => "文档详情",
        (Locale::EnUs, "document_details") => "Details",
        (Locale::ZhCn, "source_audio") => "音频源文件",
        (Locale::EnUs, "source_audio") => "SOURCE AUDIO",
        (Locale::ZhCn, "audio_duration") => "时长",
        (Locale::EnUs, "audio_duration") => "DURATION",
        (Locale::ZhCn, "document_id") => "文档 ID",
        (Locale::EnUs, "document_id") => "DOCUMENT ID",
        (Locale::ZhCn, "document_revision") => "修订版本",
        (Locale::EnUs, "document_revision") => "REVISION",
        (Locale::ZhCn, "document_parent") => "上级项目",
        (Locale::EnUs, "document_parent") => "PARENT",
        (Locale::ZhCn, "no_processing_record") => "没有处理记录",
        (Locale::EnUs, "no_processing_record") => "No processing record",
        (Locale::ZhCn, "no_speaker_data") => "没有说话人数据",
        (Locale::EnUs, "no_speaker_data") => "No speaker data",
        (Locale::ZhCn, "recheck_audio_source") => "重新检查音频源文件",
        (Locale::EnUs, "recheck_audio_source") => "Check source again",
        (Locale::ZhCn, "audio_source_verified") => "已验证音频源文件",
        (Locale::EnUs, "audio_source_verified") => "Source audio verified",
        (Locale::ZhCn, "audio_source_available") => "音频源文件可用",
        (Locale::EnUs, "audio_source_available") => "Source audio available",
        (Locale::ZhCn, "audio_source_mismatch") => "音频源文件与处理时使用的文件不同",
        (Locale::EnUs, "audio_source_mismatch") => "Source audio differs from the processed recording",
        (Locale::ZhCn, "cancel_processing") => "取消处理",
        (Locale::EnUs, "cancel_processing") => "Cancel processing",
        (Locale::ZhCn, "documents_workspace") => "项目 / 工作区",
        (Locale::EnUs, "documents_workspace") => "DOCUMENTS  /  WORKSPACE",
        (Locale::ZhCn, "settings_workspace") => "设置 / 工作区",
        (Locale::EnUs, "settings_workspace") => "SETTINGS  /  WORKSPACE",
        (Locale::ZhCn, "assistant_description") => "就工作区文档提问，或审核助手提出的修改。",
        (Locale::EnUs, "assistant_description") => "Ask questions or review changes to workspace documents.",
        (Locale::ZhCn, "assistant_review_changes") => "审核拟议变更",
        (Locale::EnUs, "assistant_review_changes") => "Review proposed changes",
        (Locale::ZhCn, "assistant_empty_value") => "空",
        (Locale::EnUs, "assistant_empty_value") => "Empty",
        (Locale::ZhCn, "assistant_uncovered_segments") => "未覆盖的片段",
        (Locale::EnUs, "assistant_uncovered_segments") => "Uncovered segments",
        (Locale::ZhCn, "assistant_waiting") => "正在等待助手响应…",
        (Locale::EnUs, "assistant_waiting") => "Waiting for Assistant…",
        (Locale::ZhCn, "assistant_prompt_placeholder") => "询问选中的文档",
        (Locale::EnUs, "assistant_prompt_placeholder") => "Ask about the selected documents",
        (Locale::ZhCn, "assistant_connection_description") => "配置助手使用的服务地址。",
        (Locale::EnUs, "assistant_connection_description") => "Configure the endpoint used by Assistant.",
        (Locale::ZhCn, "assistant_endpoint") => "服务地址",
        (Locale::EnUs, "assistant_endpoint") => "Endpoint",
        (Locale::ZhCn, "assistant_credential") => "凭据",
        (Locale::EnUs, "assistant_credential") => "Credential",
        (Locale::ZhCn, "assistant_keep_credential") => "保留已保存的凭据",
        (Locale::EnUs, "assistant_keep_credential") => "Keep saved credential",
        (Locale::ZhCn, "assistant_enter_key") => "输入 API Key",
        (Locale::EnUs, "assistant_enter_key") => "Enter API key",
        (Locale::ZhCn, "assistant_use_key_file") => "使用密钥文件",
        (Locale::EnUs, "assistant_use_key_file") => "Use key file",
        (Locale::ZhCn, "assistant_clear_credential") => "清除凭据",
        (Locale::EnUs, "assistant_clear_credential") => "Clear credential",
        (Locale::ZhCn, "assistant_key_file_path") => "相对于设置文件的密钥路径",
        (Locale::EnUs, "assistant_key_file_path") => "Key file relative to settings",
        (Locale::ZhCn, "assistant_reload_connection") => "重新加载连接",
        (Locale::EnUs, "assistant_reload_connection") => "Reload connection",
        (Locale::ZhCn, "assistant_cancel_probe") => "取消检测",
        (Locale::EnUs, "assistant_cancel_probe") => "Cancel probe",
        (Locale::ZhCn, "model_refresh") => "刷新",
        (Locale::EnUs, "model_refresh") => "Refresh",
        (Locale::ZhCn, "model_queued") => "排队中",
        (Locale::EnUs, "model_queued") => "Queued",
        (Locale::ZhCn, "model_paused") => "已暂停",
        (Locale::EnUs, "model_paused") => "Paused",
        (Locale::ZhCn, "model_install_complete") => "已安装",
        (Locale::EnUs, "model_install_complete") => "Installed",
        (Locale::ZhCn, "model_install_partial") => "部分已安装",
        (Locale::EnUs, "model_install_partial") => "Partially installed",
        (Locale::ZhCn, "model_requires_auth") => "需要认证",
        (Locale::EnUs, "model_requires_auth") => "Requires authentication",
        (Locale::ZhCn, "model_download_available") => "可下载",
        (Locale::EnUs, "model_download_available") => "Available",
        (Locale::ZhCn, "model_catalog_candidate") => "候选模型",
        (Locale::EnUs, "model_catalog_candidate") => "Candidate",
        (Locale::ZhCn, "model_catalog_loading") => "正在读取模型目录…",
        (Locale::EnUs, "model_catalog_loading") => "Reading the model catalog…",
        (Locale::ZhCn, "model_probe_passed") => "探测通过",
        (Locale::EnUs, "model_probe_passed") => "Probe passed",
        (Locale::ZhCn, "model_probe_failed") => "探测失败",
        (Locale::EnUs, "model_probe_failed") => "Probe failed",
        (Locale::ZhCn, "model_backend_passed") => "通过",
        (Locale::EnUs, "model_backend_passed") => "Passed",
        (Locale::ZhCn, "model_backend_unavailable") => "后端不可用",
        (Locale::EnUs, "model_backend_unavailable") => "Backend unavailable",
        (Locale::ZhCn, "model_backend_checksum_skipped") => "校验失败，已跳过",
        (Locale::EnUs, "model_backend_checksum_skipped") => "Skipped after checksum failure",
        (Locale::ZhCn, "model_details_actions") => "详情与操作",
        (Locale::EnUs, "model_details_actions") => "Details and actions",
        (Locale::ZhCn, "model_checksum_mismatch") => "校验和不匹配",
        (Locale::EnUs, "model_checksum_mismatch") => "Checksum mismatch",
        (Locale::ZhCn, "model_checksum_unreadable") => "无法读取文件",
        (Locale::EnUs, "model_checksum_unreadable") => "File unreadable",
        (Locale::ZhCn, "audio_processing") => "音频处理",
        (Locale::EnUs, "audio_processing") => "Audio processing",
        (Locale::ZhCn, "assistant_connection") => "助手连接",
        (Locale::EnUs, "assistant_connection") => "Assistant connection",
        (Locale::ZhCn, "save_settings") => "保存设置",
        (Locale::EnUs, "save_settings") => "Save settings",
        (Locale::ZhCn, "reload_settings") => "重新加载设置",
        (Locale::EnUs, "reload_settings") => "Reload settings",
        (Locale::ZhCn, "appearance_description") => "自定义 MeetNote 窗口。",
        (Locale::EnUs, "appearance_description") => "Customize the MeetNote window.",
        (Locale::ZhCn, "audio_processing_description") => "配置新音频处理任务使用的选项。",
        (Locale::EnUs, "audio_processing_description") => "Set the options used by new processing jobs.",
        (Locale::ZhCn, "open_models") => "打开模型管理",
        (Locale::EnUs, "open_models") => "Open Models",
        (Locale::ZhCn, "model_default") => "使用模型默认值",
        (Locale::EnUs, "model_default") => "Model default",
        (Locale::ZhCn, "speaker_library_manage") => "管理已保存的声纹档案。",
        (Locale::EnUs, "speaker_library_manage") => "Manage saved voice profiles.",
        (Locale::ZhCn, "speaker_library_renamed") => "已重命名声纹档案",
        (Locale::EnUs, "speaker_library_renamed") => "Speaker profile renamed",
        (Locale::ZhCn, "speaker_library_merged") => "已合并声纹档案",
        (Locale::EnUs, "speaker_library_merged") => "Speaker profiles merged",
        (Locale::ZhCn, "speaker_library_deleted") => "已删除声纹档案",
        (Locale::EnUs, "speaker_library_deleted") => "Speaker profile deleted",
        (Locale::ZhCn, "speaker_library_location_saved") => "已保存声纹库地址",
        (Locale::EnUs, "speaker_library_location_saved") => "Speaker Library location saved",
        (Locale::ZhCn, "keep_speaker_profile") => "保留此档案",
        (Locale::EnUs, "keep_speaker_profile") => "Keep this profile",
        (Locale::ZhCn, "delete_speaker_profile") => "删除此声纹档案？",
        (Locale::EnUs, "delete_speaker_profile") => "Delete this voice profile?",
        (Locale::ZhCn, "merge_speaker_profiles") => "合并声纹档案？",
        (Locale::EnUs, "merge_speaker_profiles") => "Merge voice profiles?",
        (Locale::ZhCn, "merge_document_speaker") => "合并说话人？",
        (Locale::EnUs, "merge_document_speaker") => "Merge speakers?",
        (Locale::ZhCn, "search_transcript_changed") => "命中的原文已变更，请重新搜索。",
        (Locale::EnUs, "search_transcript_changed") => "The matching transcript has changed. Search again.",
        (Locale::ZhCn, "saved_markdown") => "笔记",
        (Locale::EnUs, "saved_markdown") => "Notes",
        (Locale::ZhCn, "saved_markdown_hint") => "已保存的 Markdown；未保存的修改保留在编辑器中。",
        (Locale::EnUs, "saved_markdown_hint") => "Saved Markdown. Unsaved changes remain in the editor.",
        (Locale::ZhCn, "model_current_selection") => "当前选择",
        (Locale::EnUs, "model_current_selection") => "Selected",
        (Locale::ZhCn, "copy_transcript") => "复制转写",
        (Locale::EnUs, "copy_transcript") => "Copy transcript",
        (Locale::ZhCn, "transcript_copied") => "转写已复制。",
        (Locale::EnUs, "transcript_copied") => "Transcript copied.",
        (Locale::ZhCn, "copy_saved_markdown") => "复制笔记",
        (Locale::EnUs, "copy_saved_markdown") => "Copy notes",
        (Locale::ZhCn, "saved_markdown_copied") => "笔记已复制。",
        (Locale::EnUs, "saved_markdown_copied") => "Notes copied.",
        (Locale::ZhCn, "unattributed_speech") => "未标注说话人",
        (Locale::EnUs, "unattributed_speech") => "Unattributed speech",
        (Locale::ZhCn, "workspace_search") => "搜索工作区",
        (Locale::ZhCn, "search_placeholder") => "搜索原文、笔记、标题…",
        (Locale::ZhCn, "search_submit") => "搜索",
        (Locale::ZhCn, "search_clear") => "清除",
        (Locale::ZhCn, "search_hint") => "输入文字后按回车搜索已保存内容。⌘⇧F 打开搜索。",
        (Locale::ZhCn, "search_pending") => "正在搜索本地文档…",
        (Locale::ZhCn, "search_empty") => "没有找到匹配内容，试试更短的词句。",
        (Locale::ZhCn, "search_truncated") => "仅显示前 50 个匹配文档，请缩小搜索范围。",
        (Locale::ZhCn, "search_title") => "标题",
        (Locale::ZhCn, "search_tag") => "标签",
        (Locale::ZhCn, "search_summary") => "摘要",
        (Locale::ZhCn, "search_transcript") => "转写原文",
        (Locale::ZhCn, "search_markdown") => "Markdown 笔记",
        (Locale::EnUs, "workspace_search") => "Search workspace",
        (Locale::EnUs, "search_placeholder") => "Search transcripts, notes, titles…",
        (Locale::EnUs, "search_submit") => "Search",
        (Locale::EnUs, "search_clear") => "Clear",
        (Locale::EnUs, "search_hint") => "Press Enter to search saved content. ⌘⇧F opens search.",
        (Locale::EnUs, "search_pending") => "Searching local documents…",
        (Locale::EnUs, "search_empty") => "No matching content. Try a shorter phrase.",
        (Locale::EnUs, "search_truncated") => "Showing the first 50 matching documents. Refine your search.",
        (Locale::EnUs, "search_title") => "Title",
        (Locale::EnUs, "search_tag") => "Tag",
        (Locale::EnUs, "search_summary") => "Summary",
        (Locale::EnUs, "search_transcript") => "Transcript",
        (Locale::EnUs, "search_markdown") => "Markdown notes",
        (Locale::ZhCn, "assistant") => "助手",
        (Locale::ZhCn, "meetings") => "项目",
        (Locale::ZhCn, "models") => "模型",
        (Locale::ZhCn, "settings") => "设置",
        (Locale::ZhCn, "navigate_back") => "后退",
        (Locale::ZhCn, "navigate_forward") => "前进",
        (Locale::ZhCn, "toggle_sidebar") => "折叠或展开侧边栏",
        (Locale::ZhCn, "resize_sidebar") => "调整侧边栏宽度",
        (Locale::ZhCn, "navigation") => "导航",
        (Locale::ZhCn, "dismiss_notice") => "关闭提示",
        (Locale::ZhCn, "local_intelligence") => "本地语音资料整理",
        (Locale::ZhCn, "ask_library") => "询问你的语音库",
        (Locale::ZhCn, "ask_placeholder") => "询问你的录音……",
        (Locale::ZhCn, "assistant_prompt") => "助手请求",
        (Locale::ZhCn, "send") => "发送",
        (Locale::ZhCn, "organize_library") => "整理语音库",
        (Locale::ZhCn, "find_project") => "查找项目",
        (Locale::ZhCn, "split_project") => "拆分项目",
        (Locale::ZhCn, "audio_projects") => "音频项目",
        (Locale::ZhCn, "project_tree_shortcuts") => "Option-↑/↓ 在同级中调整所选项目的顺序；Option-→ 把它移入上方的同级项目；Option-← 把它移到上一层的末尾。",
        (Locale::ZhCn, "import_audio") => "导入音频",
        (Locale::ZhCn, "no_projects") => "还没有项目。导入音频开始整理。",
        (Locale::ZhCn, "documents_unavailable") => "暂时无法读取项目列表。",
        (Locale::ZhCn, "retry_workspace_read") => "重新读取",
        (Locale::ZhCn, "open_detail") => "打开详情",
        (Locale::ZhCn, "show_detail") => "显示详情",
        (Locale::ZhCn, "hide_detail") => "隐藏详情",
        (Locale::ZhCn, "process") => "处理项目",
        (Locale::ZhCn, "reprocess") => "重新处理",
        (Locale::ZhCn, "cancel") => "取消",
        (Locale::ZhCn, "summary") => "摘要",
        (Locale::ZhCn, "original") => "转录",
        (Locale::ZhCn, "no_summary") => "还没有总结。可以使用助手整理这个项目。",
        (Locale::ZhCn, "need_process") => "请先处理项目以生成带 speaker 的转录。",
        (Locale::ZhCn, "empty_meetings_title") => "从一段音频开始",
        (Locale::ZhCn, "empty_meetings") => "导入录音，转录后即可阅读和整理。",
        (Locale::ZhCn, "transcript_empty_title") => "尚未处理",
        (Locale::ZhCn, "transcript_empty_hint") => "处理后，这里会显示带说话人的逐字稿",
        (Locale::ZhCn, "processing_empty_title") => "正在处理",
        (Locale::ZhCn, "processing_empty_hint") => "完成后，这里会显示带说话人的逐字稿",
        (Locale::ZhCn, "audio_missing_title") => "找不到音频文件",
        (Locale::ZhCn, "audio_missing_hint") => "重新关联音频后即可处理",
        (Locale::ZhCn, "show_issues") => "展开",
        (Locale::ZhCn, "hide_issues") => "收起",
        (Locale::ZhCn, "no_projects_title") => "还没有项目",
        (Locale::ZhCn, "no_projects_hint") => "导入一段录音，开始整理",
        (Locale::ZhCn, "no_selection_title") => "未选择项目",
        (Locale::ZhCn, "no_selection_hint") => "从左侧选择一个项目查看",
        (Locale::ZhCn, "project_detail") => "项目详情",
        (Locale::ZhCn, "close_detail") => "关闭详情",
        (Locale::ZhCn, "models_ready") => "语音识别和说话人分离模型均已就绪",
        (Locale::ZhCn, "models_asr_not_ready") => {
            "语音识别模型尚未就绪，可在下方下载或到设置中选择"
        }
        (Locale::ZhCn, "models_diarization_not_ready") => {
            "说话人分离模型尚未就绪，可在下方下载或到设置中选择"
        }
        (Locale::ZhCn, "models_not_ready") => {
            "语音识别和说话人分离模型尚未就绪，可在下方下载或到设置中选择"
        }
        (Locale::ZhCn, "download") => "下载",
        (Locale::ZhCn, "downloaded") => "已下载",
        (Locale::ZhCn, "not_downloaded") => "未下载",
        (Locale::ZhCn, "download_queued") => "排队中",
        (Locale::ZhCn, "downloading") => "下载中",
        (Locale::ZhCn, "download_paused") => "已暂停",
        (Locale::ZhCn, "verifying") => "校验中",
        (Locale::ZhCn, "download_preparing") => "准备下载",
        (Locale::ZhCn, "download_waiting") => "等待开始",
        (Locale::ZhCn, "download_verifying") => "正在校验下载的文件",
        (Locale::ZhCn, "no_models") => "模型目录中没有可用模型。",
        (Locale::ZhCn, "task_asr") => "语音识别",
        (Locale::ZhCn, "task_diarization") => "说话人分离",
        (Locale::ZhCn, "catalog_candidate_source") => "来源尚未验证，暂不可下载",
        (Locale::ZhCn, "catalog_requires_native_adapter") => "需要原生适配后才能使用，暂不可下载",
        (Locale::ZhCn, "catalog_unavailable") => "暂不可下载",
        (Locale::ZhCn, "model_source") => "来源",
        (Locale::ZhCn, "show_model_details") => "展开详情",
        (Locale::ZhCn, "hide_model_details") => "收起详情",
        (Locale::ZhCn, "declared_checksums") => "目录声明的校验和",
        (Locale::ZhCn, "model_failure_detail") => "失败详情",
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
        (Locale::ZhCn, "processing_transcription") => "转录",
        (Locale::ZhCn, "processing_diarization") => "说话人分离",
        (Locale::ZhCn, "processing_stage_waiting") => "等待中",
        (Locale::ZhCn, "processing_stage_completed") => "已完成",
        (Locale::ZhCn, "processing_stage_cancelled") => "已取消",
        (Locale::ZhCn, "ready_to_process") => "等待处理",
        (Locale::ZhCn, "processing_progress") => "处理进度",
        (Locale::ZhCn, "processing_preparing") => "正在准备……",
        (Locale::ZhCn, "transcribing") => "正在转录",
        (Locale::ZhCn, "transcription_done") => "转录完成",
        (Locale::ZhCn, "transcription_failed") => "转录失败",
        (Locale::ZhCn, "transcription_cancelled") => "转录已取消",
        (Locale::ZhCn, "diarizing") => "正在分离说话人",
        (Locale::ZhCn, "diarization_waiting") => "等待说话人分离",
        (Locale::ZhCn, "diarization_done") => "说话人分离完成",
        (Locale::ZhCn, "diarization_failed") => "说话人分离失败",
        (Locale::ZhCn, "diarization_cancelled") => "说话人分离已取消",
        (Locale::ZhCn, "models_active") => "下载中",
        (Locale::ZhCn, "model_status") => "模型状态",
        (Locale::ZhCn, "choose_audio") => "选择音频",
        (Locale::ZhCn, "go_to_models") => "前往模型",
        (Locale::ZhCn, "choose_model") => "选择模型",
        (Locale::ZhCn, "models_missing_both") => "语音模型尚未就绪",
        (Locale::ZhCn, "models_missing_asr") => "语音识别模型尚未就绪",
        (Locale::ZhCn, "models_missing_diarization") => "说话人分离模型尚未就绪",
        (Locale::ZhCn, "interface_description") => "界面偏好与本地化选项",
        (Locale::ZhCn, "transcription_description") => "选择语音识别后端、模型和转录语言",
        (Locale::ZhCn, "speaker_description") => "选择说话人分离后端、模型和人数提示",
        (Locale::ZhCn, "speaker_library_path") => "声纹库地址",
        (Locale::ZhCn, "speaker_library") => "声纹库",
        (Locale::ZhCn, "speaker_library_loading") => "正在读取声纹库……",
        (Locale::ZhCn, "speaker_library_empty") => "声纹库中还没有说话人档案。",
        (Locale::ZhCn, "speaker_profile_name_placeholder") => "档案名称",
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
        (Locale::ZhCn, "assistant_settings_saved_draft_kept") => {
            "已保存提交的设置，后续编辑已保留。"
        }
        (Locale::ZhCn, "assistant_connection_required") => "请先配置 Assistant 连接。",
        (Locale::ZhCn, "assistant_scene") => "场景",
        (Locale::ZhCn, "assistant_scope") => "文档范围",
        (Locale::ZhCn, "content_mode") => "内容视图",
        (Locale::ZhCn, "assistant_scene_ask") => "问答",
        (Locale::ZhCn, "assistant_scene_organize") => "整理",
        (Locale::ZhCn, "assistant_scene_partition") => "完整拆分",
        (Locale::ZhCn, "assistant_scene_extract") => "提取片段",
        (Locale::ZhCn, "assistant_scene_move") => "调整项目树",
        (Locale::ZhCn, "assistant_scene_ask_description") => "根据所选文档回答问题并标出依据，不修改文档。",
        (Locale::ZhCn, "assistant_scene_organize_description") => "按你的要求改进标题、标签和摘要，不改动转写内容。",
        (Locale::ZhCn, "assistant_scene_partition_description") => "按话题把文档拆成若干子文档，完整覆盖全部转写。",
        (Locale::ZhCn, "assistant_scene_extract_description") => "只把与你的要求相关的片段提取为子文档，其余内容不动。",
        (Locale::ZhCn, "assistant_scene_move_description") => "按你的要求移动文档或调整同级顺序，不改动内容。",
        (Locale::ZhCn, "assistant_current_document") => "仅当前文档",
        (Locale::ZhCn, "assistant_all_visible_documents") => "全部可见文档",
        (Locale::ZhCn, "assistant_no_document") => "未选择文档",
        (Locale::ZhCn, "assistant_first_connection_title") => "先连接一个模型",
        (Locale::ZhCn, "assistant_first_connection_hint") => "在助手设置中添加连接；你的问题会留在这里。",
        (Locale::ZhCn, "assistant_first_document_title") => "先导入一段音频",
        (Locale::ZhCn, "assistant_first_document_hint") => "把音频处理成文档后，再让助手帮你阅读和整理。",
        (Locale::ZhCn, "assistant_imported_title") => "音频尚未转录",
        (Locale::ZhCn, "assistant_imported_hint") => "打开当前项目，处理音频后即可根据转写内容提问。",
        (Locale::ZhCn, "assistant_open_project") => "打开当前项目",
        (Locale::ZhCn, "assistant_no_connection") => "未配置连接",
        (Locale::ZhCn, "assistant_local_connection") => "本机连接",
        (Locale::ZhCn, "assistant_remote_connection") => "远程连接",
        (Locale::ZhCn, "assistant_scope_notice") => "只发送当前明确选择的文档；不会发送音频、凭据、本机路径或其他文档。",
        (Locale::ZhCn, "assistant_all_visible_scope_notice") => "发送当前工作区内全部可见文档；不会发送音频、凭据或本机路径。",
        (Locale::ZhCn, "assistant_running") => "助手正在运行",
        (Locale::ZhCn, "assistant_cancelling") => "正在取消",
        (Locale::ZhCn, "assistant_completed") => "助手任务已完成",
        (Locale::ZhCn, "assistant_partial") => "助手任务部分完成",
        (Locale::ZhCn, "assistant_cancelled") => "助手任务已取消",
        (Locale::ZhCn, "assistant_awaiting_confirmation") => "等待确认写回",
        (Locale::ZhCn, "assistant_no_answer") => "本次任务没有文本回答。",
        (Locale::ZhCn, "assistant_evidence") => "依据片段",
        (Locale::ZhCn, "assistant_model_route") => "模型",
        (Locale::ZhCn, "assistant_usage") => "输入/输出/总 token",
        (Locale::ZhCn, "assistant_diagnostics") => "执行诊断",
        (Locale::ZhCn, "assistant_undo") => "撤销",
        (Locale::ZhCn, "assistant_undoing") => "正在撤销",
        (Locale::ZhCn, "assistant_receipt_written") => "已写入变更",
        (Locale::ZhCn, "assistant_undone") => "已撤销",
        (Locale::ZhCn, "assistant_pending") => "等待写回确认",
        (Locale::ZhCn, "assistant_pending_paused") => "任务在这项写回前暂停：接受才会写入，拒绝则丢弃这项变更。",
        (Locale::ZhCn, "assistant_accepting") => "正在写回……",
        (Locale::ZhCn, "assistant_rejecting") => "正在拒绝……",
        (Locale::ZhCn, "assistant_retry") => "重试",
        (Locale::ZhCn, "assistant_open_settings") => "打开助手设置",
        (Locale::ZhCn, "assistant_error_unknown") => "助手任务没有完成",
        (Locale::ZhCn, "assistant_error_authentication") => "API key 无效或已过期",
        (Locale::ZhCn, "assistant_error_authentication_hint") => "在助手设置中更新 API key，然后重试。",
        (Locale::ZhCn, "assistant_error_credential_file") => "无法从密钥文件读取 API key",
        (Locale::ZhCn, "assistant_error_credential_file_hint") => "确认密钥文件存在且写有 API key，或在助手设置中直接填写。",
        (Locale::ZhCn, "assistant_error_connection") => "助手连接尚未配置好",
        (Locale::ZhCn, "assistant_error_connection_hint") => "在助手设置中选择或补全一个连接。",
        (Locale::ZhCn, "assistant_error_disabled") => "助手已在设置中关闭",
        (Locale::ZhCn, "assistant_error_disabled_hint") => "在助手设置中启用后再发送。",
        (Locale::ZhCn, "assistant_error_timeout") => "模型响应超时",
        (Locale::ZhCn, "assistant_error_timeout_hint") => "稍后重试；如果经常超时，可在助手设置中延长超时时间。",
        (Locale::ZhCn, "assistant_error_rate_limited") => "模型服务暂时限流",
        (Locale::ZhCn, "assistant_error_rate_limited_hint") => "稍等片刻再重试。",
        (Locale::ZhCn, "assistant_error_provider_response") => "模型服务没有正常响应",
        (Locale::ZhCn, "assistant_error_provider_response_hint") => "确认连接地址可以访问、模型可用，然后重试。",
        (Locale::ZhCn, "assistant_error_model_unsupported") => "当前模型不支持助手所需的能力",
        (Locale::ZhCn, "assistant_error_model_unsupported_hint") => "在助手设置中换用支持工具调用的模型。",
        (Locale::ZhCn, "assistant_error_context_unknown") => "模型拒绝了请求，而连接没有设置上下文上限",
        (Locale::ZhCn, "assistant_error_context_unknown_hint") => "在助手设置中填写该模型的上下文 token 上限。",
        (Locale::ZhCn, "assistant_error_context_exceeded") => "内容超出了模型的上下文上限",
        (Locale::ZhCn, "assistant_error_context_exceeded_hint") => "缩小文档范围，或在助手设置中换用上下文更大的模型。",
        (Locale::ZhCn, "assistant_error_step_limit") => "任务步骤达到上限，已停止",
        (Locale::ZhCn, "assistant_error_step_limit_hint") => "把请求拆小一些，或在助手设置中提高最大工具轮次。",
        (Locale::ZhCn, "assistant_error_invalid_change") => "模型给出的操作无效",
        (Locale::ZhCn, "assistant_error_invalid_change_hint") => "再试一次，或换个说法描述请求。",
        (Locale::ZhCn, "assistant_error_out_of_scope") => "文档不在本次授权范围内",
        (Locale::ZhCn, "assistant_error_out_of_scope_hint") => "选择要处理的文档，或把范围改为全部可见文档。",
        (Locale::ZhCn, "assistant_error_document_changed") => "文档已在别处修改或移除",
        (Locale::ZhCn, "assistant_error_document_changed_hint") => "重新运行任务，助手会读取最新内容。",
        (Locale::ZhCn, "assistant_error_tree_rule") => "项目树不能这样调整",
        (Locale::ZhCn, "assistant_error_tree_rule_hint") => "项目树最多三层，父子文档须来自同一段录音，文档也不能移到自己下面。",
        (Locale::ZhCn, "assistant_error_write_failed") => "写入文档失败",
        (Locale::ZhCn, "assistant_error_write_failed_hint") => "这项变更没有写入，可以再试一次。",
        (Locale::ZhCn, "assistant_error_expired") => "确认已过期",
        (Locale::ZhCn, "assistant_error_expired_hint") => "重新运行任务，以生成新的确认。",
        (Locale::ZhCn, "assistant_error_unavailable") => "助手服务暂不可用",
        (Locale::ZhCn, "assistant_error_unavailable_hint") => "稍后再试；如果仍不可用，请重新启动 MeetNote。",
        (Locale::ZhCn, "assistant_error_interrupted") => "任务意外中止",
        (Locale::ZhCn, "assistant_error_interrupted_hint") => "再试一次。",
        (Locale::ZhCn, "assistant_error_cancelled") => "任务已取消",
        (Locale::ZhCn, "assistant_error_cancelled_hint") => "已写入的变更会保留。",
        (Locale::ZhCn, "assistant_range") => "转写片段",
        (Locale::ZhCn, "assistant_accept") => "接受并写回",
        (Locale::ZhCn, "assistant_reject") => "拒绝",
        (Locale::ZhCn, "assistant_rejected") => "已拒绝本次写回。",
        (Locale::ZhCn, "assistant_expired") => "确认已过期，请重新运行任务。",
        (Locale::ZhCn, "assistant_tree_root") => "顶层",
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
        (Locale::ZhCn, "retry_document_read") => "重新读取",
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
        (Locale::ZhCn, "detail_no_results") => "尚未处理",
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
        (Locale::ZhCn, "documents_root_operation_pending") => {
            "当前操作仍在进行；结束或取消完成后再切换文档目录。"
        }
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
        (Locale::EnUs, "navigate_back") => "Back",
        (Locale::EnUs, "navigate_forward") => "Forward",
        (Locale::EnUs, "toggle_sidebar") => "Toggle sidebar",
        (Locale::EnUs, "resize_sidebar") => "Resize sidebar",
        (Locale::EnUs, "navigation") => "Navigation",
        (Locale::EnUs, "dismiss_notice") => "Dismiss",
        (Locale::EnUs, "local_intelligence") => "local voice library",
        (Locale::EnUs, "ask_library") => "Ask your voice library",
        (Locale::EnUs, "ask_placeholder") => "Ask about your recordings…",
        (Locale::EnUs, "assistant_prompt") => "Assistant request",
        (Locale::EnUs, "send") => "Send",
        (Locale::EnUs, "organize_library") => "Organize voice library",
        (Locale::EnUs, "find_project") => "Find a project",
        (Locale::EnUs, "split_project") => "Split a project",
        (Locale::EnUs, "audio_projects") => "AUDIO PROJECTS",
        (Locale::EnUs, "project_tree_shortcuts") => "Option-Up and Option-Down move the selected project among its siblings; Option-Right moves it into the project above; Option-Left moves it to the end of the level above.",
        (Locale::EnUs, "import_audio") => "Import audio",
        (Locale::EnUs, "no_projects") => "No projects yet. Import audio to begin.",
        (Locale::EnUs, "documents_unavailable") => "The project list cannot be read right now.",
        (Locale::EnUs, "retry_workspace_read") => "Retry",
        (Locale::EnUs, "open_detail") => "Open detail",
        (Locale::EnUs, "show_detail") => "Show details",
        (Locale::EnUs, "hide_detail") => "Hide details",
        (Locale::EnUs, "process") => "Process project",
        (Locale::EnUs, "reprocess") => "Reprocess",
        (Locale::EnUs, "cancel") => "Cancel",
        (Locale::EnUs, "summary") => "Summary",
        (Locale::EnUs, "original") => "Transcript",
        (Locale::EnUs, "no_summary") => {
            "No summary yet. Use the Assistant to organize this project."
        }
        (Locale::EnUs, "transcript_empty_title") => "Not processed yet",
        (Locale::EnUs, "transcript_empty_hint") => {
            "Process it to see the transcript with speakers here"
        }
        (Locale::EnUs, "empty_meetings_title") => "Start with a recording",
        (Locale::EnUs, "empty_meetings") => "Import audio to transcribe, read and organize.",
        (Locale::EnUs, "processing_empty_title") => "Processing",
        (Locale::EnUs, "processing_empty_hint") => {
            "The transcript with speakers appears here when it finishes"
        }
        (Locale::EnUs, "audio_missing_title") => "Audio file not found",
        (Locale::EnUs, "audio_missing_hint") => "Relink the audio to process it",
        (Locale::EnUs, "show_issues") => "Show",
        (Locale::EnUs, "hide_issues") => "Hide",
        (Locale::EnUs, "no_projects_title") => "No projects yet",
        (Locale::EnUs, "no_projects_hint") => "Import a recording to get started",
        (Locale::EnUs, "no_selection_title") => "No project selected",
        (Locale::EnUs, "no_selection_hint") => "Choose a project on the left",
        (Locale::EnUs, "project_detail") => "Project details",
        (Locale::EnUs, "close_detail") => "Close detail",
        (Locale::EnUs, "models_ready") => {
            "Speech recognition and speaker diarization models are ready"
        }
        (Locale::EnUs, "models_asr_not_ready") => {
            "The speech recognition model is not ready. Download one below or choose one in Settings."
        }
        (Locale::EnUs, "models_diarization_not_ready") => {
            "The speaker diarization model is not ready. Download one below or choose one in Settings."
        }
        (Locale::EnUs, "models_not_ready") => {
            "Speech recognition and speaker diarization models are not ready. Download them below or choose them in Settings."
        }
        (Locale::EnUs, "download") => "Download",
        (Locale::EnUs, "downloaded") => "Downloaded",
        (Locale::EnUs, "not_downloaded") => "Not downloaded",
        (Locale::EnUs, "download_queued") => "Queued",
        (Locale::EnUs, "downloading") => "Downloading",
        (Locale::EnUs, "download_paused") => "Paused",
        (Locale::EnUs, "verifying") => "Verifying",
        (Locale::EnUs, "download_preparing") => "Preparing",
        (Locale::EnUs, "download_waiting") => "Waiting to start",
        (Locale::EnUs, "download_verifying") => "Checking the downloaded files",
        (Locale::EnUs, "no_models") => "No models are available in the catalog.",
        (Locale::EnUs, "task_asr") => "Speech recognition",
        (Locale::EnUs, "task_diarization") => "Speaker diarization",
        (Locale::EnUs, "catalog_candidate_source") => {
            "The source is not verified yet, so it cannot be downloaded"
        }
        (Locale::EnUs, "catalog_requires_native_adapter") => {
            "Needs a native adapter before it can be downloaded"
        }
        (Locale::EnUs, "catalog_unavailable") => "Not available for download yet",
        (Locale::EnUs, "model_source") => "Source",
        (Locale::EnUs, "show_model_details") => "Show details",
        (Locale::EnUs, "hide_model_details") => "Hide details",
        (Locale::EnUs, "declared_checksums") => "Checksums in the catalog",
        (Locale::EnUs, "model_failure_detail") => "Failure details",
        (Locale::EnUs, "probe_backends") => "Backend checks",
        (Locale::EnUs, "probe_artifacts") => "Artifact checksums",
        (Locale::EnUs, "expected_checksum") => "expected",
        (Locale::EnUs, "no_probe_record") => "No probe record yet",
        (Locale::EnUs, "probe_passed") => "passed",
        (Locale::EnUs, "probe_failed") => "failed",
        (Locale::EnUs, "probe_kind_download") => "Post-download check",
        (Locale::EnUs, "probe_kind_reprobe") => "Re-probe",
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
        (Locale::EnUs, "processing_transcription") => "Transcription",
        (Locale::EnUs, "processing_diarization") => "Speaker identification",
        (Locale::EnUs, "processing_stage_waiting") => "Waiting",
        (Locale::EnUs, "processing_stage_completed") => "Completed",
        (Locale::EnUs, "processing_stage_cancelled") => "Cancelled",
        (Locale::EnUs, "ready_to_process") => "Ready to process",
        (Locale::EnUs, "processing_progress") => "Processing progress",
        (Locale::EnUs, "processing_preparing") => "Preparing…",
        (Locale::EnUs, "transcribing") => "Transcribing",
        (Locale::EnUs, "transcription_done") => "Transcribed",
        (Locale::EnUs, "transcription_failed") => "Transcription failed",
        (Locale::EnUs, "transcription_cancelled") => "Transcription cancelled",
        (Locale::EnUs, "diarizing") => "Identifying speakers",
        (Locale::EnUs, "diarization_waiting") => "Waiting to identify speakers",
        (Locale::EnUs, "diarization_done") => "Speakers identified",
        (Locale::EnUs, "diarization_failed") => "Speaker identification failed",
        (Locale::EnUs, "diarization_cancelled") => "Speaker identification cancelled",
        (Locale::EnUs, "models_active") => "Downloads active",
        (Locale::EnUs, "model_status") => "Model status",
        (Locale::EnUs, "choose_audio") => "Choose audio",
        (Locale::EnUs, "go_to_models") => "Go to Models",
        (Locale::EnUs, "choose_model") => "Choose model",
        (Locale::EnUs, "models_missing_both") => "Models aren't ready",
        (Locale::EnUs, "models_missing_asr") => "The transcription model isn't ready",
        (Locale::EnUs, "models_missing_diarization") => "The speaker separation model isn't ready",
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
        (Locale::EnUs, "assistant_settings_saved_draft_kept") => {
            "Submitted settings saved; later edits kept."
        }
        (Locale::EnUs, "assistant_connection_required") => "Configure an Assistant connection first.",
        (Locale::EnUs, "assistant_scene") => "Scene",
        (Locale::EnUs, "assistant_scope") => "Document scope",
        (Locale::EnUs, "content_mode") => "Content view",
        (Locale::EnUs, "assistant_scene_ask") => "Ask",
        (Locale::EnUs, "assistant_scene_organize") => "Organize",
        (Locale::EnUs, "assistant_scene_partition") => "Partition",
        (Locale::EnUs, "assistant_scene_extract") => "Extract",
        (Locale::EnUs, "assistant_scene_move") => "Move tree",
        (Locale::EnUs, "assistant_scene_ask_description") => "Answers questions from the selected documents and cites the evidence; nothing is changed.",
        (Locale::EnUs, "assistant_scene_organize_description") => "Refines the title, tags and summary you ask for; the transcript stays as recorded.",
        (Locale::EnUs, "assistant_scene_partition_description") => "Splits the document into topic sub-documents that together cover the whole transcript.",
        (Locale::EnUs, "assistant_scene_extract_description") => "Pulls only the passages that match your request into sub-documents; the rest stays as it is.",
        (Locale::EnUs, "assistant_scene_move_description") => "Moves a document or reorders it among its siblings as you ask; content is unchanged.",
        (Locale::EnUs, "assistant_current_document") => "Current document only",
        (Locale::EnUs, "assistant_all_visible_documents") => "All visible documents",
        (Locale::EnUs, "assistant_no_document") => "No document selected",
        (Locale::EnUs, "assistant_first_connection_title") => "Connect a model to begin",
        (Locale::EnUs, "assistant_first_connection_hint") => "Add a connection in Assistant settings. Your question will stay here.",
        (Locale::EnUs, "assistant_first_document_title") => "Start with an audio recording",
        (Locale::EnUs, "assistant_first_document_hint") => "Process the audio into a document, then ask the Assistant to help read and organize it.",
        (Locale::EnUs, "assistant_imported_title") => "This audio has not been transcribed yet",
        (Locale::EnUs, "assistant_imported_hint") => "Open the current project and process its audio to ask questions about the transcript.",
        (Locale::EnUs, "assistant_open_project") => "Open current project",
        (Locale::EnUs, "assistant_no_connection") => "No connection",
        (Locale::EnUs, "assistant_local_connection") => "Local connection",
        (Locale::EnUs, "assistant_remote_connection") => "Remote connection",
        (Locale::EnUs, "assistant_scope_notice") => "Only the explicitly selected document is sent; audio, credentials, local paths, and other documents stay local.",
        (Locale::EnUs, "assistant_all_visible_scope_notice") => "All documents visible in the current workspace are sent; audio, credentials, and local paths stay local.",
        (Locale::EnUs, "assistant_running") => "Assistant is running",
        (Locale::EnUs, "assistant_cancelling") => "Cancelling",
        (Locale::EnUs, "assistant_completed") => "Assistant task completed",
        (Locale::EnUs, "assistant_partial") => "Assistant task partially completed",
        (Locale::EnUs, "assistant_cancelled") => "Assistant task cancelled",
        (Locale::EnUs, "assistant_awaiting_confirmation") => "Awaiting write confirmation",
        (Locale::EnUs, "assistant_no_answer") => "This task returned no text answer.",
        (Locale::EnUs, "assistant_evidence") => "Evidence segments",
        (Locale::EnUs, "assistant_model_route") => "Model",
        (Locale::EnUs, "assistant_usage") => "Input/output/total tokens",
        (Locale::EnUs, "assistant_diagnostics") => "Execution diagnostics",
        (Locale::EnUs, "assistant_undo") => "Undo",
        (Locale::EnUs, "assistant_undoing") => "Undoing",
        (Locale::EnUs, "assistant_receipt_written") => "Change written",
        (Locale::EnUs, "assistant_undone") => "Undone",
        (Locale::EnUs, "assistant_pending") => "Write confirmation pending",
        (Locale::EnUs, "assistant_pending_paused") => "The run paused before this write: accepting writes it, rejecting discards it.",
        (Locale::EnUs, "assistant_accepting") => "Writing…",
        (Locale::EnUs, "assistant_rejecting") => "Rejecting…",
        (Locale::EnUs, "assistant_retry") => "Retry",
        (Locale::EnUs, "assistant_open_settings") => "Open Assistant settings",
        (Locale::EnUs, "assistant_error_unknown") => "The Assistant task did not finish",
        (Locale::EnUs, "assistant_error_authentication") => "The API key is invalid or has expired",
        (Locale::EnUs, "assistant_error_authentication_hint") => "Update the API key in Assistant settings, then try again.",
        (Locale::EnUs, "assistant_error_credential_file") => "The API key could not be read from its file",
        (Locale::EnUs, "assistant_error_credential_file_hint") => "Make sure the key file exists and holds the key, or enter the key in Assistant settings.",
        (Locale::EnUs, "assistant_error_connection") => "The Assistant connection is not set up",
        (Locale::EnUs, "assistant_error_connection_hint") => "Choose or complete a connection in Assistant settings.",
        (Locale::EnUs, "assistant_error_disabled") => "The Assistant is turned off",
        (Locale::EnUs, "assistant_error_disabled_hint") => "Turn it on in Assistant settings, then send again.",
        (Locale::EnUs, "assistant_error_timeout") => "The model took too long to respond",
        (Locale::EnUs, "assistant_error_timeout_hint") => "Try again later; if it keeps timing out, raise the timeout in Assistant settings.",
        (Locale::EnUs, "assistant_error_rate_limited") => "The model service is limiting requests",
        (Locale::EnUs, "assistant_error_rate_limited_hint") => "Wait a moment, then try again.",
        (Locale::EnUs, "assistant_error_provider_response") => "The model service did not respond properly",
        (Locale::EnUs, "assistant_error_provider_response_hint") => "Check that the connection URL is reachable and the model is available, then try again.",
        (Locale::EnUs, "assistant_error_model_unsupported") => "This model lacks what the Assistant needs",
        (Locale::EnUs, "assistant_error_model_unsupported_hint") => "Choose a model with tool calling in Assistant settings.",
        (Locale::EnUs, "assistant_error_context_unknown") => "The model rejected the request and the connection has no context limit",
        (Locale::EnUs, "assistant_error_context_unknown_hint") => "Enter the model's context token limit in Assistant settings.",
        (Locale::EnUs, "assistant_error_context_exceeded") => "The content exceeds the model's context limit",
        (Locale::EnUs, "assistant_error_context_exceeded_hint") => "Narrow the document scope, or choose a model with a larger context in Assistant settings.",
        (Locale::EnUs, "assistant_error_step_limit") => "The task reached its step limit and stopped",
        (Locale::EnUs, "assistant_error_step_limit_hint") => "Split the request into smaller ones, or raise the maximum tool rounds in Assistant settings.",
        (Locale::EnUs, "assistant_error_invalid_change") => "The model proposed an invalid change",
        (Locale::EnUs, "assistant_error_invalid_change_hint") => "Try again, or rephrase the request.",
        (Locale::EnUs, "assistant_error_out_of_scope") => "The document is outside this request's scope",
        (Locale::EnUs, "assistant_error_out_of_scope_hint") => "Select the document to work on, or widen the scope to all visible documents.",
        (Locale::EnUs, "assistant_error_document_changed") => "The document was changed or removed elsewhere",
        (Locale::EnUs, "assistant_error_document_changed_hint") => "Run the task again to read the latest version.",
        (Locale::EnUs, "assistant_error_tree_rule") => "The project tree cannot change this way",
        (Locale::EnUs, "assistant_error_tree_rule_hint") => "The tree has at most three levels, a parent and its children share one recording, and a document cannot move under itself.",
        (Locale::EnUs, "assistant_error_write_failed") => "The document could not be written",
        (Locale::EnUs, "assistant_error_write_failed_hint") => "This change was not written; try again.",
        (Locale::EnUs, "assistant_error_expired") => "The confirmation expired",
        (Locale::EnUs, "assistant_error_expired_hint") => "Run the task again for a new confirmation.",
        (Locale::EnUs, "assistant_error_unavailable") => "The Assistant service is unavailable",
        (Locale::EnUs, "assistant_error_unavailable_hint") => "Try again later; if it stays unavailable, restart MeetNote.",
        (Locale::EnUs, "assistant_error_interrupted") => "The task stopped unexpectedly",
        (Locale::EnUs, "assistant_error_interrupted_hint") => "Try again.",
        (Locale::EnUs, "assistant_error_cancelled") => "The task was cancelled",
        (Locale::EnUs, "assistant_error_cancelled_hint") => "Changes already written are kept.",
        (Locale::EnUs, "assistant_range") => "Transcript",
        (Locale::EnUs, "assistant_accept") => "Accept and write",
        (Locale::EnUs, "assistant_reject") => "Reject",
        (Locale::EnUs, "assistant_rejected") => "This write was rejected.",
        (Locale::EnUs, "assistant_expired") => "Confirmation expired; run the task again.",
        (Locale::EnUs, "assistant_tree_root") => "to the top level",
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
        (Locale::EnUs, "retry_document_read") => "Retry reading",
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
        (Locale::EnUs, "detail_no_results") => "Not processed yet",
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
        (Locale::EnUs, "documents_root_operation_pending") => {
            "Wait for the current operation to finish or stop before switching the documents folder."
        }
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

pub fn message(locale: Locale, key: &str, value: &str) -> String {
    match (locale, key) {
        (Locale::ZhCn, "voice_sample_count") => format!("声纹样本 · {value}"),
        (Locale::EnUs, "voice_sample_count") => format!("Voice samples · {value}"),
        (Locale::ZhCn, "search_count") => format!("匹配文档 · {value}"),
        (Locale::EnUs, "search_count") => format!("Matching documents · {value}"),
        (Locale::ZhCn, "search_failed") => format!("搜索失败：{value}"),
        (Locale::EnUs, "search_failed") => format!("Search failed: {value}"),
        (Locale::ZhCn, "search_unreadable") => format!("{value} 个文档未能读取，结果不完整。"),
        (Locale::EnUs, "search_unreadable") => format!("Could not read {value} documents. Results are incomplete."),
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
        (Locale::ZhCn, "speaker_number") => format!("说话人 {value}"),
        (Locale::ZhCn, "speakers_summary") => format!("说话人 · {value}"),
        (Locale::EnUs, "speakers_summary") => format!("Speakers · {value}"),
        (Locale::EnUs, "speaker_number") => format!("Speaker {value}"),
        (Locale::ZhCn, "speaker_count") => format!("{value} 位说话人"),
        (Locale::EnUs, "speaker_count") if value == "1" => "1 speaker".into(),
        (Locale::EnUs, "speaker_count") => format!("{value} speakers"),
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
        (Locale::ZhCn, "transcribing_count") => format!("正在转录 {value}"),
        (Locale::EnUs, "transcribing_count") => format!("Transcribing {value}"),
        (Locale::ZhCn, "diarizing_count") => format!("正在分离说话人 {value}"),
        (Locale::EnUs, "diarizing_count") => format!("Identifying speakers {value}"),
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
        (Locale::ZhCn, "assistant_document_count") => format!("{value} 篇文档"),
        (Locale::EnUs, "assistant_document_count") => match value {
            "1" => "1 document".into(),
            _ => format!("{value} documents"),
        },
        (Locale::ZhCn, "assistant_receipts") => format!("已写入 {value} 项变更，可逐项撤销"),
        (Locale::EnUs, "assistant_receipts") => {
            format!("{value} writes applied; each can be undone")
        }
        (Locale::ZhCn, "assistant_quoted") => format!("「{value}」"),
        (Locale::EnUs, "assistant_quoted") => format!("“{value}”"),
        (Locale::ZhCn, "assistant_receipt_created") => format!("已新建{value}"),
        (Locale::EnUs, "assistant_receipt_created") => format!("Created {value}"),
        (Locale::ZhCn, "assistant_receipt_updated") => format!("已更新{value}"),
        (Locale::EnUs, "assistant_receipt_updated") => format!("Updated {value}"),
        (Locale::ZhCn, "assistant_receipt_deleted") => format!("已删除{value}"),
        (Locale::EnUs, "assistant_receipt_deleted") => format!("Deleted {value}"),
        (Locale::ZhCn, "assistant_uncovered_count") => format!("{value} 段转写未纳入"),
        (Locale::EnUs, "assistant_uncovered_count") => match value {
            "1" => "1 transcript segment left out".into(),
            _ => format!("{value} transcript segments left out"),
        },
        (Locale::ZhCn, "assistant_pending_affects") => format!("将修改{value}"),
        (Locale::EnUs, "assistant_pending_affects") => format!("Changes {value}"),
        (Locale::ZhCn, "assistant_tree_under") => format!("{value}下"),
        (Locale::EnUs, "assistant_tree_under") => format!("under {value}"),
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
        (Locale::ZhCn, "workspace_read_failed") => format!("读取失败：{value}"),
        (Locale::EnUs, "workspace_read_failed") => format!("Read failed: {value}"),
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
        (Locale::ZhCn, "assistant_tree_moved") => format!("{first}移到{second}"),
        (Locale::EnUs, "assistant_tree_moved") => format!("Moves {first} {second}"),
        (Locale::ZhCn, "assistant_tree_reordered") => {
            format!("{first}调整到同级第 {second} 位")
        }
        (Locale::EnUs, "assistant_tree_reordered") => {
            format!("Moves {first} to position {second} among its siblings")
        }
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
