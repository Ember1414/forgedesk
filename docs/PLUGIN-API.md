# ForgeDesk 插件 API（PLUGIN-API）

> 面向插件开发者的宿主接口规范。版本：**apiVersion `0.1`**。
> **1.0 之前不保证任何兼容性**：MINOR 升级只新增（不改签名、不删除），
> MAJOR 不匹配的插件拒绝加载（清单校验在加载时执行）。
>
> 三个官方示例插件（T6.5）落地后，本文档将以它们为主线补充完整源码解读。

## 1. 清单（plugin.json）

```json
{
  "id": "com.example.my-plugin",
  "name": "My Plugin",
  "version": "1.0.0",
  "apiVersion": "0.1",
  "author": "you",
  "license": "MIT",
  "description": "One-line description.",
  "homepage": "https://github.com/you/my-plugin",
  "main": "plugin.wasm",
  "permissions": ["git:read", "ui:command", "ui:toast"],
  "contributes": {
    "commands": [{ "id": "fill-template", "title": "Fill from template" }],
    "panels": [{ "id": "stats", "title": "Stats", "location": "sidebar" }]
  }
}
```

校验规则（加载时强制）：

- `id`：反向域名风格（≥2 个小写标签，`[a-z0-9-]`），全应用唯一；
- `version`：SemVer（`X.Y.Z`，可带 `-prerelease`）；
- `apiVersion`：MAJOR 必须等于 `0`；MINOR 必须不大于宿主支持的 `1`
  （插件声明更新次版本 = 可能调用了宿主没有的函数 → 拒绝加载）；
- `main`：只能是插件目录内的普通 `*.wasm` 文件名（禁止任何路径分隔符与 `..`）；
- `permissions`：只能取自 §2 的白名单，白名单外**整份清单被拒绝**；
- 未知字段一律拒绝（`deny_unknown_fields`）；
- `contributes.*.id`：kebab 风格，全名由宿主拼为 `<plugin-id>.<id>`。

正式 JSON Schema 可用 `schemars` 从本仓库 `crates/plugin-host` 的
`PluginManifest` 类型生成（T6.1）。

## 2. 权限白名单

| 权限 | 说明（授权对话框逐项展示的语义） | 危险级 |
| --- | --- | --- |
| `fs:read` | 只读访问你显式打开的仓库中的文件 | |
| `fs:write` | 修改你显式打开的仓库中的文件 | ⚠ 额外确认 |
| `git:read` | 读取仓库状态与历史 | |
| `git:write` | 执行改变仓库状态的操作（经快照 + 审计，界面标注"由插件 X 执行"） | ⚠ 额外确认 |
| `net:github` | 通过宿主 HTTP 层访问 api.github.com（走你的代理与限流） | ⚠ 额外确认 |
| `ui:panel` | 在界面注册面板 | |
| `ui:command` | 向命令面板注册命令 | |
| `ui:toast` | 弹出通知 | |
| `settings:read` | 读取本插件命名空间下的设置 | |
| `settings:write` | 写入本插件命名空间下的设置 | |

要点：

- **白名单封闭**：清单请求白名单外的权限 → 拒绝加载；
- **授权以用户为准**：清单声明 ∩ 用户逐项授权 = 实际生效权限（T6.4）；
  用户撤销授权后，下一次调用立即失败（`PERMISSION_DENIED`）；
- **每次调用都校验**：派发器是唯一权限裁决点，无旁路。

## 3. 插件模块的导出与导入

### 3.1 必需导出

| 导出 | 签名 | 说明 |
| --- | --- | --- |
| `memory` | memory | 插件线性内存（宿主经它与插件交换数据） |
| `fd_alloc(len: i32) -> i32` | function | 在插件内存中划出可写区域，返回指针 |
| `fd_invoke(ptr: i32, len: i32) -> i64` | function | 命令入口；参数/返回值约定见 §4.3 |

### 3.2 可选导出

| 导出 | 签名 | 说明 |
| --- | --- | --- |
| `fd_activate() -> i32` | function | 激活钩子；缺失 = 直接激活成功 |
| `fd_deactivate() -> i32` | function | 停用钩子；此处 trap 不会上抛（停用总是成功） |

### 3.3 宿主导入（模块名 `fd`）

| 导入 | 签名 | 权限 |
| --- | --- | --- |
| `fd.log` | `(level: i32, msg_ptr: i32, msg_len: i32)` | 无（诊断通道） |
| `fd.host_call` | `(op: i32, arg_ptr: i32, arg_len: i32) -> i32` | 按 op（§4.1） |
| `fd.host_result` | `(out_ptr: i32, out_cap: i32) -> i32` | 无 |

- `fd.log` 的 `level`：0=debug、1=info、2=warn、3=error。文本 ≤ 8 KiB。
  日志进文件前经过宿主脱敏层（红线 R8：不得把令牌/密码写进日志）。
- `fd.host_call`：`op` 是 §4.1 的操作 id；JSON 入参放在插件内存
  `[arg_ptr, arg_ptr+arg_len)`。返回 0 = 成功（结果在 staging）；
  负值 = 错误码（§5，错误详情 JSON 也在 staging）。
- `fd.host_result`：把 staging 拷进 `[out_ptr, out_ptr+out_cap)`。
  返回写入字节数；缓冲不足返回 `-required_len`（staging 保留，可加大缓冲重试）；
  staging 为空返回 0。

### 3.4 资源限制（T6.1）

| 项 | 默认值 | 超限行为 |
| --- | --- | --- |
| 线性内存 | 64 MiB | `memory.grow` 返回 -1（规范行为） |
| 单次命令执行 | fuel 预算（release 1G）+ 30s 墙钟 | `TIMEOUT` 错误码；实例标记为已崩溃 |
| 宿主函数调用 | 5s 墙钟 | `TIMEOUT` |
| staging/参数大小 | 1 MiB | `TOO_LARGE` |

插件崩溃（trap/超时/超限）**绝不影响宿主**：实例被隔离并标记为已崩溃，
后续调用返回缓存的同一个结构化错误；在插件日志页可查原因（T6.4）。

## 4. 宿主操作（`fd.host_call` 的 op 表）

### 4.1 操作与权限

op id 是 ABI 的一部分：**只增不改**。

| id | 名称 | 权限 | 入参（JSON） | 结果（JSON） |
| --- | --- | --- | --- | --- |
| 1 | `get_repo_info` | `git:read` | `{}` | `{"path","name","currentBranch","isDirty"}` |
| 2 | `get_status` | `git:read` | `{"filter"?}` | `{"entries":[{"path","status"}]}` |
| 3 | `read_file` | `fs:read` | `{"path"}` | `{"content"}`（UTF-8 文本） |
| 4 | `list_dir` | `fs:read` | `{"path"}` | `{"entries":[{"name","kind":"file"\|"dir"}]}` |
| 5 | `write_file` | `fs:write` | `{"path","content"}` | `{}` |
| 6 | `http_get_json` | `net:github` | `{"url","headers"?}` | `{"status","body"}` |
| 7 | `get_setting` | `settings:read` | `{"key"}` | `{"value": any\|null}` |
| 8 | `set_setting` | `settings:write` | `{"key","value"}` | `{}` |
| 9 | `get_git_log` | `git:read` | `{"limit"(1..=1000),"path"?}` | `{"commits":[{"id","summary","author","time"}]}` |
| 10 | `git_stage` | `git:write` | `{"paths": [..1..100]}` | `{}` |
| 11 | `git_commit` | `git:write` | `{"message"}` | `{"commitId"}` |
| 12 | `register_command` | `ui:command` | `{"id","title","keybinding"?}` | `{}` |
| 13 | `register_panel` | `ui:panel` | `{"id","title","location":"sidebar"\|"bottom"\|"repo-tab"}` | `{}` |
| 14 | `show_toast` | `ui:toast` | `{"level":"info"\|"success"\|"warning"\|"danger","message"}` | `{}` |

### 4.2 关键约束

- **路径**：`read_file`/`list_dir`/`write_file`/`git_stage` 的路径必须是
  仓库相对路径（非空、无 `..` 分量、无盘符/绝对前缀、无 NUL、≤4096 字符）。
  宿主先做形状校验；符号链接逃逸与 canonicalize 由服务实现落盘校验。
- **设置命名空间**：`get_setting`/`set_setting` 的 key 由宿主强制加前缀
  `plugin.<你的插件id>.`，插件读不到其他插件或宿主的设置。
- **HTTP 白名单**：只接受 `https://`；host 必须在白名单内
  （默认仅 `api.github.com`；用户可在设置中显式添加更多域名，T6.4）。
  请求一律走宿主 HTTP 层：自动使用用户的代理配置与限流，插件无法直连 socket
  （WASI 沙箱无网络能力）。
- **写操作审计**：`write_file`/`git_stage`/`git_commit` 全部走宿主的
  快照与审计链路，审计记录标注来源插件；提交在界面标注"由插件 X 执行"。
- **动态注册**：`register_command`/`register_panel` 的 id 会被宿主加前缀
  `<插件id>.<id>`；location 只接受三个白名单值。

### 4.3 命令与 `fd_invoke`

命令面板触发命令（清单 `contributes.commands` 或动态注册）时，宿主调用
`fd_invoke(ptr, len)`：`[ptr, ptr+len)` 是 JSON 参数（当前为 `"{}"`，预留）。
返回值 i64 打包为 `(结果指针 << 32) | 结果长度`，指向插件内存中一段
UTF-8 JSON——宿主读取后交给命令面板展示。返回 `(0 << 32) | 0` 表示无输出。

典型 host_call 流程（伪代码）：

```text
let out = fd_alloc(4096);            // 结果缓冲
let code = fd.host_call(op, arg_ptr, arg_len);
if code != 0 {
    // 错误详情 JSON 在 staging：读出来决定重试/降级/报错
}
let n = fd.host_result(out, 4096);
if n < 0 {
    let required = -n;
    // 缓冲不足：fd_alloc(required) 后重试 host_result
}
```

## 5. 错误码（`fd.host_call` 返回值）

| 码 | 常量 | 含义 |
| --- | --- | --- |
| 0 | `OK` | 成功 |
| -1 | `GENERIC` | 服务内部失败（详情见 staging 错误 JSON） |
| -2 | `PERMISSION_DENIED` | 权限未授予或已被用户撤销 |
| -3 | `INVALID_ARGUMENT` | 参数缺失/类型错/越界/路径非法 |
| -4 | `NOT_FOUND` | 目标不存在（文件、键、未打开仓库等） |
| -5 | `TIMEOUT` | 操作超时（含 fuel 耗尽） |
| -6 | `TOO_LARGE` | 参数/结果超过大小上限 |

错误详情 staging JSON 形如
`{"error": {"code": -3, "message": "invalid argument for `path`: ..."}}`。
`message` 面向开发者（英文）；插件展示给用户的文案应基于错误码自行本地化。

## 6. 生命周期与事件

- **load → activate → 事件驱动 → deactivate**；
- `deactivate` 后实例保留，可再次激活；
- 崩溃的实例不可复活，需重新加载（管理页"重新加载"，T6.4）；
- 事件订阅（`repo_opened` / `repo_changed` / `commit_created` / `sync_completed`）
  在 T6.3 落地：回调限时、异步分发、超时丢弃，绝不阻塞主流程。

## 7. 面板渲染（T6.3 已定案）

插件面板采用**方案 C：声明式 UI DSL**——插件返回 JSON 描述（表格/列表/
文本/进度/按钮），宿主渲染。表达能力让位于安全与样式统一；
HTML/iframe 渲染作为 1.0 之后的 RFC。DSL 具体 schema 随 T6.3 在本节补充。

## 8. 打包与分发

- 目标三元组：`wasm32-wasip1`（无需 WASI 环境即可运行的 freestanding 插件
  目前是推荐形态；宿主导入全部来自 `fd` 模块）；
- 插件目录布局：`plugin.json` + `plugin.wasm`（+ 资源文件）；
- **不做在线插件市场**（不做清单）：插件以"本地目录导入（开发者模式）"
  或打包分发，安装时展示 SHA256 供核对；
- 红线 R6：插件不得作为遥测通道；插件输出进日志前一律过宿主脱敏层。
