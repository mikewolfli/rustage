// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The Simplified Chinese interface pack (roadmap task T24.03).
//!
//! The English pack is *derived* from the identified vocabulary
//! ([`crate::app::locale::vocabulary::entries`]), so it cannot drift from the interface. A
//! translation cannot be derived: the key is the identity and the text is the translation, so this
//! module holds the translated text keyed by the same stable keys and nothing else.
//!
//! Two rules follow from that and are enforced by tests rather than by prose:
//!
//! * **Every identified key is translated.** A key the vocabulary knows and this table does not is a
//!   hole in the menu, dialog or diagnostic a Chinese-reading person sees, so
//!   [`crate::app::locale::vocabulary`]'s tests compare this table against `entries()` in both
//!   directions.
//! * **A placeholder is kept verbatim.** `{id}`, `{name}`, `{reason}`, `{effect}`, `{message}` and
//!   `{version}` are filled by [`crate::app::locale::catalog::Catalog::fill`]; translating the braces
//!   would leave the value unfilled, so a test asserts the placeholder sets match the English ones.
//!
//! The text is interface vocabulary, not the vocabulary of a generated application: T24.04 covers
//! the target application's own resources, and this pack is not a substitute for them.

#![allow(dead_code)]

/// Every identified interface string, in Simplified Chinese, sorted by key.
///
/// The order matches [`crate::app::locale::vocabulary::entries`] so the two tables can be compared
/// position by position as well as by key.
pub const MESSAGES: [(&str, &str); 73] = [
    ("cmd.build.run.category", "构建"),
    ("cmd.build.run.description", "为项目目标构建，一次一个"),
    ("cmd.build.run.title", "运行"),
    ("cmd.edit.redo.category", "编辑"),
    ("cmd.edit.redo.description", "恢复最近撤销的更改"),
    ("cmd.edit.redo.title", "重做"),
    ("cmd.edit.undo.category", "编辑"),
    (
        "cmd.edit.undo.description",
        "撤销最近的更改，限于其所属领域",
    ),
    ("cmd.edit.undo.title", "撤销"),
    ("cmd.file.close.category", "文件"),
    (
        "cmd.file.close.description",
        "关闭当前文档，并决定如何处理未保存的更改",
    ),
    ("cmd.file.close.param.decision.help", "如何处理未保存的更改"),
    ("cmd.file.close.title", "关闭"),
    ("cmd.file.save-all.category", "文件"),
    (
        "cmd.file.save-all.description",
        "写出所有有未保存更改的文档",
    ),
    ("cmd.file.save-all.title", "全部保存"),
    ("cmd.file.save-as.category", "文件"),
    ("cmd.file.save-as.description", "将文档写到另一个文件"),
    ("cmd.file.save-as.title", "另存为"),
    ("cmd.file.save.category", "文件"),
    ("cmd.file.save.description", "将当前文档写回磁盘"),
    ("cmd.file.save.title", "保存"),
    ("cmd.keys.show.category", "视图"),
    ("cmd.keys.show.description", "显示键盘快捷键"),
    ("cmd.keys.show.title", "键盘快捷键"),
    ("cmd.panel.close.category", "视图"),
    ("cmd.panel.close.description", "用键盘隐藏面板"),
    ("cmd.panel.close.title", "关闭面板"),
    ("cmd.tree.activate.category", "视图"),
    ("cmd.tree.activate.description", "打开选中的树项目"),
    ("cmd.tree.activate.title", "激活"),
    ("cmd.view.palette.category", "视图"),
    ("cmd.view.palette.description", "从键盘查找命令"),
    ("cmd.view.palette.title", "命令面板"),
    ("cmd.view.welcome.category", "视图"),
    ("cmd.view.welcome.description", "显示欢迎页"),
    ("cmd.view.welcome.title", "欢迎"),
    ("diag.command.bad-argument", "{id}：{name} 无效：{reason}"),
    ("diag.command.disabled", "{id} 不可用：{reason}"),
    ("diag.command.missing", "{id} 需要一个 {name} 参数"),
    ("diag.command.no-such-command", "没有命令 {id}"),
    ("diag.command.not-an-id", "{id} 不是命令 ID"),
    (
        "diag.command.unauthorized",
        "{id} 具有 {effect} 效果，但未获授权",
    ),
    ("diag.command.unknown", "没有命令 {id}"),
    ("menu.build.item.build.run", "运行"),
    ("menu.build.title", "构建"),
    ("menu.edit.item.edit.copy", "复制"),
    ("menu.edit.item.edit.cut", "剪切"),
    ("menu.edit.item.edit.paste", "粘贴"),
    ("menu.edit.item.edit.redo", "重做"),
    ("menu.edit.item.edit.undo", "撤销"),
    ("menu.edit.title", "编辑"),
    ("menu.file.item.file.close", "关闭"),
    ("menu.file.item.file.new-project", "新建项目"),
    ("menu.file.item.file.open-project", "打开项目"),
    ("menu.file.item.file.save", "保存"),
    ("menu.file.item.file.save-all", "全部保存"),
    ("menu.file.item.file.save-as", "另存为"),
    ("menu.file.title", "文件"),
    ("menu.view.item.keys.show", "键盘快捷键"),
    ("menu.view.item.panel.close", "关闭面板"),
    ("menu.view.item.tree.activate", "激活树项目"),
    ("menu.view.item.view.palette", "命令面板"),
    ("menu.view.item.view.welcome", "欢迎"),
    ("menu.view.submenu.panels.item.panel.inspector", "检查器"),
    ("menu.view.submenu.panels.item.panel.outline", "大纲"),
    ("menu.view.submenu.panels.title", "面板"),
    ("menu.view.title", "视图"),
    ("shell.error.prefix", "rustage：{message}"),
    (
        "shell.status.icon-strip",
        "图标条：运行 `rustage --render-icons <directory>` 渲染它。",
    ),
    ("shell.status.not-implemented", "桌面设计器尚未实现。"),
    (
        "shell.status.probes",
        "依赖契约探针：运行 `rustage --probe-apis <directory>`。",
    ),
    ("shell.version", "rustage {version}"),
];
