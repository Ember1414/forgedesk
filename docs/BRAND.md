# ForgeDesk 品牌与视觉规范

> 里程碑：M0 / T0.3 ｜ 最后更新：2026-10-07
> 本文件是品牌资产的**唯一说明来源**。修改图标必须先改本文件与 `docs/brand/icon-source.svg`。

---

## 1. 命名与描述语（强制统一）

| 项目 | 内容 |
| --- | --- |
| 产品名 | **ForgeDesk** |
| 中文名 | 对外统一使用英文名 ForgeDesk（不使用中文译名） |
| 完整描述语 | `ForgeDesk — A Git client for everyone` |
| 中文描述语 | `ForgeDesk —— 一款面向所有人的 Git 桌面客户端` |
| 兼容描述语 | `ForgeDesk for Git` |
| 一句话卖点 | 「看得见的 Git，不只是命令行。」 |
| 包名 / 标识符 | npm `forgedesk`；Tauri identifier `org.forgedesk.app`（待 §15.6 最终确认） |

**硬性约束**：产品名、包名、窗口标题、安装包显示名中**不得**出现 `Git` 或 `GitHub` 字样（作为产品名的一部分）。
允许且必须在描述语句中使用 "A Git client"，例如 `ForgeDesk — A Git client`。

---

## 2. 图标设计概念

### 2.1 隐喻

**抽象「锻炉 / 砧台」+ 上方飞溅的火花。**

- 「锻造（Forge）」对应产品名的 Forge，表达"把原始材料加工成可用之物"——
  隐喻把杂乱的 Git 操作加工成清晰的工作流。
- 火花表达动作与成形的瞬间，赋予图标动感，避免沦为静态的工具符号。

### 2.2 几何构成（全部为基础图形的组合）

| 部件 | 几何形式 | 含义 |
| --- | --- | --- |
| 底板 | 圆角矩形 820×820，圆角 186，占画布 80% | 应用容器 |
| 砧面 | 圆角矩形 512×104，圆角 30 | 工作台面 |
| 腰身 | 圆角矩形 160×120，圆角 16 | 收窄的支撑 |
| 底座 | 圆角矩形 384×88，圆角 26 | 稳定的根基 |
| 火花（大） | 菱形 M512 198 → 560 262 → 512 326 → 464 262 | 主火花 |
| 火花（小 ×2） | 菱形 ±112px 偏移，尺寸约 1/2 | 飞溅火花 |

### 2.3 色彩

| 用途 | 值 | 来源 |
| --- | --- | --- |
| 底板渐变起点 | `#4F46E5`（靛蓝） | 与 `--fd-brand`（`#4F46E5`）一致 |
| 底板渐变中点 | `#5B3FE0` | 过渡色 |
| 底板渐变终点 | `#7C3AED`（紫罗兰） | 品牌辅助色 |
| 标志主体 | `#FFFFFF` | 保证在深色底上的最大对比度 |
| 顶部高光 | 白色 18% → 0% 线性渐变 | 增加体积感 |

**注意**：图标使用固定的品牌渐变，**不随应用主题变化**（应用图标不应随主题切换而改变）。

---

## 3. 使用规范

### 3.1 可以

- 在应用图标、安装器、官网、README、宣传材料中使用。
- 在深色或浅色背景上使用（底板本身提供了足够的对比边界）。
- 等比例缩放。

### 3.2 不可以

| 禁止项 | 原因 |
| --- | --- |
| 改变图标配色、渐变方向或几何比例 | 破坏品牌一致性 |
| 旋转、拉伸、加描边、加投影、加外发光 | 破坏几何纯粹性 |
| 在图标上叠加文字 | 小尺寸下不可读 |
| 用图标与 Git / GitHub / Tauri 的 Logo 组合并置 | 暗示官方关联（红线 R2） |
| 使用未经本文件定义的变体（如单色版、线框版） | 需先在此登记 |

### 3.3 最小尺寸与留白

| 项 | 规则 |
| --- | --- |
| 最小使用尺寸 | **16×16 px**（低于此尺寸改用文字标识） |
| 安全留白 | 图标四周留白 ≥ 底板的 1/8（即 ≥ 画布宽的 10%） |
| 画布占比 | 底板占画布 80%，为 macOS 图标网格预留间距 |
| 背景 | PNG 必须带透明通道；不得填充背景色 |

---

## 4. 合规声明（AGENTS.md 红线 R2）

**ForgeDesk 的应用图标完全原创，具体声明如下：**

1. 未使用 **Git** 官方 Logo 或其任何变体。
2. 未使用 **GitHub** 官方 Logo、**Octocat** 或任何形态的动物形象（含"猫形/触手形"衍生图形）。
3. 未使用 **Tauri** 官方 Logo 或其变体。
4. 未使用"分支连线"、"节点圆点"等与 Git 图形语言直接对应的元素作为标志主体。
5. 未使用任何第三方设计素材（无素材库下载、无 AI 生成图），全部由基础几何图形手工构成。

`docs/brand/icon-source.svg` 是唯一设计真源，其内部注释同样记录了上述声明。

### 4.1 证据链与校验记录（2026-10-07）

> 本节把"图标原创"从一句声明变成**可复核的事实**：每一项都能用仓库内的文件或一条命令验证。

**① 真源自检**（`docs/brand/icon-source.svg`，2058 字符）

| 检查 | 结果 |
| --- | --- |
| 图元构成 | 全部为基础几何：`rect`×5（底板 / 砧面 / 腰身 / 底座 / 顶部高光）+ `path`×3（三枚菱形火花）+ `linearGradient`×2 |
| 内嵌位图（`<image>`） | **无** |
| 外部引用（`href` / `url(http…)`） | **无** |
| 注释之外的品牌词（`octocat` / `github` / `tauri`） | **无**（注释里出现这些词，是声明本身） |
| 与本节五条声明逐条对应 | 一致（同一套声明也写在 SVG 注释里） |

**② 产物可复现**：按 §5.2 第 2 步重跑 `node scripts/brand/render-icon.mjs`，`docs/brand/icon-1024.png` 的 SHA256 **前后完全一致**
（`def8097a509dc22a…`）——即图标确实由这张矢量图渲染而来，且与依赖版本无关（本次在升级后的 `sharp` 0.35.5 下复现成功）。

**③ 产物台账**（SHA256 前 16 位十六进制；复算命令见本节末）

| 文件 | SHA256 |
| --- | --- |
| `docs/brand/icon-source.svg`（真源） | `e8799b01658040f5` |
| `docs/brand/icon-1024.png` | `def8097a509dc22a` |
| `src-tauri/icons/icon.ico` | `885778fc59ac21e4` |
| `src-tauri/icons/icon.icns` | `702fa91cf735daf1` |
| `src-tauri/icons/icon.png` | `b90538569894f34c` |
| `src-tauri/icons/32x32.png` | `31835f939000b52e` |
| `src-tauri/icons/64x64.png` | `43d3a579fd7e0829` |
| `src-tauri/icons/128x128.png` | `13d2bee0a07e9fc2` |
| `src-tauri/icons/128x128@2x.png` | `f47e01d67005e99f` |
| `src-tauri/icons/Square30x30Logo.png` | `62f0a0c80890ef08` |
| `src-tauri/icons/Square44x44Logo.png` | `13ca6965970de1a2` |
| `src-tauri/icons/Square71x71Logo.png` | `322b3a95555aba0c` |
| `src-tauri/icons/Square89x89Logo.png` | `56023b8fc38fed2f` |
| `src-tauri/icons/Square107x107Logo.png` | `82f0a278fe021394` |
| `src-tauri/icons/Square142x142Logo.png` | `b842d8f842f11ad9` |
| `src-tauri/icons/Square150x150Logo.png` | `8fed4d69dcb3e417` |
| `src-tauri/icons/Square284x284Logo.png` | `b93437568d01ac2d` |
| `src-tauri/icons/Square310x310Logo.png` | `4d92d674136f60e4` |
| `src-tauri/icons/StoreLogo.png` | `28ce7cbd78707440` |

台账的用途：**任何**图标改动都会改变其中一行，"这次动的是哪个文件、有没有漏改"因此一眼可见（漏跑 `tauri icon` 会导致台账与 §2 的几何描述不一致）。

**④ 自动化门禁**：`pnpm compliance` 的「图标检查（R2）」通过——纯矢量源、无受限品牌词文件名、无已知官方 Logo 哈希命中、
`tauri.conf.json` 的 `bundle.icon` 引用全部存在、无过小或损坏的文件。

**⑤ 诚实说明——为什么人工确认不能省**：`scripts/compliance/known-logos.json` 的 `knownSha256` **目前是空的**，
这是在能离线核实官方 Logo 文件来源之前**故意**保持的状态（填进非官方渠道来的哈希只会制造虚假的安全感）。
因此"与官方 Logo 无相似"这句话最终仍须由人并排目视确认（§4.2）。

复算命令（跨平台，Node）：

```bash
node -e "const{createHash}=require('crypto'),fs=require('fs'),p=require('path');const d='src-tauri/icons';for(const f of ['docs/brand/icon-source.svg','docs/brand/icon-1024.png'].concat(fs.readdirSync(d).sort().map(n=>p.join(d,n))))console.log(createHash('sha256').update(fs.readFileSync(f)).digest('hex').slice(0,16)+'  '+f)"
```

### 4.2 视觉区分度盲测（B-3 / B-4 的人工部分）

**现状**：按 `docs/adr/ADR-005-defer-cross-platform-verification.md` 的决策，3 人盲测**延后到首次对外预发布之前**执行
（`docs/acceptance/M0.md` 已记录该延后，自动化部分已通过）。界面侧的同类评审（红线 R3，1 人）已记录在
`.github/COMPETITOR-REVIEW.md`；本节管的是**图标侧**（红线 R2）。

**方法（两轮，参与者各自独立完成、不互相讨论）**：

1. **图标轮**：把本图标与 Git、GitHub（含 Octocat）、Tauri 的官方 Logo 并排（等尺寸、打乱顺序、不加文字标签），
   请参与者指出"哪些出自同一产品 / 同一项目"。
2. **界面轮**：把本产品主界面的真实截图与四个竞品的公开宣传截图并排
   （GitHub Desktop / GitKraken / Sourcetree / Fork），请参与者做两两配对。

**判定**：≥ 2/3 人在两轮中均正确 → 通过。出现误判时记录**误判部位**（是"配色像"还是"结构像"），
然后回到设计修改——**不允许**放宽判定来让它通过。

**记录表（待人类填写；编码代理不得代填）**：

| 日期 | 参与者编号 | 轮次 | 结果 | 误判部位 / 说明 |
| --- | --- | --- | --- | --- |
| — | 1 / 2 / 3 | 图标 | 待执行 | — |
| — | 1 / 2 / 3 | 界面 | 待执行 | — |

---

## 5. 资产清单与再生成流程

### 5.1 文件清单

| 文件 | 说明 |
| --- | --- |
| `docs/brand/icon-source.svg` | **设计真源**（矢量，1024×1024） |
| `docs/brand/icon-1024.png` | 由 SVG 栅格化得到（供 Tauri CLI 使用） |
| `src-tauri/icons/icon.ico` | Windows 图标 |
| `src-tauri/icons/icon.icns` | macOS 图标 |
| `src-tauri/icons/*.png` | 32 / 64 / 128 / 128@2x / icon.png |
| `src-tauri/icons/Square*Logo.png`、`StoreLogo.png` | Windows Appx 尺寸（预留给未来可能的商店分发） |

> 说明：本项目为桌面三平台，已移除 `tauri icon` 默认生成的 `ios/` 与 `android/` 目录。

### 5.2 修改图标的流程（三步）

```bash
# 1. 编辑矢量源（唯一真源）
#    docs/brand/icon-source.svg

# 2. 栅格化为 1024 PNG
node scripts/brand/render-icon.mjs

# 3. 生成全部平台图标
pnpm tauri icon docs/brand/icon-1024.png -o src-tauri/icons

# 4. 清理移动端图标（桌面项目不需要）
#    （Windows: Remove-Item -Recurse -Force src-tauri/icons/ios, src-tauri/icons/android）
```

### 5.3 修改后的必做检查

- [ ] 本文件第 2、3 节的描述与几何参数已同步更新
- [ ] 在 16×16、32×32、256×256 三个尺寸下目视确认仍可辨认
- [ ] 与 Git / GitHub / Tauri 官方 Logo 并排对比，确认无相似性（红线 R2）
- [ ] `pnpm tauri icon` 产物已更新，且未提交 `ios/`、`android/` 目录

---

## 6. 界面视觉的原创性要求（红线 R3，摘要）

完整要求见 `docs/PLAN.md` §3.3 与 §9.5。要点：

1. **不得**逐像素复刻 GitHub Desktop / GitKraken / Sourcetree / Fork / Git-cola 的主界面布局。
2. UI 图标可使用 `lucide-react`（ISC 许可，需登记到第三方许可清单），
   但**应用图标、品牌图形必须自绘**。
3. 不得使用竞品的品牌色组合作为本产品主色（本产品主色为 `--fd-brand` `#4F46E5` 靛蓝体系）。
4. 对外材料中的竞品对比**只允许使用文字表格**，不得放置竞品截图或 Logo。
5. M0/T0.4 完成后需执行一次「视觉区分度盲测」：3 人可将本产品主界面截图与竞品截图区分开。
