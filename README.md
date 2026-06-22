# Linux Driver → Asterinas 自动翻译工作流

基于 Claude Code Workflow，将 Linux 内核网卡驱动自动翻译为 Asterinas（Rust OS 内核）可用的安全 Rust 驱动。

## 工作流文件

| 文件 | 说明 |
|------|------|
| `.claude/workflow/linux-driver-to-asterinas.js` | 主工作流（v3，最新） |
| `.claude/workflow/linux-driver-to-asterinas-v2.js` | v2 版本 |
| `.claude/workflow/linux-driver-to-asterinas-v1.js` | v1 版本 |

## 使用方法

### 前置条件

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) CLI 已安装并登录
- Asterinas 项目已在 Docker 容器中运行（参见 [Building and Running](#)）
- Linux 驱动源码已放在项目根目录下（如 `linux-e1000/`）

### 运行工作流

在 Claude Code 中，直接用自然语言描述任务即可触发工作流，例如：

```
帮我翻译 ./linux-r8169/drivers/net/ethernet/realtek 这个 Linux 驱动
```

Claude Code 会自动调用 `.claude/workflow/` 下的工作流脚本，传入驱动源码路径作为参数。工作流会自动：

1. 分析该目录下的 C 源码，提取驱动名、PCI ID、模块结构
2. 按依赖关系并行翻译为 Rust 模块
3. 组装 crate（`Cargo.toml` + `lib.rs`）
4. 编译验证并自动修复错误
5. 接入内核构建系统和网络栈
6. 审查代码正确性和集成兼容性
7. 启动 QEMU 测试网络连通性

### 工作流输出

- 翻译后的 Rust crate 位于 `kernel/comps/<driver_name>/`
- Review 报告位于 `kernel/comps/<driver_name>/REVIEW-*.md`
- 内核集成改动会直接修改 `Cargo.toml`、`Components.toml`、`init.rs`、`mod.rs`、`common.rs` 等文件

## 工作流阶段

1. **Discover** — 一个 agent 分析 Linux 驱动源码，输出结构化信息（驱动名、PCI ID、目标芯片、文件角色分类、Rust 模块拆分计划及依赖关系）。使用 JSON schema 强制输出格式。
2. **Translate** — 基于 Discover 阶段规划的模块依赖图，按波次并行翻译。无依赖的模块同一波并行执行，有依赖的等前序完成后再启动。如果出现循环依赖则退化为顺序执行。每个 agent 只负责翻译一个 `.rs` 模块。
3. **Assemble** — 读取所有已翻译的模块文件，统一编写 `Cargo.toml` 和 `lib.rs`，确保导入路径一致、init 注册正确。
4. **Compile** — 运行 `cargo check` 验证编译，失败则由修复 agent 根据错误信息修正代码，最多重试 3 次。
5. **Integrate** — 单个 agent 把新 crate 接入内核构建和网络栈，修改 5 个文件：根 `Cargo.toml` 加 workspace member、`kernel/Cargo.toml` 加依赖、`init.rs` 加网络接口初始化、`mod.rs` 加导出、`common.rs` 加 fallback。
6. **Review** — 两个并行 agent 分别做正确性审查和集成审查。
7. **Test** — 启动 QEMU（`NIC=<driver>`），配 DNS，`wget bing.com`，失败则自动修复（最多 3 次）。

## 已完成的驱动

### e1000

- **目标芯片**：82540EM
- **Crate**：`aster-e1000`
- **路径**：`kernel/comps/e1000/`
- **Review 报告**：`kernel/comps/e1000/REVIEW-correctness.md`、`kernel/comps/e1000/REVIEW-integration.md`
- **统计**：5 个 agent，约 57 万 token，耗时约 35 分钟

### e1000e

- **目标芯片**：82574L
- **Crate**：`aster-e1000e`
- **路径**：`kernel/comps/e1000e/`
- **Review 报告**：`kernel/comps/e1000e/REVIEW-integration.md`
- **统计**：翻译了 8 个模块（regs, phy, nvm, mac, desc, tx, rx, driver），21 个 agent，约 250 万 token，耗时约 2 小时 15 分钟

### r8169

- **Crate**：`aster-r8169`
- **路径**：`kernel/comps/r8169/`
- **Review 报告**：`kernel/comps/r8169/REVIEW-correctness.md`、`kernel/comps/r8169/REVIEW-integration.md`

## 代码规模与功能裁剪

翻译后的 Rust 代码相比 Linux 原版 C 代码规模大幅缩减：

| 驱动 | Linux 原版（仅驱动自身） | Rust 翻译版 | 缩减比例 |
|------|--------------------------|-------------|----------|
| e1000 | ~17,000 行 | ~2,600 行 | ~15% |
| e1000e | ~30,000 行 | ~1,960 行 | ~6.5% |

> 注意：Linux 源码目录中包含整个 `drivers/net/` 子树（virtio_net、macsec、tun 等几十个无关驱动），上表仅统计了 `drivers/net/ethernet/intel/e1000*/` 下的实际驱动代码。

缩减主要来自以下功能裁剪，而非翻译丢失：

1. **ethtool 实现** — Linux 驱动包含完整的 ethtool 接口（e1000: ~1,900 行，e1000e: ~2,400 行），用于查询/配置链路状态、寄存器转储等。Asterinas 暂无 ethtool 框架，未翻译。
2. **模块参数（param.c）** — Linux 驱动支持通过内核命令行传递参数（如中断限制、Tx/Rx 描述符数量等），Asterinas 不使用此机制。
3. **PTP 硬件时间戳** — e1000e 包含 `ptp.c`（~355 行）实现精确时间协议，当前未翻译。
4. **管理固件接口（manage.c）** — e1000e 的管理引擎交互代码（~329 行），当前未翻译。
5. **电源管理** — Wake-on-LAN、suspend/resume 等 ACPI 相关逻辑未翻译。
6. **多芯片变体适配** — Linux e1000e 驱动支持 82571/82573/80003es2lan/ich8lan 等多种 MAC 系列，翻译仅针对 82574L 单一芯片。
7. **调试/诊断/trace 基础设施** — Linux 的 dev_info/dev_err 日志、ftrace、ethtool 寄存器转储等未翻译。
8. **Rust 表达力** — enum、match、derive 宏、trait 等语言特性比 C 的 switch-case/函数指针更紧凑。
9. **Asterinas 框架代劳** — OSTD 的 `NetDevice` trait 等已提供收发框架，驱动无需像 Linux 那样实现 net_device_ops 的全套样板代码。

如果只保留核心收发路径和硬件操作（去掉 ethtool、PTP、param、manage、多芯片适配），Linux 原版大约 1-2 万行，与 Rust 版的差距并不悬殊。

## 源代码改动（以 e1000 为例）

1. `tools/qemu_args.sh` — 加了 `NIC` 环境变量支持，默认 `virtio-net-pci`，设 `NIC=e1000` 即可切换
2. `Components.toml` — 注册了 `aster-e1000` 和 `aster-r8169`
3. `kernel/comps/e1000/Cargo.toml` — 加了缺失的 `zerocopy` 依赖
4. `kernel/comps/e1000/src/intr.rs` — 移除了 `bitflags` 的重复 derive
5. `kernel/comps/e1000/src/driver.rs` — 加了 probe 成功和 MAC 地址的 info 日志

## 测试方法

```bash
# 使用 e1000 网卡启动内核
NIC=e1000 make run_kernel VNC_PORT=27

# 在 Asterinas 内配置 DNS（先在 Docker 容器里查看 DNS 地址）
cat /etc/resolv.conf

# 将 nameserver 写入 Asterinas 的 resolv.conf
echo 'nameserver 127.0.0.53' > /etc/resolv.conf

# 测试网络连通性
wget bing.com
```

## 其他目录

| 目录 | 说明 |
|------|------|
| `linux-e1000/` | e1000 Linux 原始驱动源码（未纳入 git） |
| `linux-e1000e/` | e1000e Linux 原始驱动源码（未纳入 git） |
| `linux-r8169/` | r8169 Linux 原始驱动源码（未纳入 git） |
| `test/initramfs/src/test_e1000_net.sh` | e1000 网络测试脚本 |

## 版权声明

本仓库基于以下开源项目构建：

- **Asterinas** — https://github.com/asterinas/asterinas
  采用 MPL-2.0 许可证，原始版权归属 Asterinas 项目及其贡献者。

- **Linux Kernel** — https://github.com/torvalds/linux
  翻译的网卡驱动（e1000、e1000e、r8169）源自 Linux 内核源码，采用 GPL-2.0-only 许可证，原始版权归属 respective authors。

本仓库中的工作流代码及新增内容遵循各自上游项目的许可证条款。