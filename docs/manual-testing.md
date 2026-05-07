# 手动测试

终端行为必须保留手动测试流程。自动化测试无法完整覆盖 raw mode、PTY 透明转发、窗口 resize、交互式输入和 TUI 应用交接等行为。

当 Tide 新增或修改任何用户可见的终端行为、模式、命令生命周期能力、Block 操作或 TUI handoff-return 功能时，都要同步更新本文档。

## 测试前准备

先运行静态检查：

```sh
cargo fmt --check
cargo check
cargo test
```

交互测试必须在真实终端模拟器里运行，不要在 IDE output panel 里测试。

建议使用一个可以随时关闭的独立终端 tab。开发阶段如果 raw mode 或屏幕状态异常，可以直接关闭该 tab。

## Milestone 1：透明 zsh Wrapper

目标：`cargo run` 启动 Tide，Tide 启动真实 `zsh`，普通 shell 使用体验应尽量接近正常 zsh。

### 1. 启动 Tide

命令：

```sh
cargo run
```

预期：

- 程序正常启动，没有 panic。
- 出现普通 zsh prompt。
- 目前不应显示 Tide 自己的 UI。
- 在 raw-mode shell session 下，键盘输入表现正常。

### 2. 基础命令透明转发

在 Tide 内运行：

```sh
echo hello
pwd
printf 'a\nb\n'
```

预期：

- 输出表现和普通 zsh 一致。
- 每条命令结束后都能回到 prompt。
- 不应出现额外控制序列或 Tide debug 文本。

### 3. 交互式输入

在 Tide 内运行：

```sh
read name
```

输入：

```text
tide
```

然后运行：

```sh
echo $name
```

预期：

- `read` 能正常接收键盘输入。
- `echo $name` 输出 `tide`。
- Enter、Backspace 和普通文本输入都正常。

### 4. Ctrl-C 处理

在 Tide 内运行：

```sh
sleep 10
```

按 `Ctrl-C`。

预期：

- `sleep` 被中断。
- zsh 回到 prompt。
- Tide 本身不退出。
- 终端仍然可用。

### 5. Ctrl-D 和 exit 处理

如果需要，重新启动 Tide：

```sh
cargo run
```

在 Tide 内分别测试：

```sh
exit
```

以及另一次运行时，在空 prompt 下按 `Ctrl-D`。

预期：

- zsh 退出。
- Tide 退出。
- 外层终端恢复到正常输入模式。
- Tide 退出后继续输入文字，不应卡在 raw mode。

### 6. 窗口 Resize

启动 Tide：

```sh
cargo run
```

在 Tide 内运行：

```sh
stty size
```

调整终端窗口大小后，再运行：

```sh
stty size
```

预期：

- resize 前后输出的 rows / columns 应发生变化。
- resize 后 prompt 渲染仍然正常。
- 程序不 panic。

### 7. 全屏 TUI 透明转发冒烟测试

使用本机已安装的任意命令测试：

```sh
nvim
vim
less Cargo.toml
```

预期：

- TUI 应用正常打开。
- Tide 不绘制 overlay。
- 键盘输入属于 TUI 应用。
- 退出 TUI 后回到 zsh prompt。

Milestone 1 还不会创建 `TuiSession` block，也不会显示 ReturnPanel。这个测试只验证透明转发。

### 8. 退出后的终端恢复

Tide 退出后，在外层 shell 里运行：

```sh
echo ok
stty -a
```

预期：

- 文本输入正常回显。
- Enter 正常换行。
- 终端没有停留在异常 raw-mode 状态。

如果开发过程中终端状态异常，可以运行：

```sh
reset
```

或者：

```sh
stty sane
```

## Milestone 1 已知限制

- 还没有 TUI handoff-return 检测。
- 还没有 ReturnPanel。
- 还没有 AI 功能。

这些都是 Milestone 1 的预期限制，不应当视为测试失败。

## Block Mode 雏形：最近 10 条 Block

目标：Tide 默认保持透明 shell。用户显式按 `Ctrl-X Ctrl-B` 后，进入 alternate-screen Block Mode，浏览当前 Tide session 最近 10 条命令 block。

### 1. 捕获普通命令

启动 Tide：

```sh
cargo run
```

在 Tide 内运行：

```sh
echo one
pwd
false
printf 'line 1\nline 2\n'
```

然后按 `Ctrl-X Ctrl-B`。

预期：

- 进入一个线框包裹的 `Tide Blocks` 页面。
- 能看到刚才执行过的命令。
- 最近执行的命令显示在列表上方。
- `false` 应显示为失败状态。
- 选中 block 后，下方能看到 command、cwd、exit、duration 和 output。

### 2. Block Mode 选择

在 Block Mode 内按：

```text
j
k
```

预期：

- `j` 选择下一条 block。
- `k` 选择上一条 block。
- 选中项变化后，下方详情同步变化。

### 3. 返回透明 shell

在 Block Mode 内按：

```text
Esc
```

或者：

```text
q
```

预期：

- 退出 alternate screen。
- 回到原来的 zsh shell 画面。
- shell 仍然可继续输入命令。

### 4. 最近 10 条限制

在 Tide 内连续运行 12 条简单命令：

```sh
echo 1
echo 2
echo 3
echo 4
echo 5
echo 6
echo 7
echo 8
echo 9
echo 10
echo 11
echo 12
```

然后按 `Ctrl-X Ctrl-B`。

预期：

- Block Mode 中最多显示最近 10 条 block。
- 最旧的 `echo 1` 和 `echo 2` 不再保留。
- `echo 12` 应在列表最上方。

### 5. 当前雏形限制

- Block Mode 暂时只读，只支持选择和查看。
- 暂不支持复制、重跑、保存、删除、AI 解释等操作。
- Block output 只保存在当前 Tide 进程内，退出 Tide 后丢弃。
- 暂不接入数据库或文件日志。
- zsh hook 注入方式仍是早期实现，后续需要继续打磨。

## 回归检查清单

提交任何影响终端行为的变更前，至少确认：

- `cargo fmt --check` 通过。
- `cargo check` 通过。
- `cargo test` 通过。
- `cargo run` 能启动 zsh。
- 基础命令能输出结果并回到 prompt。
- `Ctrl-C` 能中断前台命令，且 Tide 不退出。
- `exit` 或 `Ctrl-D` 能退出 Tide。
- Tide 退出后外层终端输入正常。
- resize 能更新 PTY size。
- 简单全屏 TUI 仍然能以透明转发方式工作。
- `Ctrl-X Ctrl-B` 能进入 Block Mode。
- Block Mode 能浏览最近 10 条命令 block。
- `Esc` 或 `q` 能从 Block Mode 回到透明 shell。
