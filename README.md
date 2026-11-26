# Solo Clipboard

一个基于 Rust 和 GPUI 的 Windows 剪贴板管理工具，自动捕获文本和图片，并将历史记录保存在本地。

## 使用说明

- **打开与粘贴**。在目标应用中定位输入位置，按默认全局快捷键 `Win + V` 打开历史窗口。点击一条记录即可粘贴回目标应用。按 `Esc` 隐藏窗口，也可以从通知区域重新打开。
- **查找记录**。在顶部搜索框输入关键词。历史记录按日期倒序分组，时间显示到秒。滚动时当前日期固定在搜索栏下方，点击日期行可以跳转到指定日期。
- **管理记录**。置顶的记录显示在当天分组的最前面。可以单独删除记录，也可以点「批量删除」，选中记录后点「删除选中」。「清空全部」会在确认后删除全部历史。
- **导入与导出**。使用工具栏的「导入」或「导出」读写 JSON 历史文件。导入会合并历史并跳过重复记录。操作成功后，提示在 3 秒后自动消失。操作失败时，提示会保留。
- **设置**。可以开启开机启动、设置关闭窗口时退出程序或最小化到通知区域，也可以更改或禁用全局快捷键。

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
├── search.rs     # 搜索框与输入法支持
├── storage.rs    # 历史和设置的 JSON 存储
└── ui.rs         # 历史列表、工具栏和设置界面
```

## 开发计划

- [ ] 支持富文本剪贴板
- [ ] 添加标签和分类功能
- [ ] 支持剪贴板内容编辑
- [ ] 按时间段批量删除（如删除一周前的记录）
- [ ] 多语言支持

## 许可证

MIT License

## 贡献

欢迎提交 Issue 和 Pull Request！
