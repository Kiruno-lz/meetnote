# 录音设备接入计划

## 目标与边界

接入白名单内“蓝牙识别 + USB 大容量存储”录音设备（当前仅 DJI Mic 3 TX）：首次在配对模式下经蓝牙读取并校验型号、SN 与固件，再保持配对模式有线连接完成配对；此后有线连接即自动把新录音拷贝到导入目录、在工作区创建文档并排队分析。

- 白名单制：只有目录（`crates/devices::catalog`）中列出的型号会被连接、展示、配对与导入；同厂商未列出的产品同样忽略。
- 设备只是音频来源。录音进入工作区后与手动导入的文档完全相同：同一 `DocumentService` 写入、同一处理链路、同一外部编辑与恢复规则。
- 设备注册表与导入账本是设备同步状态，不是文档事实来源；丢失后最坏结果是重新配对、重复导入提示，不会破坏文档。
- 设备上的原文件默认保留。删除仅在用户于设备卡片“高级”中开启后、拷贝经 SHA-256 校验一致时执行。
- 设备协议只读：仅发送 DUML `00/01` 版本查询，不写 `cmdset 0x5B`、不访问 `15F1E600-…` 服务、不建立经典蓝牙 HFP 连接，不读写设备设置，不做实时录音或无线传输。

## 实测事实（DMT03，2026-11-20）

测试设备：型号 DMT03，SN `9SFXN8N024XU81`，MAC `8C:58:23:A7:3B:43`，固件 `03.01.00.10`。macOS 27.2.1，设备处于配对模式且未连接接收器/手机。协议细节、帧样本与回归用例见 [DJI Mic 3 BLE 识别研究](../reserch/dji-mic3-ble-identification.md)。

| 来源 | 观察到的内容 | 结论 |
|---|---|---|
| BLE local name | `DJI Mic 3 TX-a73b42`；部分扫描响应为 `BES_BLE` | 机型名来自 name；后缀是经典蓝牙地址低 3 字节，不是 SN |
| BLE manufacturer data | `aa08 b20000 8c5823a73b43` | 公司 ID `0x08AA`（DJI，小端）；`b20000` 为产品码；末 6 字节为 BLE MAC |
| LE GATT `FFF0`（连接后） | FFF5 Write Command 发送 DUML `00/01` → FFF4 notify 回 `hw_version = DM303_TX`、loader、app 版本；另收到 `5B/03` 推送，记录 `0x01` 为固件（4 字节小端）+ ASCII SN，`0x04` 为名称后缀 | **型号、SN、固件的唯一可信来源**；广播字段一律不提供 SN/固件 |
| 经典蓝牙 EIR / SDP、`system_profiler`、SPP `/dev/cu.DJIMic3TX-*` | EIR/SDP 无 SN/固件；`system_profiler` 固件恒为 `0.0.1`；SPP 只有二进制调试帧 | 不作为识别来源 |
| USB | `idVendor 0xBE57`（bestechnic）`idProduct 0x0104` “Wireles Mic Tx”，序列号恒为 `20151006.1`；FAT 卷 “NO NAME” | USB 只能识别机型，无 SN/固件；实例以卷 `VolumeUUID` 区分 |
| 卷内录音 | 每段录音成对写出 `…_orig.WAV`（原始）与 `…_edit.WAV`（机内处理） | 只导入 `_orig`；缺原始文件时保留 `_edit` |

## 架构

```text
GPUI UI(rune)  设备页、添加设备窗口
      │  DeviceHost 状态投影
apps/meetnote::workspace_host::devices   编排：发现 → 校验 → 配对 → 导入 → 排队分析
      │                          │
crates/devices                   crates/application
  catalog(白名单型号目录)           device_import::DeviceImport（计划→拷贝→建档→账本）
  verify(GattSession 契约) ← duml   DocumentService::import_packaged_audio
  ble(Sightings、校验状态)
  registry(devices.json)           （staging → 文档包 → 原子 rename）
  discovery(UsbWatcher、BLE 会话)
  macos::ble(CoreBluetooth) / macos::helper / usb(ioreg+mount+diskutil)
  ingest(计划、_orig 优先、校验拷贝)
```

`crates/devices` 归属 Audio Host 的音频来源采集：纯函数（DUML 帧、身份校验、ioreg/mount/diskutil 解析、导入计划、账本去重）与平台 IO 分离，平台 IO 只在 `macos::ble`、`macos::helper`、`usb` 与 `discovery::SystemDiscovery` 中。`DeviceDiscovery` trait 是唯一的系统边界，L2 测试以脚本化实现替换。UI 只读 `DeviceHost` 投影，不执行文件 IO 或子进程；注册表读写、导入与 USB 轮询都在后台线程。

### 白名单目录与扩展

型号相关的全部事实集中在 `catalog::Model` 一条记录：`id`、显示名、型号代码（目录内唯一，用于连接 BLE 身份与 USB 连接）、广播匹配（name 前缀、含公司 ID 的 manufacturer data 前缀、MAC 来源）、USB 匹配（vendor/product ID + 产品名片段）、校验协议 `Verifier` 与录音布局 `Recordings`。BLE 识别、GATT 连接决策、USB 识别与导入筛选只查目录，不含任何厂商常量。

校验契约在 `verify`：`verify::session(model)` 按 `model.verifier` 返回 `GattProfile`（service/notify/write UUID）与 `GattSession`（`next_request` 决定写什么、`feed` 消费通知并给出 `VerifiedDevice` 或 `Rejection`）。CoreBluetooth 层只负责连接、订阅、写入与超时重试，不认识任何协议。会话按目录型号的期望值校验，设备报告其他硬件即拒绝，不会被改标为别的型号。

- 新增同协议型号（如 DJI Mic 3 RX、Mic 2）：目录加一条记录（广播前缀、USB ID、DUML 硬件名、型号代码），附实测 fixture 的 L1 测试。
- 新增协议：新模块实现 `GattSession` 并在 `Verifier` 增加变体、`verify::session` 增加一个分支；传输层不改。
- 新增录音布局：`Recordings` 增加变体并在 `ingest::list_recordings` 处理。
- 已配对设备按注册表中的型号代码查目录取录音布局；型号被移出目录后按 `Recordings::All` 导入。

### 识别与校验

- **BLE 发现**：`objc2-core-bluetooth` + `dispatch2` 专用队列，扫描全部广播、允许重复以刷新 RSSI。只有匹配目录型号广播规则的外设成为候选：DJI Mic 3 TX 为 name 前缀 `DJI Mic 3 TX`，或 manufacturer data 前缀 `AA08 B20000`（公司 ID `0x08AA` + 产品码），MAC 取 manufacturer data 末 6 字节；缺少 name 时用目录显示名。同一外设分包发送的字段按外设合并；候选按信号格（1–4）排序，RSSI 抖动不触发重绘。
- **GATT 校验**：只连接匹配目录型号的外设，并按该型号的 `Verifier` 校验。DJI DUML 流程：连接 → 发现 `FFF0` → 发现 `FFF4/FFF5` → 订阅 `FFF4` → 经 `FFF5` 无响应写入 `00/01` 查询（答复前每次通知后重发，共最多 4 次）→ `duml::Identifier` 从通知流重组帧（CRC8 `0x8C/0x77`、CRC16 `0x8408/0x3692`，只接受发送方 `0x3A`）→ 断开。通过条件全部满足：`hw_version` 严格等于目录型号的硬件名（Mic 3 TX 为 `DM303_TX`，型号代码 `DMT03`）；`5B/03` 的固件与 `00/01` 的 app 版本一致；SN 为非空可打印 ASCII。任一不满足即拒绝并显示原因，该外设不能配对。失败重试 3 次（间隔 3 s），单次识别超时 20 s；扫描会话结束时断开全部连接。
- **蓝牙权限**：TCC 以责任进程判定蓝牙用途。主 bundle 含 `NSBluetoothAlwaysUsageDescription`（`.app` 内，由 `scripts/package-macos-app.sh` 写入）时进程内扫描；未打包的 `meetnote` 二进制由 `apps/meetnote/build.rs` 以 `-sectcreate __TEXT __info_plist` 内嵌同一用途说明，并以 `responsibility_spawnattrs_setdisclaim` 重新启动自身（参数 `__meetnote-bluetooth-helper`）成为独立责任进程，子进程以 JSON 行把 `BleEvent` 写到 stdout，stdin 关闭即退出；子进程未报告任何状态就结束视为被 TCC 拒绝。状态为未授权时自动打开一次系统设置的蓝牙隐私页，并在窗口内提供“打开蓝牙设置”。`MEETNOTE_BLUETOOTH=1` 强制进程内扫描。
- **USB**：后台线程每 1.5 s 读取 `/sbin/mount`；挂载集合变化或添加窗口打开时运行 `ioreg -r -c IOUSBHostDevice -l -w0`，解析 vendor/product ID、序列号、产品名、`bcdDevice` 与子树内 `BSD Name`，与 `mount` 关联得到挂载点，再以 `diskutil info -plist` 取卷 `VolumeUUID`。只识别目录 USB 匹配（Mic 3 TX：`0xBE57:0x0104` 且产品名含 `mic tx`），DJI 厂商 ID 或名称含 `DJI` 的其他设备一律忽略。USB 只给出目录型号，序列号与版本号不作为身份。连接键优先级：USB 身份键 → `vol:<VolumeUUID>` → `usb:<vid>:<pid>:<locationID>`。
- **配对**：添加窗口单页完成。BLE 扫描与 USB 检测同时进行；校验通过的候选可点选（仅一个时自动选中）。当所选（或唯一）已校验设备与一个未配对的 USB 连接属于同一目录型号（型号代码一致）时写入注册表：键为 `sn:<SN>`，SN/固件/型号来自 GATT，MAC/蓝牙名来自广播，USB 连接键记入 `usb_links`；随后停止蓝牙扫描并进入完成步骤。MAC 只在含厂商数据的广播包中出现，而 GATT 连接期间设备不广播，校验往往先于该包完成：目录型号带 MAC 来源时，已校验且 USB 已就绪的目标最多再等 5 s 收到 MAC 后才配对，超时则不带 MAC 配对；之后任何一次扫描看到已配对 SN 的 MAC 时补写注册表（只填空值）。同一连接键再次配对到其他设备时从原设备移除。设备页在检测到未配对的目录型号插入时显示横幅，点击打开添加窗口。

### 导入

1. 已配对设备（按设备键或 `usb_links` 匹配连接）的卷挂载且工作区已打开 → 递归列出 `wav/mp3/m4a/aac/flac`（跳过隐藏目录与 `System Volume Information`），再按型号录音布局筛选：Mic 3 TX 为 `OriginalsOverEdits`，同一录音的 `_edit` 在存在 `_orig` 时剔除。
2. 账本按 `设备键 + 卷内相对路径 + 字节数 + 修改时间（秒）` 去重，只导入新文件，按修改时间升序。
3. 导入目录为空（默认）：拷贝到 `<root>/.meetnote-staging/<uuid>.part`（scanner 跳过点目录），fsync，SHA-256 与源比对 → 以最终路径 `<root>/<doc-id>/<原文件名>` 保存文档包 → rename 到包内。音频成为文档包附件，移动工作区时随文档移动。
4. 自定义目录：拷贝到 `<dir>/<原文件名>.part` 校验后 rename；目标已存在时追加 ` (n)`；再以该路径 `import_audio`。目录不能位于工作区内（会被 scanner 视为无效包）或设备卷上，选择时拒绝并提示。
5. 每个文件成功后立即写账本，中断后已完成的文件不重复导入；`.part` 残留在下次导入开始时清除。
6. 开启“导入后删除”时，校验通过才删除源文件；删除失败只报告，不回滚文档。
7. 开启“自动分析”（默认开）时，新文档进入分析队列；处理空闲时按顺序以当前 revision 启动 `ProcessingRun`，失败（如未安装模型）报告一次并清空本批队列。
8. 每个连接只自动导入一次：“本次已同步”按连接键记录，拔出后清除，再次插入重新比对账本。移除正在导入的设备会取消导入并清除其全部连接键的同步标记。

注册表 `devices.json` 与 `settings.json` 同目录，schema 版本化（`usb_links` 缺省为空），写入使用 `fs2` 文件锁 + 临时文件原子替换。

## UI

侧栏新增“设备”入口（`⌘3`，原 Models/Settings 顺延为 `⌘4`/`⌘5`，`⌘,` 不变），无二级侧栏。

**设备页**：页头 + 单列卡片（最大宽度 760 pt 居中）。无设备时空状态插画与“添加设备”按钮在剩余空间内整体居中；有设备时按钮居中位于全部卡片下方，下方保留 160 pt 空白。

**设备卡片**：

- 左侧设备插画：以 rune 原语（圆角矩形、渐变、圆点）绘制发射器机身、网罩、状态屏与按键，不使用品牌图片与标识；连接中时右上角指示灯呼吸。meetnote 未启用 rune 的 `svg` 特性，彩色 SVG 图标不会栅格化，因此不用彩色 `Icon`。
- 标题为机型名，型号代码以 badge 显示，右上角“移除”；状态行：正在导入 n/m / 正在检查新录音 / 已连接 · 已同步 / 已连接等待挂载 / 未连接，后接“{相对时间}导入 n 条”。
- 信息栅格：SN、MAC、固件三列；等宽数字、可选中复制，未知值显示“—”。
- 导入目录行：默认显示“工作区（每条录音存入其文档）”，自定义显示路径；“更改”打开文件夹选择，自定义时提供“恢复默认”。
- 导入进行中显示进度条，上方左侧为当前文件名，右侧为传输速度与百分比；速度取最近 2 s 窗口内全批累计字节（`devices::rate::TransferRate`，十进制单位，不足 0.25 s 不显示）；结束后显示“上次导入 n 个录音”。
- “导入后自动分析”开关；折叠的“高级”（箭头随展开旋转）内为“导入后删除设备上的录音”及风险说明。
- “移除”经确认对话框执行；移除同时清除该设备的账本，不删除已导入的文档、音频与设备上的文件。

**添加设备窗口**（模态对话框，两步）：

1. 连接：三条编号步骤（长按链接键进入配对模式并靠近电脑 / 经蓝牙读取并校验型号、SN、固件 / 保持配对模式用 USB-C 连接），完成的步骤显示对勾。候选行显示机型名与校验详情（已校验时分两行：`SN …` 与 `固件 …`；否则为“正在读取”或“拒绝：原因”）及状态胶囊，详情换行不截断；仅已校验候选可点选。底部状态行依次为：正在搜索 / 等待校验 / 请选择设备 / {机型}已校验，等待数据线；蓝牙关闭、未授权或不可用时改为警示行，未授权时附“打开蓝牙设置”。
2. 完成：对勾缩放弹入，展示机型名与 SN 及“以后有线连接即自动导入”。导入随即在后台开始，“完成”只关闭窗口。

步骤切换整体淡入；顶部两点指示当前步骤。

动效全部使用 rune 渲染器时钟（`Cycle`、`LoopSpec`、`Sequence`），减少动态效果时由渲染器停在静止姿态，应用不另做分支。

## 测试

| 层级 | 位置 | 覆盖 |
|---|---|---|
| L1 | `crates/devices::catalog` | id 与型号代码唯一、USB 片段小写；只匹配列出的广播（同公司 ID 的其他产品码、`DJI Mic 3 RX`、`DJI Osmo` 拒绝）与 USB 设备（同芯片 ID 的其他产品、DJI 厂商 ID 拒绝）；MAC 提取 |
| L1 | `crates/devices::duml` | 以实测帧逐字节构造 `00/01` 请求；两帧齐全才识别；CRC 损坏丢弃；`DMT03_TX` 等非严格硬件名拒绝；固件不一致拒绝；分包与噪声重组；忽略其他发送方；会话在答复前重发且上限 4 次；会话只按目录型号校验，不改标型号 |
| L1 | `crates/devices` 其他 | 实测 DMT03 分包广播合并、识别与 MAC 提取，非 DJI 广播拒绝，广播字段不提供 SN/固件，校验结果补全候选，未列出型号既不连接也不成为候选；ioreg fixture（真实 BES 形态；未列出的 DJI 设备被忽略）、挂载关联、`VolumeUUID` 解析与连接键优先级；注册表往返、`usb_links` 查找与重配迁移、schema/损坏文件拒绝且不覆盖、原子写无残留；传输速度滑动窗口与单位格式、导入计划去重与排序、录音布局（`All` 与 `OriginalsOverEdits`）、同名编号；校验拷贝、源变化时不删除；蓝牙放置（进程内 / helper / 拒绝）与状态映射；helper 事件行转发与静默退出视为拒绝；USB 轮询只在挂载变化时读描述符 |
| L1 | `apps/meetnote` | 相对时间单位选择、卡片 key 规范化、导入速度跨文件累计字节 |
| L2 | `crates/persistence/tests/device_import.rs` | 默认包内导入后 scanner 无 issue 且账本续传不重复；自定义目录 `(2)` 改名与校验删除；保留文件名拒绝 |
| L2 | `apps/meetnote` 运行时测试 `device_flow` | 假发现源驱动：BLE 校验 → USB 连接配对为 `sn:<SN>` 并记录 `vol:` 连接 → 只导入 `_orig` → 无模型时自动分析报告并清空队列 → 拔插不重复导入；已配对设备经 `usb_links` 连接即导入到自定义目录并删除源、移除确认后保留文档；校验失败的设备永不配对；校验先于 MAC 包时等待 MAC 再配对、始终无 MAC 时宽限后配对、已配对缺 MAC 时再次扫描补全；未列出的 DJI 设备不出现在候选中；未授权时只打开一次蓝牙设置 |
| L3 | `meetnote --snap … --scene <场景> --at 1.0`，场景 `devices`、`devices-empty`、`add-device`、`add-device-verified`、`add-device-paired` | 深/浅外观截图逐项过视觉清单 |
| 实机 | `meetnote devices probe [秒数]` | 未打包二进制经 helper 完成 GATT 校验：`DM303_TX` / `DMT03` / `9SFXN8N024XU81` / `03.01.00.10`。//TODO - TX 配对模式 + USB 同时连接，跑通配对 → `_orig` 导入 → 分析全流程；RX 与充电盒未验证 |
