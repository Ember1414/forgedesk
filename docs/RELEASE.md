# 发布流程（Release）

> 适用：ForgeDesk v1.0 及以后的对外发布。
> 关联：`docs/PLAN.md` §8（产品化与分发）、§11（CI/CD）；`docs/adr/ADR-003`（清单托管）、`ADR-005`（跨平台降级）。
> **零成本原则**：不使用任何付费签名证书、付费商店或付费域名（红线 R5）。

---

## 1. 版本号规则

- 遵循 **SemVer**（`MAJOR.MINOR.PATCH`）；预发布用 `-beta.N`。
- **单一真相源**：`src-tauri/tauri.conf.json` 的 `version`。`package.json` 与
  `Cargo.toml`（`[workspace.package].version`）必须与它一致，`Cargo.lock` 由 cargo 自动跟随。
- 同步与校验一律走脚本，不要手工改三处：

```bash
pnpm version:sync 0.7.1     # 改 tauri.conf.json → 同步 package.json / Cargo.toml → 刷新 Cargo.lock
pnpm version:check          # 只校验三处一致（CI 门禁用）
```

- 历史对应关系（开发期未打 tag）：M0=`0.0.1`，M1–M6=`0.1.0`…`0.6.0`，M7 开发期=`0.7.0`，
  **首个对外正式版 = `1.0.0`**。

**每次发版必须**：更新 `CHANGELOG.md`（新增条目）、提交、打 tag `vX.Y.Z`。

---

## 2. 发布渠道

| 渠道 | 版本形态 | 用途 | 检查频率 |
| --- | --- | --- | --- |
| `stable` | `1.0.0` | 所有用户 | 启动后 60s + 每 24h（可关） |
| `beta` | `1.0.0-beta.N` | 提前 1–2 周验证 | 同上 |

更新清单托管在 Cloudflare Pages（见 `ADR-003`），路径为
`https://forgedesk.pages.dev/updates/<渠道>/<target>.json`（例如 `updates/stable/windows-x86_64.json`）。

> **应用侧已就绪（T7.1）**：`update_check` / `update_install` 命令（见 `docs/API.md`）与状态栏横幅。
> 尚缺的只有**发布配置**：`plugins.updater.{pubkey, endpoints}` 与 §3 的两套密钥——
> 配置缺一即被视为"未配置"，命令返回 `configured: false`，界面不打扰用户。
> **这两项刻意不写进仓库**：发布流水线在构建时用 `scripts/ci/make-build-config.mjs --pubkey … --channel …`
> 注入（理由见 §4.1）。写进仓库的代价是开发构建会从"静默无更新源"变成"每次检查都网络失败"。
> 渠道（stable / beta）在 v1 通过**各渠道各自构建**体现；同一次构建不切换渠道（应用内文案亦如此说明）。

---

## 3. 两套密钥（用途不同，都要离线备份）

### 3.1 GPG 发布密钥 —— 签 `SHA256SUMS`

作用：让用户能验证**下载到的安装包**没有被替换。

```bash
# 生成（Ed25519，2 年有效期；uid 用项目身份，不用个人邮箱）
gpg --batch --quick-generate-key \
  "ForgeDesk Release Signing <release@forgedesk.invalid>" ed25519 sign 2y

# 公钥入仓并上传 keyserver（用户从两处都能拿到）
gpg --armor --export <KEYID> > docs/keys/forgedesk-release.pub
gpg --keyserver keyserver.ubuntu.com --send-keys <KEYID>

# 导出私钥（ASCII armor）→ 存 GitHub Actions Secret：GPG_PRIVATE_KEY
gpg --armor --export-secret-keys <KEYID>
```

签名与校验：

```bash
gpg --armor --detach-sign --local-user <KEYID> --output SHA256SUMS.asc SHA256SUMS
# 用户侧：
gpg --verify SHA256SUMS.asc SHA256SUMS && sha256sum -c SHA256SUMS
# Windows（无 gpg 时）：
Get-FileHash .\ForgeDesk_<版本>_windows_x64.exe -Algorithm SHA256   # 与 SHA256SUMS 比对
```

> 私钥密码单独存 Secret `GPG_PASSPHRASE`。CI 里**只**用环境变量注入，不落盘、不进日志。

### 3.2 更新签名密钥（minisign / Ed25519）—— 签更新包

作用：自动更新只安装**由我们签名**的包（防中间人换包）。

```bash
# 生成（需要时输入并保存一个密码）
pnpm tauri signer generate -w ~/.tauri/forgedesk.key
# Windows（PowerShell）：`~` 不展开，用
#   pnpm tauri signer generate -w "$env:USERPROFILE\.tauri\forgedesk.key"
```

- **公钥**（生成的 `.key.pub` 内容）存为 GitHub **仓库变量** `TAURI_UPDATER_PUBKEY`：
  发布构建时由 `make-build-config.mjs` 注入到应用的 update 配置，效果等价于"硬编码在应用内"，
  但不把发布参数写进仓库（见 §4.1）。
- **私钥与密码**存 CI Secrets：`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
- 私钥与密码必须**离线多份备份**（见 §7）——**丢了就无法再给已发布版本签发更新**。

### 3.3 密钥保管与轮换

| 项 | 存放 |
| --- | --- |
| GPG 公钥 | 仓库 `docs/keys/forgedesk-release.pub` + keyserver |
| GPG 私钥 | 离线加密备份（≥2 份，异地）+ CI Secret |
| 更新私钥 / 密码 | 离线加密备份（≥2 份，异地）+ CI Secrets |
| Cloudflare Token | GitHub Secrets（`CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID`，权限仅 Pages: Edit） |

轮换：生成新密钥 → 更新公钥（GPG 走仓库+keyserver；更新密钥**需要发一个新版本**把新 pubkey 编进应用）
→ 旧私钥在**所有已发布版本的更新窗口结束前不要销毁**。

---

## 4. 发布步骤

前置：`main` 全绿（lint / typecheck / test / cargo fmt+clippy+test / compliance / check:docs）。

```bash
# 1) 定版本并同步三处
pnpm version:sync 1.0.0

# 2) 更新 CHANGELOG.md（把 [0.7.0] - 未发布 改为 [1.0.0] - <日期>，并补 M7 条目）

# 3) 本地跑一遍门禁（与 CI 同组）
pnpm lint && pnpm i18n:lint && pnpm typecheck && pnpm test
pnpm format:check && pnpm check:contrast && pnpm check:workflows && pnpm check:repo && pnpm check:docs && pnpm check:site
pnpm compliance
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 4) 提交并打 tag（tag 是唯一发布触发条件）
git add -A && git commit -m "chore(release): v1.0.0"
git tag -a v1.0.0 -m "ForgeDesk v1.0.0"
git push origin main --follow-tags
```

然后由 `release.yml`（tag 触发）自动完成：

1. 质量门禁（与上表同一组）；
2. **Windows 构建**：NSIS(`.exe`) + MSI(`.msi`) + `pnpm portable:win` 便携版 zip；
3. 归一化命名（`scripts/ci/rename-bundles.mjs`）+ 汇总 `SHA256SUMS`；
4. GPG 签名 → `SHA256SUMS.asc`；
5. 创建 GitHub Release（`draft=false`），附全部产物、`SHA256SUMS`、`SHA256SUMS.asc`；
6. 生成 updater 清单（含签名）并发布到 Pages 的 `updates/<渠道>/<target>.json`（如 `updates/stable/windows-x86_64.json`）；
7. Release Notes 按 Conventional Commits 分类生成。

**约束**：所有 action 固定到 commit SHA；失败时**不发布残缺 Release**（先全部构建成功再发布）；
CI **不**自动向第三方仓库（winget/homebrew/flathub…）推送，只生成清单并开 PR（见 PLAN §M8.2）。

**清单路径的约定**：应用侧的 endpoint 模板是
`https://forgedesk.pages.dev/updates/<渠道>/{{target}}.json`（由 `scripts/ci/make-build-config.mjs` 在构建时注入）。
路径里**不带版本号**：静态托管做不了"没有更新就返回 204"的协商，把版本写进路径会让每次发版都要新建文件，
漏传一次就等于全量用户断更；版本由清单内容回答，旧清单不会触发升级。

### 4.1 流水线的实现（`.github/workflows/release.yml`）

**作业结构**

| 作业 | 运行环境 | 职责 |
| --- | --- | --- |
| `preflight` | ubuntu-22.04 | 三处版本号一致 + **tag ↔ 版本号一致**（打错 tag 是本流程唯一无法自愈的错误）；探测发布凭据 |
| `build-windows` | windows-latest | 注入更新源/公钥 → `tauri build`（msi + nsis + `.sig`）→ 归一化命名 → 便携版 zip → 合并 `SHA256SUMS` → GPG 签名（可选）→ Release Notes → updater 清单 |
| `publish` | ubuntu-22.04 | 创建 Release（`--verify-tag`；`needs` 保证先全部构建成功再发布）→ 组装 `site/` + `updates/`（含 `SHA256SUMS`）后部署到 Pages |

> 站点与清单同宿主（ADR-003 的 Direct Upload）：Pages 发布是目录**快照**，因此发布作业把
> `site/`、`updates/<渠道>/windows-x86_64.json` 与 `updates/<渠道>/SHA256SUMS`（有签名时含 `.asc`）
> 组装到同一个目录再上传，并**拉回另一渠道已有的文件**（否则 stable 发布会把 beta 用户断更）。
> 校验和放到同源，是为了让下载页能直接显示数值 —— GitHub 的 Release 附件不返回 CORS 头。

**需要配置的凭据**（仓库 Settings → Secrets and variables）

| 名称 | 类型 | 用途 | 缺失时 |
| --- | --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Secret | 签更新包（updater 验签用的私钥） | **跳过整个构建**（警告，不失败） |
| `TAURI_UPDATER_PUBKEY` | Variable | 构建时写进应用的公钥 | 同上（两者必须同时存在） |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Secret | 私钥口令（未加密私钥可留空） | 以空口令签名 |
| `GPG_PRIVATE_KEY` / `GPG_PASSPHRASE` | Secret | 签 `SHA256SUMS` | 跳过签名（警告）：少一条独立校验途径 |
| `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID` | Secret | 部署更新清单与官网 | 跳过清单发布（警告）：**用户收不到这个版本** |

> 密钥生成与保管见 §3。**一个凭据都没配时**，流水线会明确警告并跳过构建——这是有意的：
> 让 main 上每次打 tag 都变红，只会训练出"红着也能合"的习惯。

**演练（不产生 Release）**：Actions → Release → Run workflow，`dry_run` 保持默认开启。
它会完整跑一遍构建与打包，产物留在 workflow artifact 里，不创建 Release、不部署清单。

**渠道（stable / beta）**：渠道是**构建时属性**——`make-build-config.mjs --channel` 决定清单地址，
应用内也因此只能如实显示"渠道由安装的构建决定"（见设置页文案）。
beta 走 `workflow_dispatch` 且版本号需自带预发布后缀（如 `1.0.0-beta.1`）；tag 触发一律 stable。

**首次发布时必须核对的两件事**

1. `.sig` 的落点：流水线优先取 `*-setup.exe.sig`（NSIS 安装器的签名），取不到才回退到任意 `.sig`。
   首次发布后请确认 `latest.json` 里 `url` 指向的包正是被签名的那一个；
   （注意：`rename-bundles.mjs` 会把 `…-setup.exe` 归一化成 `…_windows_x64.exe`，
   因此清单里的 `url` 与 Release 附件名都是归一化后的名字——`pnpm release:rehearse` 已把这条形状钉住）
2. 更新清单的 URL 能匿名访问（`curl -fsS https://forgedesk.pages.dev/updates/stable/windows-x86_64.json`），
   并核对里面的 `version` 与 `signature` 与 Release 附件一致。

**本地演练（不需要任何凭据）**

```bash
pnpm release:rehearse
```

它用**假产物**造一棵与 `tauri build` 输出同形的 bundle 树，然后按 `release.yml` 的顺序跑
"归一化命名 → 便携版 zip → 合并 SHA256SUMS → updater 清单 → Release Notes"，
并断言产物命名、`SHA256SUMS` 的行数与格式、清单的版本/签名/URL 形状。

- **覆盖**：脚本之间的接口与顺序（含"归一化后 NSIS 安装器改名成 `…_windows_x64.exe`"这类细节）；
- **不覆盖**：`tauri build` 本身、Ed25519 与 GPG 签名、GitHub Release 创建、Pages 部署。
- 第一次跑它就已经抓出两处问题：清单脚本要求调用方先建目录（已改为自建）、
  以及清单步骤按 `-setup.exe` 匹配归一化后的产物名（永远匹配不到，已修正）。

加了 `--keep` 可以保留现场（`target/release-rehearsal/`）供人工翻看产物。

---

## 5. 发布后检查

```bash
# 用旧版本应用实测：检测到新版本 → 下载 → 签名校验 → 重启后为新版本
# 篡改包必须被拒绝（改动 zip 一个字节后手动放入更新目录，安装应失败并提示）
```

- 下载页（Pages）显示正确的版本与安装指引。**版本号是页面运行时从发布清单读的**
  （`/updates/stable/windows-x86_64.json`），因此它同时也是"清单可匿名访问"的一次活体验证：
  页面若仍显示"尚无可用版本"，说明清单没上传成功或路径不符；
- 页面上列出的四个下载入口都能点开（NSIS / MSI / 便携版 zip / `SHA256SUMS`）——
  它们由命名约定拼出，改名时 `pnpm check:site` 会先红；
- 页面的「查看当前版本的校验和」能展开，且三个哈希与 Release 里的 `SHA256SUMS` 一致
  （它读的是同源的 `updates/<渠道>/SHA256SUMS`）；
- `gpg --verify SHA256SUMS.asc SHA256SUMS` 在新环境可通过；
- 便携版 zip 解压后可启动（见 `docs/install/windows.md`）。

---

## 6. 回滚

三种情形，处理方式不同：

| 情形 | 做法 | 注意 |
| --- | --- | --- |
| 用户升级后出问题，需要回旧版 | 把 updater 清单**指回**旧版本（`latest.json` 里写旧版本号与旧签名包），或让用户到 Releases 下载旧版 | 仅严重事故时用；同时发公告 |
| 发布产物本身有问题（缺文件/命名错） | 删除该 Release 并重新构建**同一 tag** 的产物 | **不要**重写已推送的 tag；改产物可以，改提交历史不行 |
| 代码有缺陷 | 在 `main` 修复 → `pnpm version:sync <下个补丁版本>` → 新 tag 发布 | 优先走这条；前两条是应急 |

**绝不**：force push 已发布的 tag、删除 tag 后重打、用 `--force` 覆盖远端（与 AGENTS 红线 R7 一致）。

---

## 7. 灾难恢复

| 事故 | 影响 | 恢复 |
| --- | --- | --- |
| 更新私钥丢失 | **无法再签发可被旧版本接受的更新**（pubkey 硬编码在已发布应用里） | 只能发布一新版本（用户手动下载）换 pubkey；因此**必须**离线多份备份 |
| GPG 私钥丢失 | 无法签 `SHA256SUMS` | 生成新密钥、更新公钥与 keyserver，并公告密钥变更 |
| Cloudflare Token 泄露 | 清单/官网可被篡改 | 立即吊销 Token、重新签发仅 Pages:Edit 的最小权限 Token、复核清单内容 |
| 发布流水线坏了 | 无法发版 | 可本地 `pnpm tauri build` + `pnpm portable:win` 手动产出并上传（应急路径） |

---

## 8. 发布检查清单

- [ ] 三处版本号一致（`pnpm version:check`）
- [ ] `CHANGELOG.md` 已更新且日期正确
- [ ] 全部门禁本地通过（§4 第 3 步）
- [ ] tag 已推送，`release.yml` 全绿
- [ ] Release 含：安装器、便携版 zip、`SHA256SUMS`、`SHA256SUMS.asc`
- [ ] updater 清单已发布且签名校验通过
- [ ] 用旧版本实测自动更新成功；篡改包被拒
- [ ] 下载页与 Release 说明一致
- [ ] 公告已发（beta → stable 至少间隔 1 周）
