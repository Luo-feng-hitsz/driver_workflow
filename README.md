# Linux Driver → Asterinas 自动翻译工作流

基于 Claude Code Workflow，将 Linux 内核网卡驱动自动翻译为 Asterinas（Rust OS 内核）可用的安全 Rust 驱动。

## 工作流文件

| 文件 | 说明 |
|------|------|
| `.claude/workflow/linux-driver-to-asterinas.js` | 主工作流（v3，最新） |
| `.claude/workflow/linux-driver-to-asterinas-v2.js` | v2 版本 |
| `.claude/workflow/linux-driver-to-asterinas-v1.js` | v1 版本 |

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
| `e1000-generated/` | e1000 工作流生成的中间产物 |
| `linux-e1000/` | e1000 Linux 原始驱动源码 |
| `linux-e1000e/` | e1000e Linux 原始驱动源码 |
| `linux-r8169/` | r8169 Linux 原始驱动源码 |
| `test/initramfs/src/test_e1000_net.sh` | e1000 网络测试脚本 |