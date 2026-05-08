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

## Layered Block Renderer 雏形

目标：Tide 默认显示 Plain View，只渲染 shell 文本层。用户显式按 `Ctrl-B` 后，在同一份 shell 历史上叠加 Block Metadata Layer；按 `Enter` 可内联展开当前 block 的 Detail 信息。Block/Detail 使用 alternate screen 渲染，不会影响主屏显示内容。

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

然后按 `Ctrl-B`。

预期：

- 不进入独立列表页或弹窗。
- 同一份 shell 历史被重新渲染，并在命令输出范围前后插入 block top/bottom metadata line。
- 每条命令显示 block id、command、status、exit code、duration。
- `false` 应显示为失败状态，top border 有 `✗`，bottom metadata 显示 `failed · <exit> · <duration>`。
- 当前选中 block 使用 `╭╮╰╯` 或高亮边框显示，但 body 文本不应整块反色。
- block 左右有外边距，右边框对齐，不贴终端边缘。
- 底部显示简短 footer：`j/k move  Enter detail  g/G top/bottom  q quit`。

### 2. Block View 选择

在 Block View 内按：

```text
j
k
Down
Up
```

预期：

- `j` 选择下一条 block。
- `k` 选择上一条 block。
- 上下方向键也可移动选中 block。
- 选中 block 的 top/bottom metadata line 高亮变化。
- 选中屏幕外的历史 block 时，视口应跟随选中项移动，类似 tmux copy-mode 的历史浏览体验。

### 3. Detail View 内联展开

在 Block View 内按：

```text
Enter
```

预期：

- 不出现弹窗。
- 当前选中 block 的输出之后、bottom metadata line 之前插入 Detail 信息。
- Detail 信息包含 command、cwd、exit code、duration、status、stdout/stderr 摘要和 actions。
- 按 `q` 或 `Esc` 返回 Block View。

### 4. 返回 Plain View（alternate screen 恢复）

在 Block View 内按：

```text
Esc
```

或者：

```text
q
```

预期：

- 回到 Plain View，主屏 zsh prompt 立即恢复正常，没有闪烁。
- prompt 位置正确，光标在 prompt 后。
- prompt 颜色正常，SGR 属性不泄漏（block 选中时的 Reverse 属性不应残留）。
- 不需要额外输入命令来恢复显示。
- 多次 Ctrl-B / q 循环不累计错位。
- 其它 shell 功能（zsh-autosuggestions, zsh-syntax-highlighting, fzf-tab 等）不受破坏。
- 退出后 TUI 应用（vim/nvim/fzf/less）不受影响。

### 5. 历史保留和 viewport

在 Tide 内连续运行多条简单命令：

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

然后按 `Ctrl-B`。

预期：

- BlockStore 不应固定只保留 10 条。
- 屏幕只显示当前 viewport 能容纳的 block。
- `j` / Down 和 `k` / Up 可以移动 selected block，并带动 viewport 滚动。
- `G` 跳到最后一个 block，并恢复 follow-tail。
- `g` 跳到第一个 block，并关闭 follow-tail。
- 少量 block 总高度小于屏幕高度时，应底部对齐，顶部留空。
- 大量 block 超出屏幕高度时，应默认显示最新可见 blocks，最后一个 block 靠近底部。
- viewport 按 visual line 截取，因此屏幕顶部和底部可以显示非选中 block 的半截内容。
- selected block 仍按 block 移动，并且在高度允许时必须完整可见；半截 block 不应成为 selected 高亮。
- 未展开 block 最多显示 `preview_lines` 行，并在超出时提示还有多少行。
- `Enter` 后当前 block 才展开 Detail，并最多显示 `expanded_lines` 行。

### 6. 强制重绘验证

目标：View 模式切换后，屏幕应立刻更新，不留残留 UI。

1. 在 Plain View 下运行几条命令后按 `Ctrl-B` 进入 Block View。
2. 在 Block View 中按 `j`、`k` 选择不同 block。
3. 按 `Enter` 进入 Detail View。
4. 按 `q` 或 `Esc` 返回 Block View，再按 `q` 或 `Esc` 返回 Plain View。

预期：

- 每次切换 View 后，屏幕立刻重绘，不应残留前一模式的 UI 元素。
- 从 Plain 进入 Block View 时，正常显示 block metadata（在 alternate screen 中）。
- 从 Block View 返回 Plain 时，主屏 prompt 立即恢复，没有 block 边框残留、颜色不泄漏。
- 从 Detail 返回 Block View 时，不再显示 Detail 行。
- 退出 Block View 时不应出现白屏闪烁——clear 和所有绘制命令在同一个 `flush()` 中原子化到达终端。

### 7. auto_follow_on_reach_bottom 行为验证

目标：`auto_follow_on_reach_bottom = false`（默认）时，`j` 到达最后一条 block 后不应切到 Tail anchor；新命令出现时 viewport 应保持在原位。

1. 配置中设置 `auto_follow_on_reach_bottom = false`（或保持默认）。
2. 启动 Tide，运行 5-8 条命令。
3. 按 `Ctrl-B` 进入 Block View。
4. 按 `g` 跳到第一条 block（Top anchor）。
5. 多次按 `j` 逐渐向下移动，直到到达最后一条 block。
6. 注意观察 anchor 标记（或判断 scroll behavior：再运行一条新命令，viewport 不应自动跟随新命令）。

预期：

- `j` 到达最后一条 block 后，anchor 保持 Manual（而不是 Tail）。
- 此时运行新命令，viewport 不应自动滚动到新 block。
- 只有按 `G` 时，才会启用 Tail anchor 并跟随新命令。

然后改为 `auto_follow_on_reach_bottom = true` 重复测试：

预期：

- `j` 到达最后一条 block 时 anchor 变成 Tail。
- 新命令出现时 viewport 自动跟随。

### 8. 输入累积和帧率限制验证

目标：快速重复按 `j`/`k` 时，输入被累积，仅在帧间隔到达时刷新屏幕。

1. 进入 Block View，快速重复按 `j` 5-10 次（不需要等待每次渲染）。
2. 观察屏幕更新节奏。

预期：

- 屏幕不会每次按键都重绘（不应出现闪烁）。
- 最多每秒约 60 帧的更新频率（16ms FRAME_DURATION）。
- 最终选中的 block 和 viewport 位置正确。
- 高速移动时，viewport 应连续按 visual line 变化，不应每次强制跳到某个完整 block 顶部。
- selected block 应保持完整可见；上下相邻 block 可以被截断显示。

### 9. 当前雏形限制

- Block View / Detail View 暂时只读，只支持选择和查看。
- 暂不支持复制、重跑、保存、删除、AI 解释等操作。
- Block output 只保存在当前 Tide 进程内，退出 Tide 后丢弃。
- 暂不接入数据库或文件日志。
- Normal / Plain View 当前应为透明 passthrough；Block / Detail View 才使用 Tide renderer 重绘捕获历史。

## zsh integration 手动测试

Tide 不再通过临时 `ZDOTDIR` 注入 hook。用户需要在自己的 `.zshrc` 中 source Tide integration。

测试前确认 `.zshrc` 中存在：

```zsh
source ~/.tide/zsh-integration.zsh
```

需要重点确认：

- `cargo run` 启动时不应 visibly 打印 hook 脚本内容。
- hook 安装命令不应污染 shell history。
- 用户正常 `.zshrc`、prompt 和插件行为不应被破坏。
- powerlevel10k / starship / zsh-autosuggestions / zsh-syntax-highlighting / fzf-tab / atuin / zoxide 等插件行为不应被 Tide 修改。
- 普通命令仍能被捕获为 block。
- `false` 仍能记录为 failed。
- `Ctrl-B` 仍能进入 Block View。
- 如果没有安装 integration，Tide 不应崩溃，但会进入无法捕获 command block 的 degraded mode。

## Block Capture 调试

如果需要确认 block 捕获边界，可以启用 debug 输出：

```sh
TIDE_DEBUG_BLOCKS=1 cargo run
```

在 Tide 内运行：

```sh
echo debug
false
```

预期：

- 每个 command 结束时，会输出一行 `tide block #...` 调试信息。
- 调试信息包含 status、exit、duration、command 和 output_bytes。
- `echo debug` 应显示 `exit=0`。
- `false` 应显示非零退出码和 failed 状态。

这个 debug 输出只用于开发验证。正常使用 Tide 时不要设置 `TIDE_DEBUG_BLOCKS`。

## Hook / Parser 回归点

涉及 zsh hook 或 OSC parser 的修改，需要确认：

- 普通 shell 输出仍然透明显示。
- Tide 自己的 OSC 777 hook 事件不会显示到用户终端。
- 命令里包含分号时仍能被正确记录，例如 `echo hi; pwd`。
- 命令里包含换行时 parser 单元测试仍通过。
- 同一个 PTY chunk 内出现多个 hook 事件时，事件顺序不乱。
- hook 事件被拆成多个 PTY chunk 时，普通输出不会被长时间延迟。

## 全屏程序兼容性冒烟测试

目标：Normal 模式本身是透明 passthrough，因此全屏交互程序不需要白名单也应正常工作。退出后，Tide 仍能通过 zsh marker 记录一次命令执行。

启动 Tide：

```sh
cargo run
```

在 Tide 内测试本机已安装的任意命令：

```sh
vim
nvim
yazi
fzf
less Cargo.toml
top
htop
ssh user@host
lazygit
man tmux
```

预期：

- 程序运行期间画面和输入不被 Block renderer 干扰。
- `Ctrl-B`、`j`、`k`、`Enter`、`Esc` 等按键属于该交互程序本身，不触发 Tide Block View。
- 退出程序后仍停留在透明 Normal View。
- 按 `Ctrl-B` 后能看到对应 command block。
- 如果没有可线性重绘的 captured output，Block View 显示 `no captured text output`。
- Detail View 显示 command、cwd、exit code、duration 和 status。

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
- `Ctrl-B` 能进入 Block View。
- Block View 能在同一 shell 历史上通过 viewport 浏览命令 block。
- `Enter` 能内联展开当前 block 的 Detail View。
- `Esc` 或 `q` 能从 Detail View 回到 Block View，再从 Block View 回到 Plain View。
- `vim` / `nvim` / `yazi` / `fzf` / `less` 等全屏程序运行期间应 passthrough，退出后仍回到 Normal View。
- 修改 hook / parser 后，`cargo test` 中的 parser 测试全部通过。
