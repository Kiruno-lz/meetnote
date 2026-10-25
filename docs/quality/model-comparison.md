# 本机 ASR 模型与后端评测

日期：2026-10-25。硬件：Apple M2 Pro, 10 cores, 32 GiB；Rust runner 使用 release 构建。

文本评分为精确 Levenshtein；中文 CER、英文原样 WER，按参考单位加权。英文同时给出 whisper-normalizer 0.1.15 的 normalized WER（数字、缩写等口语格式归一），原样错误率保留，不将格式差异隐去。两次执行共用固定输入。原生短片段、构造长片段、AMI 标注裁剪片段分别列出。RTF 越低越快；RSS 是每个独立配置进程的峰值，不能视为独立 GPU 显存。

原生后端每次调用重新加载模型，耗时包含加载与解码；MLX 是驻留 session，表中为推理调用耗时，加载时间保存在逐项 receipt，不能把两种加载策略当成同一部署性能。模型 checksum 校验会预热文件缓存，本报告不测磁盘冷启动。

| 配置 | 语言/场景 | 样本×重复 | CER/WER | normalized WER | RTF | 调用中位/P95 秒 | 进程峰值 MiB | 失败 |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| paraformer-cpu-1 | zh/composed-clean | 1×2 | 94.88% | — | 0.0875 | 25.855/25.863 | 1292 | 0 |
| paraformer-cpu-1 | zh/composed-noise20db | 1×2 | 99.67% | — | 0.0866 | 25.585/25.636 | 1292 | 0 |
| paraformer-cpu-1 | zh/native | 24×2 | 11.76% | — | 0.0756 | 0.896/1.066 | 1292 | 0 |
| paraformer-cpu-4 | zh/composed-clean | 1×2 | 94.88% | — | 0.0278 | 8.213/8.240 | 1304 | 0 |
| paraformer-cpu-4 | zh/composed-noise20db | 1×2 | 99.67% | — | 0.0272 | 8.030/8.065 | 1304 | 0 |
| paraformer-cpu-4 | zh/native | 24×2 | 11.76% | — | 0.0535 | 0.631/0.712 | 1304 | 0 |
| paraformer-cpu-6 | zh/composed-clean | 1×2 | 94.88% | — | 0.0247 | 7.292/7.336 | 1313 | 0 |
| paraformer-cpu-6 | zh/composed-noise20db | 1×2 | 99.67% | — | 0.0246 | 7.254/7.297 | 1313 | 0 |
| paraformer-cpu-6 | zh/native | 24×2 | 11.76% | — | 0.0518 | 0.612/0.704 | 1313 | 0 |
| paraformer-final-text-app-dev | zh/composed-clean | 1×2 | 14.81% | — | 0.0177 | 5.234/5.274 | 399 | 0 |
| paraformer-final-text-app-dev | zh/composed-noise20db | 1×2 | 16.23% | — | 0.0175 | 5.169/5.195 | 399 | 0 |
| paraformer-final-text-app-validation | zh/composed-clean | 1×2 | 14.62% | — | 0.0181 | 5.042/5.099 | 404 | 0 |
| paraformer-final-text-app-validation | zh/composed-noise20db | 1×2 | 17.67% | — | 0.0178 | 4.949/4.963 | 404 | 0 |
| paraformer-final-text-dev | zh/composed-clean | 1×2 | 14.81% | — | 0.0171 | 5.059/5.065 | 445 | 0 |
| paraformer-final-text-dev | zh/composed-noise20db | 1×2 | 16.34% | — | 0.0171 | 5.052/5.054 | 445 | 0 |
| paraformer-final-text-dev | zh/native | 24×2 | 11.76% | — | 0.0539 | 0.637/0.698 | 445 | 0 |
| paraformer-final-text-validation | zh/composed-clean | 1×2 | 15.14% | — | 0.0174 | 4.838/4.857 | 445 | 0 |
| paraformer-final-text-validation | zh/composed-noise20db | 1×2 | 17.88% | — | 0.0174 | 4.835/4.839 | 445 | 0 |
| paraformer-final-text-validation | zh/native | 24×2 | 10.94% | — | 0.0567 | 0.629/0.713 | 445 | 0 |
| paraformer-validation-before | zh/composed-clean | 1×2 | 93.48% | — | 0.0310 | 8.635/9.792 | 1214 | 0 |
| paraformer-validation-before | zh/composed-noise20db | 1×2 | 99.68% | — | 0.0277 | 7.714/7.859 | 1214 | 0 |
| paraformer-validation-before | zh/native | 24×2 | 10.94% | — | 0.0590 | 0.654/0.740 | 1214 | 0 |
| qwen-0.6b-ami | en/ami-oracle-nonoverlap | 5×2 | 16.84% | 12.00% | 0.0332 | 0.254/0.423 | 957 | 0 |
| qwen-0.6b-mlx | en/composed-clean | 1×2 | 10.39% | 10.00% | 0.0274 | 6.736/6.772 | 1029 | 0 |
| qwen-0.6b-mlx | en/composed-noise20db | 1×2 | 73.06% | 72.81% | 0.0160 | 3.934/3.944 | 1029 | 0 |
| qwen-0.6b-mlx | en/native | 24×2 | 5.81% | 5.26% | 0.0303 | 0.291/0.461 | 1029 | 0 |
| qwen-0.6b-mlx | zh/composed-clean | 1×2 | 10.78% | — | 0.0214 | 6.334/6.439 | 1029 | 0 |
| qwen-0.6b-mlx | zh/composed-noise20db | 1×2 | 11.76% | — | 0.0216 | 6.389/6.461 | 1029 | 0 |
| qwen-0.6b-mlx | zh/native | 24×2 | 9.48% | — | 0.0248 | 0.267/0.377 | 1029 | 0 |
| qwen-1.7b-ami | en/ami-oracle-nonoverlap | 5×2 | 21.05% | 13.00% | 0.0500 | 0.396/0.572 | 1406 | 0 |
| qwen-1.7b-mlx | en/composed-clean | 1×2 | 13.38% | 13.16% | 0.0401 | 9.861/10.065 | 1471 | 0 |
| qwen-1.7b-mlx | en/composed-noise20db | 1×2 | 72.89% | 71.93% | 0.0260 | 6.383/6.425 | 1471 | 0 |
| qwen-1.7b-mlx | en/native | 24×2 | 3.35% | 2.81% | 0.0445 | 0.431/0.706 | 1471 | 0 |
| qwen-1.7b-mlx | zh/composed-clean | 1×2 | 9.26% | — | 0.0329 | 9.710/9.717 | 1471 | 0 |
| qwen-1.7b-mlx | zh/composed-noise20db | 1×2 | 9.91% | — | 0.0330 | 9.747/9.778 | 1471 | 0 |
| qwen-1.7b-mlx | zh/native | 24×2 | 9.04% | — | 0.0349 | 0.418/0.554 | 1471 | 0 |
| sensevoice-final | en/ami-oracle-nonoverlap | 5×2 | 26.32% | 20.00% | 0.0929 | 0.655/0.893 | 1169 | 0 |
| sensevoice-final | en/composed-clean | 1×2 | 11.09% | 9.47% | 0.0582 | 14.317/14.341 | 1169 | 0 |
| sensevoice-final | en/composed-noise20db | 1×2 | 76.23% | 75.79% | 0.0582 | 14.305/14.309 | 1169 | 0 |
| sensevoice-final | en/native | 24×2 | 9.51% | 7.89% | 0.0787 | 0.754/1.142 | 1169 | 0 |
| sensevoice-final | zh/composed-clean | 1×2 | 12.64% | — | 0.0575 | 16.994/17.067 | 1169 | 0 |
| sensevoice-final | zh/composed-noise20db | 1×2 | 16.88% | — | 0.0579 | 17.095/17.141 | 1169 | 0 |
| sensevoice-final | zh/native | 24×2 | 10.35% | — | 0.0720 | 0.855/1.038 | 1169 | 0 |
| sensevoice-final-text | en/composed-clean | 1×2 | 11.09% | 9.47% | 0.0585 | 14.386/14.440 | 1074 | 0 |
| sensevoice-final-text | en/composed-noise20db | 1×2 | 76.23% | 75.79% | 0.0586 | 14.416/14.505 | 1074 | 0 |
| sensevoice-final-text | zh/composed-clean | 1×2 | 12.64% | — | 0.0576 | 17.006/17.087 | 1074 | 0 |
| sensevoice-final-text | zh/composed-noise20db | 1×2 | 16.88% | — | 0.0576 | 17.022/17.074 | 1074 | 0 |
| sensevoice-final-text-app | en/composed-clean | 1×2 | 11.09% | 9.47% | 0.0606 | 14.899/15.048 | 1081 | 0 |
| whisper-base-en-cpu | 执行失败 | — | — | — | — | — | — | runner exit -11; incomplete output |
| whisper-base-en-cpu-fixed | en/ami-oracle-nonoverlap | 5×2 | 31.58% | 26.00% | 0.0508 | 0.381/0.410 | 382 | 0 |
| whisper-base-en-cpu-fixed | en/native | 24×2 | 9.68% | 8.60% | 0.0399 | 0.389/0.452 | 382 | 0 |
| whisper-base-en-metal | 执行失败 | — | — | — | — | — | — | runner exit -11; incomplete output |
| whisper-base-en-metal-fixed | en/ami-oracle-nonoverlap | 5×2 | 29.47% | 24.00% | 0.0291 | 0.217/0.257 | 331 | 0 |
| whisper-base-en-metal-fixed | en/native | 24×2 | 9.68% | 8.60% | 0.0242 | 0.233/0.338 | 331 | 0 |
| whisper-large-metal | 执行失败 | — | — | — | — | — | — | runner exit -11; incomplete output |
| whisper-large-metal-fixed | en/ami-oracle-nonoverlap | 5×2 | 20.00% | 12.00% | 0.3377 | 2.654/2.793 | 3719 | 0 |
| whisper-large-metal-fixed | en/composed-clean | 1×2 | 61.09% | 60.70% | 0.1694 | 41.649/41.796 | 3719 | 0 |
| whisper-large-metal-fixed | en/composed-noise20db | 1×2 | 106.69% | 105.96% | 0.2000 | 49.174/49.247 | 3719 | 0 |
| whisper-large-metal-fixed | en/native | 24×2 | 4.93% | 3.68% | 0.2677 | 2.632/3.070 | 3719 | 0 |
| whisper-large-metal-fixed | zh/composed-clean | 1×2 | 38.78% | — | 0.1129 | 33.344/33.465 | 3719 | 0 |
| whisper-large-metal-fixed | zh/composed-noise20db | 1×2 | 26.25% | — | 0.1733 | 51.210/51.214 | 3719 | 0 |
| whisper-large-metal-fixed | zh/native | 24×2 | 10.13% | — | 0.2475 | 2.986/3.356 | 3719 | 0 |
| whisper-turbo-q5-metal | 执行失败 | — | — | — | — | — | — | runner exit -11; incomplete output |
| whisper-turbo-q5-metal-fixed | en/ami-oracle-nonoverlap | 5×2 | 23.16% | 15.00% | 0.1757 | 1.309/1.391 | 958 | 0 |
| whisper-turbo-q5-metal-fixed | en/composed-clean | 1×2 | 59.51% | 58.60% | 0.0814 | 20.000/20.031 | 958 | 0 |
| whisper-turbo-q5-metal-fixed | en/composed-noise20db | 1×2 | 89.79% | 89.12% | 0.0886 | 21.770/21.792 | 958 | 0 |
| whisper-turbo-q5-metal-fixed | en/native | 24×2 | 6.34% | 5.26% | 0.1340 | 1.314/1.377 | 958 | 0 |
| whisper-turbo-q5-metal-fixed | zh/composed-clean | 1×2 | 15.14% | — | 0.0696 | 20.549/20.555 | 958 | 0 |
| whisper-turbo-q5-metal-fixed | zh/composed-noise20db | 1×2 | 15.36% | — | 0.0655 | 19.344/19.351 | 958 | 0 |
| whisper-turbo-q5-metal-fixed | zh/native | 24×2 | 11.00% | — | 0.1146 | 1.371/1.420 | 958 | 0 |

模型版本、设备、线程、加载策略、模型文件 SHA-256 与分组结果见同目录 `asr-comparison.json`；逐样本评分及耗时保留在忽略的评测运行目录 `comparison-details.json`，其 SHA-256 写入汇总。原始预测与音频不提交。

FLEURS 来源固定为 google/fleurs `70bb2e84b976b7e960aa89f1c648e09c59f894dd` 的中英文 test；AMI 为 ES2002a Mix-Headset 和 manual 1.6.2 官方词标注，二者均为 CC-BY-4.0。AMI 使用官方非重叠片段与标注边界裁剪，不能证明自动 VAD 或重叠会议能力。

两次执行的波动不能替代统计显著性。数据含朗读与少量会话，不代表小时级真实会议、说话人分离、词时间戳或发布质量验收。
