# 提交签名指南（SIGNING）

> 如何让 ForgeDesk 里的提交显示 **Verified**（有效签名），以及如何验证别人的签名。
> ForgeDesk 不管理密钥本身——签名与验证都委托给本机的 git + GPG/SSH 工具链；
> 本指南只覆盖"让它们协同工作"的部分。

## ForgeDesk 里的签名显示

提交详情面板的"签名"字段直接来自 `git log` 的签名状态（`%G?`），七种取值：
有效签名 / 签名无效 / 有效但密钥不受信任 / 签名已过期 / 密钥已过期 / 密钥已吊销 /
缺少公钥 / 未签名。**历史页的每一行都携带该状态**——这就是"批量验证"：打开
历史页即见最近 N 条的签名状态，无需额外操作。

提交面板的签名三态开关（自动 / 签名 / 不签名）映射到 git 的行为：

| 开关 | git 行为 | 用途 |
| --- | --- | --- |
| 自动 | 不传参数，尊重仓库/全局的 `commit.gpgsign` | 默认 |
| 签名 | `--gpg-sign` | 仓库没开 gpgsign 但你想签 |
| 不签名 | `--no-gpg-sign` | 临时覆盖 `commit.gpgsign=true`（签名链路坏了时保交付） |

签名失败（无密钥/密钥过期）会让 git 提交本身失败——ForgeDesk 如实显示 git 的
错误并给出去诊断页的入口，**不会**静默产生未签名提交（除非你显式选了"不签名"）。

## GPG 签名（传统方案）

1. 安装并初始化：`gpg --full-generate-key`（选 ECC sign-only 或 RSA，绑定你的邮箱）；
2. 把公钥给平台：`gpg --armor --export <KEY_ID>`，粘贴到 GitHub → Settings → SSH and GPG keys；
3. 让 git 用它：
   ```bash
   git config --global user.signingkey <KEY_ID>
   git config --global commit.gpgsign true
   ```
4. 在 ForgeDesk：设置 → 网络与密钥 → GPG 签名密钥应能列出该密钥；点"签名自检"
   做一次 clearsign 往返验证（无需仓库）。

常见失败：`gpg` 不在 PATH（Windows 需要 gpg4win）；`-pinentry` 弹窗在后台被
拦截；密钥已过期（`gpg --list-keys` 看 `[expired]` 标记）。

## SSH 签名（更简单的现代方案，git 2.34+）

1. 生成签名专用密钥：`ssh-keygen -t ed25519 -C "signing" -f ~/.ssh/signing_ed25519`；
2. 告诉 git 用它：
   ```bash
   git config --global gpg.format ssh
   git config --global user.signingkey ~/.ssh/signing_ed25519.pub
   git config --global commit.gpgsign true
   ```
3. 把**公钥内容**粘贴到 GitHub → SSH keys（类型选 *Signing key*）；
4. 验证别人的 SSH 签名需要信任库：
   ```bash
   git config --global gpg.ssh.allowedSignersFile ~/.ssh/allowed_signers
   # 文件每行：邮箱 namespaces git-ssh 公钥
   # 例：alice@example.com git ssh-ed25519 AAAA...
   ```

ForgeDesk 的提交详情对 SSH 签名同样显示七态；"缺少公钥"通常意味着
`allowedSignersFile` 没配或没包含签名者。

## ForgeDesk 不做什么（边界）

- **不管理密钥**：生成/导入/吊销都在 gpg/ssh 工具链里完成；
- **不修改 git 全局配置**：设置页里的签名相关操作只经 `-c` 参数作用于单次
  提交或提供指引，绝不静默改写用户的 git 配置；
- **不做密钥服务器交互**：拉取他人公钥请在平台上手动完成。
