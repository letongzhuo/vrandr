# vrandr(中文说明)

一个轻量级的 **Vim 风格 TUI**,用来操作 [`xrandr`](https://www.x.org/wiki/Projects/XRandR/),
基于 [Ratatui](https://github.com/ratatui-org/ratatui) 和
[crossterm](https://github.com/crossterm-rs/crossterm) 构建。

`vrandr` 让你快速调整显示器布局:选中一个输出,逐步暂存想要的修改,按 `a`
一次性提交;或者按 `W` 把当前布局保存成命名配置,以后用 `T` 一键恢复。

```
┌────────────────────────────────────────────────────────────┐
│  Outputs                               Modes              │
│  > eDP-1 * 1920x1080 +                 1920x1080  60.00Hz  │
│    HDMI-1  1920x1080 (暂存修改)         1920x1080  59.94Hz  │
│    DP-1    off                         1680x1050  59.95Hz  │
├────────────────────────────────────────────────────────────┤
│  状态栏: 当前选中输出, 1 个暂存操作, [?] 帮助              │
│  预览: xrandr --output HDMI-1 --mode 1920x1080 ...         │
└────────────────────────────────────────────────────────────┘
```

输出名右侧的 `+` 表示已有暂存修改。底部状态栏始终显示按下 `a` 时
**真正**会执行的 `xrandr` 命令。

## 环境要求

- Linux + X11
- `$PATH` 中能找到 `xrandr`
- Rust 1.70+(edition 2021)

## 安装

### 从源码编译

```sh
cargo build --release
./target/release/vrandr
```

### 静态可移植构建

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
./target/x86_64-unknown-linux-musl/release/vrandr
```

`Cargo.toml` 的 release 配置开启了 `lto = true`、`opt-level = "z"`、`strip = true`、
`panic = "abort"`,最终二进制体积小(通常 < 2 MB)且只依赖 `libc`。

## 使用

在终端里运行 `vrandr`,所有操作都通过键盘完成。运行后按 `?` 可以看到
按主题分组的完整快捷键列表。

### 工作流

1. 用 `j` / `k`(或方向键)移动光标;用 `h` / `l` / `Tab` 在 **outputs**
   列表和 **modes** 列表之间切换焦点。
2. 为当前选中输出暂存修改,例如:按 `Enter` 暂存当前高亮分辨率,`R`
   循环切换旋转,`d` 关闭输出,`H` / `L` / `K` / `J` 把它放到邻居的上/
   下/左/右边,`s` / `g` / `b` 弹窗输入精确的缩放 / Gamma / 亮度等。
3. 底部状态栏会实时预览即将执行的 `xrandr` 命令。
4. 按 `a` 把**所有**暂存修改拼成一条 `xrandr` 命令一次性应用;随时按
   `r` 可以丢弃整个暂存区。
5. 按 `W` 把当前状态(已应用暂存)导出为命名配置,文件位于
   `~/.config/vrandr/profiles/<name>.toml`;按 `T` 可以列出、应用或删除
   已保存的配置。

### 命名配置

配置文件路径:

```
$XDG_CONFIG_HOME/vrandr/profiles/<name>.toml
```

如果未设置 `XDG_CONFIG_HOME`,则使用 `~/.config/vrandr/profiles/<name>.toml`。
配置文件名允许字母、数字、`_`、`-`、`.`。文件是普通 TOML:

```toml
version = 1

[[outputs]]
name = "eDP-1"
mode = "1920x1080"
rate = 60.0
primary = true
rotation = "normal"
reflection = "normal"
scale = [1.0, 1.0]
position = [0, 0]
off = false

[[outputs]]
name = "HDMI-1"
mode = "1920x1080"
rate = 59.94
relative = { to = "eDP-1", pos = "right-of" }
off = false
```

应用配置时,当前是 `disconnected` 的输出会被自动跳过,不会因为空端口而
触发 `xrandr` 报错。

### 许可

MIT。
