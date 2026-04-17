# DriveCk macOS 设计文档

## 1. 设计目标

基于 `docs/macos-requirements.md`，为 DriveCk 增加一套 macOS 专用实现，要求：

- 复用 Rust 校验核心
- 不把 GUI/CLI 专用行为放入共享 core
- 将 macOS 设备发现与交互组织放在独立目录
- 让 CLI 与 GUI 共享一套 Swift 领域服务
- 保持现代化、可维护、贴合系统的实现方式

## 2. 总体架构

采用四层结构：

1. **Rust Core 层**
   - 保留校验引擎、报告生成、共享模型。
   - 仅补充 macOS 可用的底层设备访问能力，以及更适合 FFI 的输入接口。

2. **Rust FFI 层**
   - 暴露稳定 C ABI。
   - 输入输出以 JSON 字符串为主，降低 Swift 绑定成本。
   - 负责把 Swift 传入的 `TargetInfo` / `ValidationOptions` 映射到 core。

3. **macOS Shared Swift 层**
   - 负责设备发现、领域模型、FFI 包装、任务状态编排、报告导出。
   - 同时服务 CLI 与 GUI。

4. **macOS Frontend 层**
   - CLI：终端体验、参数解析、确认提示、退出码管理。
   - GUI：SwiftUI 主界面 + AppKit 补充能力。

## 3. 目录设计

新增目录：

```text
docs/
  macos-requirements.md
  macos-design.md

macos/
  DriveCkMac.xcodeproj
  DriveCkMacShared/
  DriveCkMacCLI/
  DriveCkMacApp/
  Scripts/
```

建议职责：

- `DriveCkMacShared/`
  - `Domain/`：Swift 侧模型、状态枚举
  - `Services/`：磁盘发现、FFI 包装、报告保存
  - `Support/`：格式化、帮助文本、桥接工具

- `DriveCkMacCLI/`
  - `main.swift`
  - `CLIParser.swift`
  - `CLIFormatter.swift`

- `DriveCkMacApp/`
  - `DriveCkMacApp.swift`
  - `Features/Devices/`
  - `Features/Validation/`
  - `Components/`
  - `AppKit/`
  - `Assets.xcassets`

- `Scripts/`
  - 构建 Rust FFI 静态库的 Xcode build script

## 4. 关键设计决策

## 4.1 macOS 设备发现放在 Swift 层

### 原因

现有 Rust core 的设备发现逻辑明显依赖平台：

- Linux 依赖 `/sys/block`
- Windows 依赖 Win32 IOCTL

macOS 若继续放入 core，需要引入新的平台发现实现；但本次重点是补齐 **macOS 原生入口**。相比把设备发现强塞进共享层，将其保留在 Swift 层更符合边界划分：

- macOS 设备元数据天然更适合从系统框架获取
- GUI 与 CLI 可以共享同一套发现逻辑
- Rust core 继续聚焦“已知目标的校验”

### 结论

- **设备发现：Swift**
- **设备校验与报告：Rust**

## 4.2 FFI 输入改为支持完整 TargetInfo

现有 FFI 通过路径字符串让 Rust 重新发现目标，这不适合 macOS 方案。需要补一条新的接口：

- Swift 先发现并构造 `TargetInfo`
- Swift 将 `TargetInfo` JSON 传给 FFI
- Rust 直接执行 `validate_target_with_callbacks`

这样做的优点：

- 避免 Rust 重复做 macOS 发现
- CLI 与 GUI 使用同一份设备元数据
- 共享报告格式与状态模型

## 4.3 Rust macOS 仅补底层 I/O

Rust 共享层需要能在 macOS 上打开目标设备并执行：

- `pread`
- `pwrite`
- `fsync` / `fcntl(F_FULLFSYNC)`
- 缓存控制（例如 `F_NOCACHE`）

设计目标不是让 core 承担完整 macOS 设备发现，而是保证：

- 只要外部传入合法 `TargetInfo`
- core 就能在 macOS 上完成一致的校验循环

## 5. Rust 层设计

## 5.1 core 调整

### 5.1.1 平台 I/O 抽象

重构 `platform` 模块：

- 将 Unix 风格的设备读写能力提取为可复用实现
- 让 macOS 可以复用 `pread/pwrite` 风格逻辑
- 针对 macOS 增加 `F_NOCACHE`、`F_FULLFSYNC` 等适配

### 5.1.2 不新增 UI 特化结构

保留共享模型：

- `TargetInfo`
- `ValidationOptions`
- `ValidationReport`
- `ValidationResponse`

必要时只增加：

- JSON 反序列化入口
- 更利于 FFI 的错误封装

不增加：

- GUI ViewModel
- CLI 输出格式对象
- macOS 业务文案

## 5.2 FFI 调整

### 新增能力

建议新增：

- `driveck_ffi_validate_target_json`
  - 输入：包含 `TargetInfo` 与 `ValidationOptions` 的 request JSON、进度回调、取消回调
  - 输出：包含 `response?` 与 `error?` 的 execution-result envelope JSON，用于保留取消或中途失败时的 partial report

- `driveck_ffi_format_report_text_json`
  - 继续复用现有格式化逻辑

保留已有接口以避免影响其它前端，但 macOS 实现优先使用新接口。

### 返回模型

统一 envelope：

```json
{
  "ok": true,
  "data": { ... },
  "error": null
}
```

### 原因

- Swift 处理 JSON 更轻量
- FFI ABI 更稳定
- 调试时更容易直接打印检查

## 6. Swift Shared 层设计

## 6.1 领域模型

Swift 侧建立与 Rust 对应的可编解码模型：

- `DriveTarget`
- `ValidationRunConfiguration`
- `ValidationRunState`
- `ValidationSnapshot`
- `ValidationOutcome`

其中：

- `DriveTarget` 与 Rust `TargetInfo` 对齐
- `ValidationOutcome` 与 Rust `ValidationResponse` 对齐

Swift 侧会补充少量纯展示字段，例如：

- `displayName`
- `subtitle`
- `statusBadges`
- `accentColorToken`

这些字段只在 Swift 层派生，不回写 Rust。

## 6.2 设备发现服务

`DiskDiscoveryService` 负责：

1. 枚举 `/dev/disk*`
2. 使用 DiskArbitration 读取磁盘描述
3. 过滤只保留“外接 / 可移除 / 整盘目标”
4. 聚合子分区挂载状态
5. 生成统一 `DriveTarget`

### 发现策略

- 识别整盘节点，例如 `disk2`
- 对同一物理盘的分区节点汇总挂载状态
- 将执行路径优先映射到更适合原始访问的设备节点（例如 `rdiskN`）
- 用 `MediaName`、协议、容量等构成展示信息

### 错误策略

- 某个设备描述失败时跳过该设备
- 全量失败时向上抛出可解释错误

## 6.3 FFI 绑定服务

`DriveCKFFIBridge` 负责：

- 调用 C ABI
- 管理 C 字符串释放
- 解析 JSON
- 将 progress callback 桥接为 Swift 闭包
- 将 cancel callback 桥接为原子取消信号

### 线程模型

- FFI 调用在后台任务执行
- progress callback 可从后台线程进入
- 回到 Swift 后统一切换到主线程更新 UI

## 6.4 校验编排服务

`ValidationCoordinator` 负责：

- 接收所选设备与配置
- 驱动 FFI 开始校验
- 发出阶段变化
- 汇总进度
- 支持取消
- 生成最终 `ValidationOutcome`

CLI 与 GUI 都调用它，但消费方式不同：

- CLI：同步等待并打印进度
- GUI：通过 observable state 驱动界面

## 6.5 报告服务

`ReportExportService` 负责：

- 通过 FFI 生成文本报告
- GUI 使用 `NSSavePanel` 导出
- CLI 直接写入指定路径

## 7. GUI 设计

## 7.1 窗口结构

使用 `NavigationSplitView`：

- **Sidebar**
  - 设备列表
  - 刷新入口
  - 空状态提示

- **Detail**
  - 顶部摘要卡
  - 安全提示卡
  - 校验控制区
  - 进度区 / 结果区
  - 报告预览区

## 7.2 视图分层

### Sidebar

- `DeviceListView`
- `DeviceRowView`

### Detail 顶部

- `TargetHeroCard`
- `TargetMetadataView`
- `TargetSafetyBanner`

### 运行状态

- `ValidationControlCard`
- `ValidationProgressCard`

### 结果展示

- `ValidationSummaryCard`
- `ValidationMapCard`
- `TimingOverviewCard`
- `ReportPreviewCard`

## 7.3 AppKit 混合点

以下能力用 AppKit 补齐：

- `NSVisualEffectView`：背景材质
- `NSTextView`：报告预览、选中复制、等宽排版
- `NSSavePanel`：报告导出
- `NSWorkspace` 或通知中心：磁盘挂载变化后的刷新触发

## 7.4 动效方案

### 状态切换

- 设备切换：淡入 + 轻微位移
- 校验开始：控制区收束，进度卡出现
- 校验结束：结果卡片依次显现

### 细节反馈

- 设备行 hover：轻微背景高亮
- 主按钮 hover/press：缩放和亮度变化
- 数值更新：使用系统数值过渡动画
- 进度条：渐变填充和轻微高光移动

### 降级策略

- 关闭或减弱长时动画
- 遵循 Reduce Motion

## 7.5 结果可视化

### 24x24 样本图

- 使用固定 24 列网格
- 状态颜色：
  - ok：绿色
  - read error：橙色
  - write error：紫色
  - mismatch：红色
  - restore error：深红
  - untested：灰色

- 支持 hover tooltip：
  - 区域编号
  - 偏移量
  - 状态名

### 时序统计

使用 `Charts` 展示：

- read timings
- write timings

同时保留摘要卡：

- min / median / mean / max
- throughput

## 8. CLI 设计

## 8.1 命令模型

保持与现有 Rust CLI 尽量一致：

- `--list`
- `--yes`
- `--seed`
- `--output`
- `--help`
- `DEVICE`

## 8.2 输出规则

- 设备列表：表格化输出
- 进度：stderr 单行刷新
- 报告：stdout
- 错误：stderr

## 8.3 退出码

- `0`：无异常且未发现失败
- `1`：校验完成但发现问题，或用户取消
- `2`：参数 / FFI / 权限 / 运行错误

## 9. Xcode 工程设计

## 9.1 工程形态

在 `macos/DriveCkMac.xcodeproj` 中建立两个 target：

1. `DriveCkMacApp`：macOS app
2. `driveck-mac-cli`：command line tool

两个 target 共享 `DriveCkMacShared/` 源文件。

## 9.2 Rust 构建集成

通过 Xcode shell script build phase：

1. 检测 `cargo`
2. 构建 `driveck-ffi` 为静态库
3. 将产物输出到固定目录
4. 让 app / cli target 在链接阶段复用该库

推荐产物：

- `libdriveck_ffi.a`

### 原因

- 避免 `.dylib` 的嵌入与运行时查找复杂度
- CLI 与 app 都能直接链接

## 9.3 签名与沙箱

默认配置：

- 不启用 App Sandbox
- 允许用户脚本构建
- 本地开发构建默认关闭强制签名

原因：

- 原始磁盘访问不适合沙箱
- Rust 构建脚本需要自由读写产物目录

## 10. 状态管理设计

GUI 主状态使用单一 observable store，例如 `AppViewModel`，拆分为：

- `devicesState`
- `selectionState`
- `validationState`
- `reportState`
- `alertState`

`validationState` 建议枚举化：

- `idle`
- `preparing`
- `running(phase, current, total)`
- `completed(outcome)`
- `failed(error)`
- `cancelled(partialOutcome?)`

这样能简化视图条件分支，也方便做状态转场动画。

## 11. 错误处理设计

错误统一映射到 Swift 侧 `UserFacingError`：

- `title`
- `message`
- `suggestion`
- `underlying`

映射策略：

- `Permission denied` -> 权限不足，建议使用管理员权限运行
- `mounted` -> 设备已挂载，建议先弹出/卸载分区
- `not implemented` / `ffi decode failed` -> 内部错误，展示底层详情

CLI 使用紧凑文本；GUI 使用 banner / alert / inline message。

## 12. README 调整点

README 需要新增：

- macOS 目录结构
- Rust + Xcode 的前置依赖
- macOS CLI 构建和运行方式
- macOS GUI 构建和运行方式
- 权限与挂载限制说明
- FFI 构建说明

## 13. 风险与取舍

## 13.1 权限

macOS 原始磁盘访问可能需要更高权限。本次不实现特权 helper，采用：

- 明确错误提示
- README 说明
- CLI 优先满足高权限场景

GUI 在权限不足时展示清晰引导，而不是假装成功。

## 13.2 构建环境

Rust 工具链与 Xcode 工具链需要同时存在；若本地缺少 `cargo`，Xcode target 应在构建脚本中给出直观失败信息。

## 13.3 发现精度

DiskArbitration 元数据可能在部分设备上不完整，因此发现层要容忍字段缺失，但不能放松对整盘与挂载状态的判断。

## 14. 实施顺序

1. 编写需求与设计文档
2. 调整 Rust FFI 与 macOS 底层 I/O
3. 建立 `macos/` 工程与共享 Swift 层
4. 实现 CLI
5. 实现 GUI
6. 更新 README

## 15. 完成定义

满足以下条件即可认为设计落地：

1. `macos/` 下有清晰的原生工程结构。
2. CLI 与 GUI 都经过共享 Swift 层接入 Rust。
3. GUI 具备现代 macOS 交互、必要动效和结果可视化。
4. README 与文档能让新开发者理解整体结构与构建方式。
