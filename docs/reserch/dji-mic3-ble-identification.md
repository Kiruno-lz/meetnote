# DJI Mic 3 TX 蓝牙识别：型号、SN 与固件

## 探索原因

MeetNote 要在不插线的情况下识别 DJI Mic 3 发射器（TX），并取得型号、序列号和固件版本，用于设备导入与资产归属。识别有四条约束：

- 只能依据设备在蓝牙上发出的原始字节。
- 不能用近似字段做判断。
- 不能读录音文件里的 `bext`。
- 不能把 TX 当作经典蓝牙耳机（HFP）去连接。

被动方式已经验证读不到这些信息。TX 处于配对模式时，原始 HCI 里能看到的只有四样：

- LE 广播与扫描应答，只有名称 `DJI Mic 3 TX-a73b42`、Flags、UUID `FFF0`、厂商数据 `AA 08 B2 00 00 8C 58 23 A7 3B 43`、服务数据 `FDEE`；
- 经典蓝牙的 EIR，含名称、厂商 `0x038F`、UUID16 列表和 DeviceID `AA 08 01 00 01 00 01 00`；
- 远端版本：LL 5.4，厂商 `0x02B0`（恒玄 BES），subversion `0x0017`；
- GAP 设备名 `BES_BLE`。

这些里面都没有 SN、固件或型号字符串。一旦通过 HFP 连接，TX 会退出配对广播，后续也就无从读取。所以识别必须改走 LE GATT，按设备的私有协议读取。

## 探索目标

1. 确定 TX 在 LE GATT 上的服务布局，以及读写数据所用的通道。
2. 解出通道上的帧格式和校验算法，能构造设备接受的请求帧。
3. 找出型号、SN、固件、Loader 各自对应的字节位置，并标明每个字段是确认值还是推断值。
4. 验证普通用户权限下的完整识别流程：不需要管理员密码，也不需要 HCI 抓包。

## 探索结果

### 识别结论

| 字段 | 取值来源 | 示例（2026-11-20 实测） | 定性 |
|---|---|---|---|
| 型号标识 | `00/01` 版本查询应答中的 `hw_version`，必须**严格等于** `DM303_TX` | `DM303_TX` | 设备原始字节 |
| SN | `5B/03` 推送帧 record `0x01` 中，固件 4 字节之后的 ASCII | `9SFXN8N024XU81` | 设备原始字节 |
| 固件 | `5B/03` 推送帧 record `0x01` 的前 4 字节，按小端逐字节以十进制显示 | `0A 00 01 03` → `03.01.00.10` | 字节位置确认；版本格式见下 |
| Loader | `00/01` 应答中位于 `hw_version` 之后的 4 字节，按标准字段位置推断 | `10 05 19 63` → `99.25.05.16` | 推断 |
| 名称后缀 | `5B/03` 推送帧 record `0x04` | `a73b42` | 设备原始字节 |

关于固件和 Loader 的说明：

- **固件格式有两处佐证。** `00/01` 应答中位于标准 `app_version` 位置的 4 字节同样是 `0A 00 01 03`。
- **固件尚未核对 DJI Mimo。** 没有和 Mimo 里显示的版本比对过。//TODO - 在 Mimo 中查看该 TX 的固件版本，确认它与 `03.01.00.10` 一致。
- **Loader 只有字段位置作为依据。** 它的取值没有第二个来源可以印证。

字面量 `DMT03` 在任何广播、EIR、ATT PDU 和原始 trace 字节中都没有出现，同一设备只在 USB 描述符里写作 `DJI MIC3 DMT03`。因此：

- `DMT03` 是 USB 和官方型号代码；
- `DM303_TX` 是蓝牙上的硬件标识；
- 两者分属不同字段，实现时不能互相替代。现有的 `is_model_code`（三个大写字母加两位数字）本身也不接受 `DM303_TX`。

### GATT 布局

TX 只接受 LE 连接（CoreBluetooth `connect(options: nil)`）。连接后对端会先发起 MTU 请求（`0xFB`），再读取 Mac 的 GAP 名称。

| 服务 | 特征 | ATT handle | 属性 | 用途 |
|---|---|---|---|---|
| `FFF0` | `FFF3` | `0x8002` | read/write/notify/indicate | 用 WriteReq 写入会返回 ATT 错误 `0xFC`，但请求仍会被处理；不作为写通道 |
| `FFF0` | `FFF4` | `0x8005` | read/write/notify/indicate | **设备→App 的 DUML 通道**。即使 CCCD 尚未打开，也会主动推送 |
| `FFF0` | `FFF5` | `0x8008` | read/writeWithoutResponse/notify/indicate | **App→设备的 DUML 写通道**，使用 Write Command |
| `1844` | `2B7D`/`2B7E`/`2B7F` | `0x4402`… | — | LE Audio Volume Control，与识别无关 |
| `15F1E600-A277-43FC-A484-DD39EF8A9100` | `…E601`/`…E602` | `0xA002`… | — | 用途未知，不得写入 |

对 `FFF3`、`FFF4`、`FFF5` 直接 Read 会返回 ATT 错误 `0x13`，读 CCCD 也会失败。这是设备自身的行为，不影响识别。

### DUML 帧格式

所有 `FFF4` 和 `FFF5` 上的数据都是 DJI DUML v1 帧，多字节字段一律小端：

```text
off  len  字段
0    1    0x55
1    2    bits 0-9 总长度（含 CRC16），bits 10-15 版本 = 1   例：34 04 → len 52, ver 1
3    1    CRC8(bytes 0..2)，reflected poly 0x8C，seed 0x77
4    1    sender   （低 5 位 type，高 3 位 index）TX = 0x3A（type 26 / index 1）
5    1    receiver  App = 0x02
6    2    seq      应答原样回带请求的 seq
8    1    cmd type  请求 0x40（需应答）、推送 0x00、应答 0x80
9    1    cmdset
10   1    cmdid
11   n    payload
-2   2    CRC16(bytes 0..len-3)，reflected poly 0x8408，seed 0x3692
```

这两个 CRC 算法用 52、48、14、43 字节四类实测帧验证过，全部通过。发往 receiver `0x3A` 和 `0x00` 都能收到应答，发往 `0x1A` 没有应答。

### TX 主动推送（无需请求）

配对模式下，TX 大约每 0.5 秒在 `FFF4` 上循环推送三种帧：

| 帧 | payload | 含义 |
|---|---|---|
| `5B/03`，52 字节 | `03 24 00` + record `01` + record `04` | 设备信息：固件 + SN、名称后缀 |
| `5B/03`，48 字节 | `03 20 00` + record `02`（26 字节） | 状态，含义未解析 |
| `5B/04`，14 字节 | `05` | 心跳 |

`5B/03` 的 payload 结构如下：

```text
03                         子类型
24 00                      后续总长度（36）
01 | 00 00 00 00 | 12 | 0A 00 01 03  39 53 46 58 4E 38 4E 30 32 34 58 55 38 31
type  reserved      len   固件(4)      SN "9SFXN8N024XU81"(14)
04 | 00 00 00 00 | 06 | 61 37 33 62 34 32                       名称后缀 "a73b42"
```

每条 record 的格式是 `type(1) reserved(4) len(1) value(len)`。解析时按 `len` 逐条推进，不能按固定偏移取值。

### 版本查询 `00/01`（只读）

请求帧（`seq = 0x1001`）如下，写入 `FFF5`：

```text
55 0D 04 33 02 3A 01 10 40 00 01 9D C2
```

应答在大约 0.1 秒后从 `FFF4` 返回：

```text
55 2B 04 58 3A 02 01 10 80 00 01 | 00 | 01 | 44 4D 33 30 33 5F 54 58 00×8 | 10 05 19 63 | 0A 00 01 03 | 01 00 00 00 | 5A DC
                                  status ?    hw_version[16] "DM303_TX"     loader      app_version   未知
```

`status = 0x00` 表示成功。`hw_version` 需要去掉末尾的 `0x00` 后再做严格比较。

### 识别流程

1. 无过滤扫描 LE 广播。匹配条件是名称以 `DJI Mic 3 TX` 开头，或厂商数据以 `AA 08` 开头。这一步只用来挑选候选设备，不作为识别结果。
2. 建立 LE 连接，发现服务 `FFF0`，在 `FFF4` 上开启通知。
3. 向 `FFF5` 写入 `00/01` 请求，然后从 `FFF4` 收帧：先按长度切帧，丢弃 CRC 不通过或 sender 不是 `0x3A` 的帧。
4. 收齐以下两帧后断开连接，判定识别成功：
   - `00/01` 应答，并且 `hw_version == "DM303_TX"`；
   - `5B/03` record `0x01`。
5. 输出型号、SN、固件，其中固件取自 `5B/03`；Loader 作为推断字段附带输出。

整个流程不需要 sudo，也不需要 HCI trace。要求有两条：

- 执行进程必须是包含 `NSBluetoothAlwaysUsageDescription` 的 `.app`，并通过 LaunchServices 启动。从终端或 Claude Code 直接运行的 CLI，蓝牙授权会被归到父进程，因而被 TCC 拒绝。
- 授权与 ad-hoc 签名的 cdhash 绑定，重新签名后需要重新授权。

## 复现说明

### 无设备：协议解析回归

用下面两条实测帧作为固定输入：

```text
00/01 应答: 55 2B 04 58 3A 02 3A 10 80 00 01 00 01 44 4D 33 30 33 5F 54 58 00 00 00 00 00 00 00 00 10 05 19 63 0A 00 01 03 01 00 00 00 78 DE
5B/03 推送: 55 34 04 AC 3A 02 7F 63 00 5B 03 03 24 00 01 00 00 00 00 12 0A 00 01 03 39 53 46 58 4E 38 4E 30 32 34 58 55 38 31 04 00 00 00 00 06 61 37 33 62 34 32 AA 6F
```

实现至少要覆盖以下用例：

| 用例 | 期望 |
|---|---|
| 两帧齐全 | 型号 `DM303_TX`、SN `9SFXN8N024XU81`、固件 `03.01.00.10`、Loader `99.25.05.16` |
| 只有 `5B/03` 推送 | 不识别（缺少型号） |
| `00/01` 应答 CRC16 末字节被篡改 | 该帧被丢弃，型号为空 |
| `hw_version` 改为 `DMT03_TX`，并重算 CRC16 | 型号读出 `DMT03_TX`，但不识别（严格相等） |
| 构造请求帧 `build(0x02, 0x3A, 0x1001, 0x40, 0x00, 0x01)` | 字节等于 `55 0D 04 33 02 3A 01 10 40 00 01 9D C2` |

### 有设备：实机识别

1. 让 TX 进入配对模式。配对窗口有时限，超时后需要重新进入。
2. 用已获蓝牙授权的 `.app` 执行上面的识别流程。实测时，从开始扫描到拿到 `00/01` 应答约 18 秒。其中开头 9 秒在等候广播被扫到，连接后又按固定间隔等了 8 秒才写入请求。
3. 核对输出与本文示例一致。换一台设备时，SN 和名称后缀会不同；型号必须仍是 `DM303_TX`。

### 原始字节取证（只在协议有疑问时使用，需要管理员权限）

1. 打开 bluetoothd 的文件 trace：

   ```bash
   sudo defaults write /Library/Preferences/com.apple.MobileBluetooth.debug HCITraces -dict-add HCIFileTraces -bool true StackDebugEnabled -bool true
   sudo notifyutil -p com.apple.bluetooth.prefsChanged
   ```

2. trace 写在 `/private/var/root/Library/Logs/Bluetooth/bluetoothd-hci-latest.pklg`。关闭 trace 时文件会被删除，因此必须在 trace 运行期间复制出来。
3. 用完后恢复原有的 plist 并再次发送上面的通知。
4. 解析 `.pklg` 文件：记录头是 `len(4, 大端) ts(8) type(1)`；type `0x02` 为 ACL 发送，`0x03` 为 ACL 接收。按 PB 标志重组 L2CAP，CID `0x0004` 即 ATT。

## 注意事项与补充

- **禁止 HFP。** 不调用任何会建立经典蓝牙连接的 API，包括 IOBluetooth 连接、CoreBluetooth 的 transport bridging 选项，以及系统设置里的“连接”。
- **只发只读命令。** 目前只验证过 `00/01`。`cmdset 0x5B` 等其他命令的语义未知，写入可能改动设备设置，未经单独评估不得发送。`15F1E600…` 服务同样不得写入。
- **严格比较。** 型号判断只接受 `DM303_TX` 的完全相等，不能用前缀、包含或大小写无关的比较；名称、厂商数据、`FDEE` 服务数据都不能作为型号依据。
- **固件不能只取一处。** 固件以 `5B/03` 为准，但实现应同时校验它与 `00/01` 的 `app_version` 一致。两者不一致时报告错误，不要静默取其中一个。
- **数据量不大。** 推送帧会持续到达，拿到所需的两帧就应立即断开，避免占用 TX 的配对窗口。
- **接收方覆盖未知。** 只用一台 TX（固件 `03.01.00.10`）实测过，接收器（DMR03）和充电盒（DMC03）没有验证。//TODO - 在 RX 上运行同一流程，记录它的 `hw_version`，并确认 `5B/03` 的格式是否相同。
- **字段语义有待确认。** `00/01` 应答中 `status` 之后的 `0x01`、末尾的 `01 00 00 00`，以及 `5B/03` record `0x02` 的 26 字节，含义都未解析，不作为识别依据。

## 附录：核心代码思路

探索用的临时脚本已经清理，这里只保留可以直接移植的核心逻辑。

```python
def crc8(data, seed=0x77):              # DUML header CRC
    c = seed
    for b in data:
        c ^= b
        for _ in range(8):
            c = (c >> 1) ^ 0x8C if c & 1 else c >> 1
    return c

def crc16(data, seed=0x3692):           # DUML frame CRC, appended little-endian
    c = seed
    for b in data:
        c ^= b
        for _ in range(8):
            c = (c >> 1) ^ 0x8408 if c & 1 else c >> 1
    return c

def build(sender, receiver, seq, cmd_type, cmdset, cmdid, payload=b""):
    n = 13 + len(payload)
    head = bytes([0x55, n & 0xFF, ((n >> 8) & 0x03) | (1 << 2)])
    head += bytes([crc8(head), sender, receiver, seq & 0xFF, seq >> 8, cmd_type, cmdset, cmdid]) + payload
    return head + crc16(head).to_bytes(2, "little")

def frames(blob):                       # split one notification into CRC-valid frames
    i = 0
    while i + 13 <= len(blob):
        n = blob[i + 1] | ((blob[i + 2] & 0x03) << 8)
        f = blob[i:i + n]
        if blob[i] == 0x55 and n >= 13 and len(f) == n and crc8(f[:3]) == f[3] \
                and int.from_bytes(f[-2:], "little") == crc16(f[:-2]):
            yield f; i += n
        else:
            i += 1

def version(b4):                        # 0A 00 01 03 -> "03.01.00.10"
    return ".".join(f"{b:02d}" for b in reversed(b4))

def records(p):                         # 5B/03 payload: sub(1) total(2) {type(1) rsv(4) len(1) value}
    end, i = min(len(p), 3 + int.from_bytes(p[1:3], "little")), 3
    while i + 6 <= end:
        yield p[i], p[i + 6:i + 6 + p[i + 5]]
        i += 6 + p[i + 5]

# For every frame f with f[4] == 0x3A, p = f[11:-2]:
#   f[9:11] == 00 01 and f[8] & 0x80 and p[0] == 0: model = p[2:18].rstrip(b"\0"); loader = version(p[18:22]); app = version(p[22:26])
#   f[9:11] == 5B 03 and p[0] == 0x03: record 0x01 -> firmware = version(v[:4]), sn = v[4:].decode("ascii"); record 0x04 -> name suffix
# identified = model == b"DM303_TX" and sn and firmware == app
```

探索时的 GATT 探针做了这样几件事：

- 一个 CoreBluetooth `.app`，用参数驱动，避免因重新编译导致蓝牙授权失效；
- 无过滤扫描，然后只走 LE 的 `connect`；
- 发现全部服务和特征，订阅 notify/indicate；
- 按 `service/characteristic=hex` 格式的列表，在连接 8 秒后依次写入，每次间隔 1.5 秒。

正式实现只需要保留扫描、连接、订阅 `FFF4`、写入 `FFF5` 和上面的解析逻辑。
