# osc-toolkit-cli Roadmap

## 目标

使用 Rust 构建一个跨平台、高吞吐、低内存、内存安全的 OSC CLI，服务于实时 UDP/OSC 数据路径：

- N:N 输入输出映射与实时转发
- OSC 过滤和轻量 remap
- 高可靠二进制录制与精确重放
- 可观测、可验证、可由外部编排引擎管理

本项目只负责数据平面，不负责任务编排、工作流、服务调度或 UI。进程的启动、停止、重启、配置生成和多实例管理由外部编排引擎负责。

## 设计原则

1. **数据平面优先**：优化持续转发、加工、录制和重放，而不是构建通用工作流系统。
2. **N:N 是基础模型**：任意数量的 UDP inputs 可以映射到任意数量的 outputs；不把产品模型限制为 1:1 或 1:N。
3. **透明转发不解析**：未启用 OSC 加工时，原样转发 UDP datagram，不做无意义的 decode/encode。
4. **有界内存**：所有队列和缓存必须有明确上限及溢出策略，禁止无限队列。
5. **配置启动、外部编排**：运行配置默认不可变；修改配置后由外部编排引擎验证并重启进程。
6. **录制不采用旧格式**：设计面向高频、高速和崩溃恢复的新二进制格式，不复制旧 TypeScript/JSON 实现。
7. **简单、明确的加工语义**：V1 不引入脚本、正则表达式引擎或通用表达式语言。
8. **全平台一致行为**：Windows、macOS、Linux，以及 x86_64/ARM64 是正式目标。

## 非目标

- 不搬运或兼容旧 TypeScript 实现架构
- 不实现任务编排、工作流图或插件运行时
- 不实现 Web UI
- 不内建完整 daemon/control plane
- 不实现协议级多源帧同步、混合或冲突消解
- 不承诺 UDP 绝对零丢包
- V1 不支持 multicast、TCP OSC、自动 failover 或服务发现
- V1 不实现 Live Link Face binary 到 VMC OSC 等专用协议转换

## 核心概念

### Input

一个本地 UDP 监听 endpoint：

```toml
[[inputs]]
id = "face-a"
bind = "0.0.0.0:11111"
mode = "passthrough"
```

模式：

- `passthrough`：不解析 payload，可转发和录制任意 UDP datagram
- `osc`：验证 OSC，并允许使用 OSC filter/address remap

### Output

一个 UDP 目标 endpoint：

```toml
[[outputs]]
id = "unreal-a"
target = "192.168.10.10:11111"
```

### Mapping

Mapping 是 input 与 output 之间的一条有向连接，是 N:N topology 的基本元素：

```toml
[[mappings]]
id = "face-a-to-unreal-a"
from = "face-a"
to = "unreal-a"
```

每条 mapping 后续可以独立引用：

- filter
- address remap
- recorder tap
- enabled 状态
- 独立统计

配置层表达任意 N:N mapping matrix；启动时将其编译成按 input 索引的邻接表，避免热路径扫描整个矩阵。

### Mapping group

用于减少批量连接的重复配置：

```toml
[[mapping_groups]]
id = "stage-face"
from = ["face-a", "face-b"]
to = ["unreal-a", "unreal-b"]
mode = "all-to-all"
```

V1 计划支持：

- `all-to-all`：组内笛卡尔积
- `zip`：按位置一一连接，长度必须一致

显式 mappings 与 mapping groups 都必须在启动验证阶段展开、去重并编译。

### 多输入汇聚

多个 inputs 可以映射到同一 output。默认语义是独立 UDP datagram 的尽快转发与自然交错，不执行：

- frame synchronization
- application-level merge
- latest-value aggregation
- source priority
- OSC address 冲突消解

## 配置与生命周期

V1 使用 TOML 配置：

```bash
osc-toolkit validate --config router.toml
osc-toolkit run --config router.toml
```

生命周期约束：

- 启动前完整解析和验证配置
- endpoint ID、mapping 引用、端口和加工规则必须全部有效
- 配置失败时不进入部分运行状态
- 运行期间 topology 不可变
- 配置变更由外部编排引擎执行 `validate -> stop -> start`
- 支持 graceful shutdown，确保 recorder 尽可能 flush 并完成有效 chunk

核心内部结构可以保留未来原子替换 mapping table 的能力，但在出现无中断切换的明确需求前，不建设 IPC、HTTP、WebSocket、file watcher 或自有任务管理系统。

## OSC 加工范围

### Filter

V1 支持 address 级 allow/deny：

- exact
- prefix
- OSC address pattern

不支持：

- regex
- 任意参数表达式
- 脚本
- 参数值驱动的 endpoint routing

### Address remap

V1 只修改 OSC address：

```toml
[address_remaps.unreal]
rules = [
  { from = "/old/*", to = "/new/$1" },
  { from = "/foo", to = "/bar" },
]
```

不把 endpoint mapping 与 OSC address remap 混为同一概念。

V1 不支持通用参数数学、类型转换、曲线、拆包或合包。后续只按明确场景增加有限、可预测的操作。

### Bundle

OSC-aware 模式需要安全处理：

- OSC message
- OSC bundle
- nested bundle
- typetag
- timetag

必须配置或内建合理的 packet size、bundle depth、message count、string/blob size 上限，以抵御 malformed 或恶意 packet。

## 录制模型

### 两种运行行为

1. **Record-only**：接收并录制，不转发。
2. **Record-and-forward**：同一数据同时录制和转发。

两种行为共享同一个 recorder 实现，仅 mapping/output 行为不同。

### Tap 位置

录制可以挂载在：

- `input`：记录输入的原始 datagram，一份输入只录一次
- `mapping`：记录某条连接经过加工后的数据
- `output`：记录该 output 最终收到的多输入交错顺序

Tap 位置必须写入录制 metadata，使重放语义明确。

### 新二进制格式

格式名称和扩展名在实现前通过 format specification 确定。基本要求：

- 直接保存 raw UDP datagram，不经过 JSON
- 使用 monotonic timestamp 表示录制内相对时间
- chunk-based sequential append
- 每个 chunk 独立长度和 CRC32C
- 文件未正常关闭时仍可扫描已完成 chunks
- 稀疏 chunk index，支持大文件和时间 seek
- final index/footer 只用于加速，不是可读性的必要条件
- 支持 `inspect`、`verify`、`repair` 和 replay
- V1 默认不压缩；未来压缩仅作为可选 chunk codec

建议逻辑结构：

```text
File Header
Metadata
Data Chunk 0
Data Chunk 1
...
Optional Final Index
Footer
```

每条记录至少包含：

- 相对 timestamp
- stream/input/mapping/output 标识（取决于 tap）
- payload length
- raw datagram payload

是否记录 source socket address 作为格式能力保留，并由配置决定，避免无需求时增加每 packet 开销。

### Backpressure

Recorder 使用有界队列和独立 writer：

```text
UDP receive -> forwarding
            -> bounded recorder queue -> sequential writer
```

策略必须显式且可观测：

- record-and-forward 默认转发优先；writer queue 满时继续转发、丢弃录制副本并计数
- record-only 默认完整录制优先，但仍需明确 UDP/kernel buffer 可能丢包
- 可配置 `drop-recording`、`block` 或 `fail`，且配置验证应提示风险

禁止宣称 UDP 录制绝对完整。必须输出 received、forwarded、written、application-dropped、invalid 等统计；kernel drop 仅在平台可可靠获取时报告。

## 重放

重放需要支持：

- 原始时间间隔重放
- speed multiplier
- start/end/seek
- loop
- 指定或覆盖输出 endpoint
- 按 stream/tap 选择数据
- as-fast-as-possible 模式
- graceful pause/stop（若单次 CLI 使用场景需要）

对 raw recording 直接发送 payload；除非用户显式启用 OSC filter/remap，否则不 decode/re-encode。

## 性能架构

### 热路径

```text
recv datagram
  -> lookup compiled edges for InputId
  -> optional OSC parse/filter/remap
  -> send shared payload to N outputs
  -> optional recorder enqueue
```

约束：

- 配置字符串仅在启动阶段解析
- runtime 使用整数 ID 和预编译规则
- transparent forwarding 不解析 OSC
- N 个 outputs 共享 payload，避免每目标复制
- filter 尽可能使用 borrowed view
- remap 仅在实际命中时生成新 payload
- recorder 和网络队列全部有界
- packet 级日志默认关闭，错误日志限速

### Runtime model

优先采用简单、可预测的跨平台异步 I/O 模型。网络接收、发送、recorder writer 和 shutdown coordination 分离，但避免每 packet 不必要的跨线程跳转和动态分派。

在 benchmark 证明需要前，不引入复杂 worker pool。

## CLI 草案

```bash
osc-toolkit validate --config router.toml
osc-toolkit run --config router.toml

osc-toolkit relay \
  --listen 0.0.0.0:9000 \
  --send 192.168.1.10:8000 \
  --send 192.168.1.11:8000

osc-toolkit record \
  --listen 0.0.0.0:9000 \
  --output session.oscr

osc-toolkit replay session.oscr \
  --target 127.0.0.1:8000 \
  --speed 1.0

osc-toolkit inspect session.oscr
osc-toolkit verify session.oscr
osc-toolkit repair session.oscr
```

简单单 input 场景可以使用命令参数；N:N topology 和加工规则统一使用配置文件。

## 可观测性

运行时至少维护：

- per-input received packets/bytes
- per-mapping forwarded/filtered/dropped packets/bytes
- per-output sent packets/bytes/errors
- per-recorder enqueued/written/dropped packets/bytes
- invalid OSC packets
- queue high-water marks
- uptime 和配置 fingerprint

V1 可以通过周期性结构化日志和退出摘要输出，不要求常驻 metrics server。

## 跨平台发布目标

正式目标：

- Windows x86_64 / ARM64
- macOS x86_64 / ARM64
- Linux x86_64 / ARM64

优先构建目标：

```text
x86_64-pc-windows-msvc
aarch64-pc-windows-msvc
x86_64-apple-darwin
aarch64-apple-darwin
x86_64-unknown-linux-gnu
aarch64-unknown-linux-gnu
```

后续评估 Linux musl 静态构建。每个 release 应包含 checksum；CI 执行单元测试、格式测试、跨平台构建和核心集成测试。

## 实施阶段

### Phase 0 — 规格与基准

- 确定配置 schema 和错误模型
- 定义 N:N mapping 展开、去重和顺序语义
- 定义 buffer/queue/backpressure 策略
- 定义 benchmark 方法与测试流量
- 编写录制格式 specification 和兼容性测试向量
- 选择或验证 OSC codec；若第三方 crate 无法满足安全/分配目标，再实现受控 codec

**退出条件：** 配置、mapping 和 recording format 的关键语义没有歧义，benchmark 可重复。

### Phase 1 — N:N 高速 UDP 数据平面

- Rust CLI skeleton
- TOML load/validate
- UDP IPv4/IPv6 unicast
- N inputs、N outputs、arbitrary N:N mappings
- explicit mappings、`all-to-all`、`zip`
- raw passthrough
- bounded buffers
- graceful shutdown
- 结构化统计和退出摘要
- 跨平台 CI artifacts

**退出条件：** 在目标 benchmark 中长时间运行，内存保持有界；N:N forwarding、错误隔离和 shutdown 集成测试通过。

### Phase 2 — 二进制录制与重放

- format reader/writer
- input/mapping/output taps
- record-only 和 record-and-forward
- chunk CRC32C、recovery、sparse index
- inspect/verify/repair
- timed replay、speed、seek、loop
- 大文件、磁盘变慢、queue overflow、进程强制终止测试

**退出条件：** 正常和异常关闭文件均可恢复；重放顺序与时间误差满足已定义指标；所有队列保持有界。

### Phase 3 — OSC-aware 加工

- 安全 OSC parser/encoder
- messages、bundles、nested bundles、typetags、timetags
- allow/deny filters
- exact/prefix/OSC address pattern
- address remap
- malformed packet 策略与限制
- decode-on-demand/encode-on-change

**退出条件：** passthrough 性能不因 OSC 功能存在而回退；filter/remap 有协议和 fuzz/property tests。

### Phase 4 — 生产加固

- socket buffer tuning 和平台差异处理
- recorder I/O tuning
- fuzzing 和 malformed corpus
- soak、packet burst、fan-out/convergence 压力测试
- release packaging、checksums、版本升级策略
- 配置 schema/version 兼容策略
- 完整错误码和自动化友好的 JSON 输出

**退出条件：** 全目标平台发布流程稳定，性能、内存、安全和恢复测试达到发布门槛。

### Future — 仅按真实需求推进

以下能力不预先承诺：

- 无中断原子替换 mapping table
- 外部编排引擎专用的最小 IPC/control adapter
- optional multicast transport
- optional compression codecs
- 有限参数 transform
- 专用协议 adapters（例如 Live Link Face binary -> VMC OSC）

任何 future control 能力只提供数据平面所需的最小接口，不复制外部编排引擎职责。

## 首个稳定版本的验收重点

- 任意 N:N mapping 配置能被确定性编译和执行
- 透明转发路径不解析、不重编码 OSC
- fan-out 和多输入 convergence 下内存保持有界
- recorder 磁盘变慢不会产生无限内存增长
- 异常终止后的录制文件可以恢复到最后一个有效 chunk
- 同一录制可按明确 stream/tap 语义重放
- malformed OSC 不导致进程崩溃或越界读取
- Windows、macOS、Linux 的 x86_64/ARM64 构建和核心测试可重复
- CLI 错误、统计和退出码可供外部编排引擎可靠消费
