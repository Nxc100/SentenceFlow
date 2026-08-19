# 发布到 GitHub Releases 指南(gh CLI)

> 目的:在**新的 Windows 环境**、**没有 AI 助手**的情况下,你能独立完成
> 「构建 → 发布安装包 → 让用户下载」的全过程。
> 所有命令在 **PowerShell** 中执行,除非另有说明。
> 更新时间:2026-08-19。

---

## 1. gh 是什么

`gh` 是 GitHub 官方命令行工具(GitHub CLI)。平时用的 `git` 只能管代码
仓库本身(提交、推送、分支);而**发布版本、上传安装包、管理 Issue/PR**
这些发生在 GitHub 网站上的事,`git` 管不了——那是 `gh` 的活。

一句话区分:

| 工具 | 管什么 |
|---|---|
| `git` | 本地代码历史与远程仓库同步 |
| `gh` | GitHub 网站上的东西:Releases(发布版本)、Issue、PR、Actions |

## 2. 这个项目用 gh 干什么

**只干一件事:发布安装包给用户下载。**

句流每次构建会产出两个大文件:

| 产物 | 体积 |
|---|---|
| `SentenceFlow_0.1.0_x64-setup.exe`(安装包) | ~5 MB |
| `SentenceFlow.exe`(免安装主程序) | ~16 MB |

### 为什么不直接 `git commit` 这两个文件

试过,而且踩了坑,结论是**绝对不要**:

1. **不可回收**:二进制每次构建的内容都完全不同,git 无法增量压缩,
   每次提交都在历史里永久新增一份完整副本(约 22 MB)。按每天几次提交,
   一两个月仓库就是 GB 级。
2. **删了也没用**:文件一旦进入 git 历史,`git rm` 删掉文件也回收不了
   空间,必须改写整段历史 + 强推,所有克隆过的人都得重新克隆。
3. **用户不好拿**:普通用户不会 `git clone`,在文件树里翻二进制也别扭。

改用 Releases 之后:仓库保持 **4.5 MB**(代码本身只有几百 KB),用户在
仓库首页右侧「Releases」一键下载,还自带版本号、更新说明和下载计数。

发布页:<https://github.com/Nxc100/SentenceFlow/releases>

---

## 3. 一次性准备(新机器只做一次)

### 3.1 安装 gh

```powershell
winget install --id GitHub.cli --accept-source-agreements --accept-package-agreements
```

装完**新开一个 PowerShell 窗口**(旧窗口的 PATH 不会自动刷新),验证:

```powershell
gh --version
```

若提示找不到命令,用完整路径:`& "C:\Program Files\GitHub CLI\gh.exe" --version`。
本文后续统一用变量简化:

```powershell
$gh = "C:\Program Files\GitHub CLI\gh.exe"
```

### 3.2 登录(浏览器授权一次)

```powershell
& $gh auth login --hostname github.com --git-protocol https --web
```

流程:终端显示一个 8 位验证码(形如 `ABCD-1234`)→ 按回车自动打开浏览器
→ 粘贴验证码 → 点授权。凭据存进系统钥匙串,以后免登录。

验证:

```powershell
& $gh auth status
```

应显示 `✓ Logged in to github.com account Nxc100`,且 scopes 含 `repo`。

---

## 4. 日常发布流程

### 4.1 先构建(详见《构建与发布手册》§5)

```powershell
cd D:\xc_work\xc_english\SentenceFlow
taskkill /IM SentenceFlow.exe /F 2>$null
taskkill /IM sentenceflow-desktop.exe /F 2>$null
Set-Location apps\desktop
npx tauri build --bundles nsis
Set-Location ..\..
```

### 4.2 把成品收进 release\(本地暂存区,已 gitignore)

```powershell
New-Item -ItemType Directory -Force release | Out-Null
Copy-Item target\release\sentenceflow-desktop.exe release\SentenceFlow.exe -Force
Copy-Item target\release\bundle\nsis\SentenceFlow_0.1.0_x64-setup.exe release\ -Force
Copy-Item content\build\content.db  release\content.db  -Force
Copy-Item content\channels.json     release\channels.json -Force
```

> `release/` 目录在 `.gitignore` 里,**永远不会被提交**——它只是发布前的
> 中转站。

### 4.3 发布新版本

```powershell
$gh = "C:\Program Files\GitHub CLI\gh.exe"
& $gh release create v0.1.1 `
  release\SentenceFlow_0.1.1_x64-setup.exe `
  release\SentenceFlow.exe `
  release\content.db `
  release\channels.json `
  --title "句流 SentenceFlow v0.1.1" `
  --notes "本版改动:…"
```

发布说明较长时写文件再引用(避免命令行里塞一大段):

```powershell
& $gh release create v0.1.1 release\*.exe release\content.db release\channels.json `
  --title "句流 SentenceFlow v0.1.1" --notes-file notes.md
```

### 4.4 只想刷新最新构建(不发新版本号)

覆盖已有版本的附件:

```powershell
& $gh release upload v0.1.0 release\*.exe --clobber
```

### 4.5 验证发布成功

```powershell
& $gh release view v0.1.0 --json tagName,assets |
  ConvertFrom-Json |
  ForEach-Object { "$($_.tagName): $($_.assets.Count) 个附件" }
```

更彻底的验证(下载回来比对哈希,确认没传坏):

```powershell
$tmp = "$env:TEMP\sf-check"
Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
& $gh release download v0.1.0 --pattern "*setup.exe" --dir $tmp
(Get-FileHash "$tmp\SentenceFlow_0.1.0_x64-setup.exe").Hash -eq
  (Get-FileHash "release\SentenceFlow_0.1.0_x64-setup.exe").Hash
```

返回 `True` 即一致。

---

## 5. 发版前检查清单

- [ ] `tauri.conf.json` 的 `version` 已递增,与安装包文件名、tag 号一致
- [ ] `cargo test --workspace --all-features` 全绿
- [ ] `npm run typecheck` 通过
- [ ] 内容改过就跑 `sf factory build` 重建 content.db
- [ ] 在干净环境装一次安装包做冒烟测试(首启定级 → 练一句)
- [ ] **正式对外发版**:替换 `licensing.rs` 里的开发密钥(见《构建与发布手册》§12)

---

## 6. 踩过的坑(重要)

### 6.1 `gh release create` 会自动打 tag,位置是"当时的 HEAD"

如果你在**含二进制的提交**上创建了 Release,后来又想把二进制从 git 历史
里清掉,会发现清不干净——因为那个 tag 还拽着旧提交不放(`git gc` 后
仓库体积反而变大)。

解决:把 tag 移到清理后的提交上,再强推。

```powershell
git tag -f v0.1.0 <干净的commit号>
git push --force origin v0.1.0
git reflog expire --expire=now --all
git gc --prune=now
```

移动 tag **不影响已发布的 Release 和它的附件**(附件独立存储,Release 靠
tag 名关联),只是 Release 页面显示的源码链接指向新提交。

### 6.2 装完 gh 当前终端仍报 "command not found"

PATH 是进程启动时读取的,新装的程序不会出现在已开的终端里。
新开一个终端,或直接用完整路径 `& "C:\Program Files\GitHub CLI\gh.exe"`。

### 6.3 Git Bash 里也找不到 gh

Git Bash 用的是另一套路径写法:

```bash
"/c/Program Files/GitHub CLI/gh.exe" auth login --hostname github.com --web
```

### 6.4 想确认二进制真的不在 git 历史里

```powershell
git log --all --oneline -- release/SentenceFlow.exe
```

**没有任何输出**才算干净。有输出说明还有引用(分支或 tag)指着含二进制的
提交,回到 6.1 处理。

---

## 7. 常用命令速查

| 目的 | 命令 |
|---|---|
| 看所有版本 | `& $gh release list` |
| 看某版本详情 | `& $gh release view v0.1.0` |
| 下载某版本附件 | `& $gh release download v0.1.0 --dir .\tmp` |
| 追加/覆盖附件 | `& $gh release upload v0.1.0 文件 --clobber` |
| 改标题或说明 | `& $gh release edit v0.1.0 --title "…" --notes "…"` |
| 删除某版本(附件一并删) | `& $gh release delete v0.1.0 --cleanup-tag` |
| 标记为预发布 | `& $gh release edit v0.1.0 --prerelease` |
| 查看仓库体积 | `git count-objects -vH` |

---

## 8. 用户拿到的是什么

Release 页面对用户呈现两种下载方式,发布说明里已写清:

- **`SentenceFlow_x64-setup.exe`(推荐)**——安装包,自带全部内容资源,
  缺 WebView2 会自动引导安装;
- **`SentenceFlow.exe`**——免安装主程序,**必须与 `content.db`、
  `channels.json` 放在同一目录**,单独拷走会闪退(它不是自包含的)。

系统要求:Windows 10 1809(2018 年 10 月更新)及以上。更早的系统能运行
本体,但 opencode 一键安装不可用(缺 ConPTY 系统能力),AI 功能请引导
用户改用「Zen 直连」通道。
