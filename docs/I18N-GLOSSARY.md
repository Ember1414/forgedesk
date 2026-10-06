# i18n 术语表（I18N-GLOSSARY）

> 中英对照的**术语决定表**。新文案先查此表；要新增术语，改这里并在 PR 里说明。
> 术语一旦定表，所有页面必须一致——同一概念两种译法比翻译错误更糟。

## 核心概念

| 中文 | 英文 | 备注 |
| --- | --- | --- |
| 暂存 | stage / staged | 动词用 stage，状态用 staged；不用"缓存/加入索引" |
| 取消暂存 | unstage | |
| 提交 | commit | 名词动词条同；不翻译 |
| 修改上一提交 | amend | 界面用"修改上一提交（amend）"首次出现带原文 |
| 推送 | push | 不翻译 |
| 拉取 | pull | = fetch + merge；与"抓取"区分 |
| 抓取 | fetch | 只下载不合并 |
| 克隆 | clone | |
| 快照 | snapshot | ForgeDesk 的安全网概念（T1.9），与 git stash 无关 |
| 回滚 | restore / roll back | 快照操作用 restore；泛指恢复用 roll back |
| 撤销 | revert / undo | git revert 用 revert；一般界面动作用 undo |
| 反转提交 | revert commit | |
| 重置 | reset | |
| 储藏 | stash | git stash 官方译法 |
| 变基 | rebase | 不翻译（历史原因：界面曾用"变基"，保留） |
| 拣选 | cherry-pick | |
| 冲突 | conflict | |
| 合并 | merge | |
| 工作区 | working tree / workspace | 目录本身用 working tree；页面名用"工作区" |
| 暂存区 | index / staging area | 面向用户统一说"暂存的更改" |
| 分支 | branch | |
| 标签 | tag | 不翻译 |
| 上游 | upstream | |
| 远端 | remote | |
| 提交图 | commit graph | |
| 泳道 | lane | 提交图里的纵列 |
| 签名 | signing / signed | GPG/SSH 签名 |
| 凭据 | credential | |
| 令牌 | token | |
| 密钥 | key | SSH/GPG 密钥 |
| 诊断 | diagnostics | |
| 插件 | plugin / plug-in | 统一 plugin |
| 权限 | permission | |
| 授权 | grant / authorize | 名词用 grant（一笔授权），动词用 authorize |
| 沙箱 | sandbox | |
| 命令面板 | command palette | |
| 快捷键 | shortcut / keybinding | 文案用 shortcut；设置项名可用 keybinding |
| 布局 | layout | |
| 主题 | theme | |
| 外观 | appearance | |

## 语气与风格约定

1. **英文**：祈使句、无句号、首字母大写仅限句首与专有名词（标题式大小写只用于页面标题）。
2. **中文**：界面文案不用"您"（用"你"）；按钮 2–6 字；提示句完整成句。
3. **错误信息**：中文标题给"发生了什么"，建议给"你能做什么"；不复述技术细节。
4. **数字与单位**：一律经 Intl 格式化；字节数显示 KB/MB（不译成"千字节"）。
5. **不要直译**的地道示例：
   - "Something went wrong" → "出了点问题"（不是"某些东西走错了"）
   - "No plugins installed" → "还没有安装任何插件"（不是"没有插件被安装"）
   - "Are you sure?" → "确定要…吗？"（结合动作具体化）
