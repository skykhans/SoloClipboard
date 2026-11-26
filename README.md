# Solo Clipboard

一个基于 Rust 和 GPUI 的 Windows 剪贴板管理工具，自动捕获文本和图片，并将历史记录保存在本地。

## 使用说明

- **打开与粘贴**。按默认全局快捷键 `Win + V` 或点击通知区域图标，历史窗口会以原尺寸置顶显示，任务栏不显示窗口按钮。先在目标应用中定位输入位置，打开历史窗口后点击一条记录即可粘贴。按 `Esc` 隐藏窗口。
- **查找记录**。在顶部搜索框输入关键词。历史记录按日期倒序分组，时间显示到秒。滚动时当前日期固定在搜索栏下方，点击日期行可以跳转到指定日期。
- **管理记录**。置顶的记录显示在当天分组的最前面。点击文本记录旁的「编辑」可以修改并保存多行文本。可以单独删除记录，也可以点「批量删除」，选中记录后点「删除选中」。
- **按日期删除**。点击工具栏的「按日期删除」，在日历中选一个有记录的日期。弹窗会显示当天的记录条数，确认后删除当天的全部记录，包含置顶记录。
- **导入与导出**。使用工具栏的「导入」或「导出」读写 JSON 历史文件。导入会合并历史并跳过重复记录。操作成功后，提示在 3 秒后自动消失。操作失败时，提示会保留。
- **设置**。可以开启开机启动、设置关闭窗口时退出程序或最小化到通知区域，也可以更改或禁用全局快捷键。在「剪贴板历史」中点击「清空全部」，确认后删除全部历史。

## 编译与运行

- Windows 10/11
- rustup，工具链版本由 `rust-toolchain.toml` 固定为 `1.100.0`
- Visual Studio Build Tools 的 C++ 工具链和 Windows SDK
- Zed 源码放在 `SoloClipboard` 的同级目录 `zed` 下，包含 `crates/gpui` 和 `crates/gpui_windows`。[Zed 源码仓库](https://github.com/zed-industries/zed)

在 `SoloClipboard` 仓库根目录构建并运行发布版。

```powershell
cargo build --locked --release
.\target\release\solo_clipboard.exe
```

开发时可用 `cargo run --locked` 启动调试版。首次构建需要编译 GPUI 及其依赖。

## 数据存储

历史记录和设置会自动保存到 `%LOCALAPPDATA%\SoloClipboard\`。

- `history.json` 保存剪贴板历史，包含文本和图片。
- `settings.json` 保存全局快捷键和关闭窗口时的行为。
- 开机启动设置保存在当前用户的 Windows 注册表中。

## 项目结构

```text
src/
├── main.rs       # 应用入口
├── clipboard.rs  # 剪贴板监控和读写
├── desktop.rs    # 通知区域、快捷键、开机启动和粘贴
├── instance.rs   # 单实例管理
├── search.rs     # 文本输入与输入法支持
├── storage.rs    # 历史和设置的 JSON 存储
└── ui.rs         # 历史列表、工具栏和设置界面
```

## 许可证

MIT License

## 贡献

欢迎提交 Issue 和 Pull Request！
