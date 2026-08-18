<div align="center">

<img src="doc/assets/logo.png" alt="句流 SentenceFlow logo" width="110" />

# 句流 SentenceFlow

**看中文、打英文 —— 答对瞬间，句子自动展开成「音标 + 词性 + 句子成分」的彩色解析。**

买断制单机英语整句输出训练软件 · 本地数据 · 离线可用 · 无账号无服务器

[![CI](https://github.com/Nxc100/SentenceFlow/actions/workflows/ci.yml/badge.svg)](https://github.com/Nxc100/SentenceFlow/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/Rust-1.85%2B-orange?logo=rust)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri)
![React](https://img.shields.io/badge/React-18-61DAFB?logo=react)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20Web-blue)

<img src="doc/assets/trial-parse-view.png" alt="签名时刻:答对后句子展开为彩色成分解析" width="720" />

</div>

---

## 这是什么

句流是一款面向中国学习者的**英语整句打字训练**软件:

- **打字 / 拆句重组 / 听打 / 默写**四种练习模式,Leitner 五盒间隔复习(SRS)自动排程;
- 答对的瞬间触发**签名时刻**——下划线消融、单词按句子成分聚拢成彩色卡片、词性胶囊弹入、音标浮现;颜色即教学信息,不是装饰;
- 出厂内置六级(L1–L6,锚定 CEFR)全标注句库,**零配置、完全离线可练**;
- 接入 AI 通道后解锁**生成工坊**:为任何场景("下周出差要用的机场句子")现场生成带全套解析的专属句集,每一句都先过本地确定性校验才入库;
- 学习数据只存本机,API Key 只进系统钥匙串,练习路径零网络调用——**架构上可审计**。

## AI 四通道(全部可选,不配置不影响任何学习功能)

| 通道 | 形态 | 费用 |
|---|---|---|
| **opencode 本地** | 驱动本机 [opencode](https://opencode.ai) CLI 的免费模型 | 0(限速) |
| DeepSeek 官方 | HTTPS API,自己的 Key | 按量,预算硬顶 |
| Zen 直连 | OpenAI 兼容端点 | 免费模型 0 |
| Ollama 本地 | localhost:11434 | 0,全离线 |

四通道同一 `ChannelAdapter` trait;计量双轨(付费:预估→实时计量→触顶硬拦截;免费:请求桶+可视化退避)——**永不静默烧钱**。免费模型接入后自动跑**微基准**:每个候选模型生成 6 句、本地校验打分、择优选用。

## 仓库布局

```
crates/
  sf-core/       纯逻辑核心:LevelSpec 解释、SRS 五盒、会话编排、逐词判定、统计
                 (无 IO;时间与随机种子全显式 → 桌面与 wasm 双端逐比特一致)
  sf-pipeline/   统一生成管线:prompt 装配、流式 JSON 解析、确定性校验
                 (NGSL 词表带 / chunks 全覆盖 / POS·ROLE 闭合枚举 / IPA 字符集 /
                  词典对账覆写 / simhash 查重)、factory/user 双档分诊
                 + feature "store"(SQLite 内容库) + feature "factory"(`sf` CLI)
  sf-llm/        四通道适配、计量双轨、指数退避、系统钥匙串、可续跑任务队列、
                 微基准打分、channels.json 远程策略
  sf-license/    离线授权:.sflic Ed25519 本地验签 + 签发 CLI(feature "issuer")
  sf-wasm/       sf-core 的 wasm-bindgen 绑定(JSON 字符串 ABI)
apps/
  desktop/       Tauri 2 桌面端(售卖主体):src-tauri Rust 壳(36 个 command)
                 + React 前端(今日/练习/内容库/生成工坊/报告/设置)
  web-trial/     Web 试用版(纯静态站):L1–L2 各一节,IndexedDB 进度可导出带走
packages/ui/     共享 React 组件 —— 设计规范(§5/§6)的唯一实现处:
                 设计令牌(浅色/深色/护眼纸色)、练习引擎组件、签名时刻动效
content/         specs/*.yaml(六级 LevelSpec,单一事实源)· seed/(种子句库)
                 · lexicon/(NGSL 全量词典)· channels.json
tools/
  audit/         spaCy 离线抽审脚本(纯 QA,不在运行时依赖树)
  dev-keys/      开发测试签名密钥(⚠ 发布前必须更换,见下)
doc/             完整开发规范(v5 合订版)· 开发状态与偏差记录 · 手动测试指引
```

## 快速开始

**前置**:Rust 1.85+(stable)、Node.js 22+、npm。Windows 另需 WebView2(Win11 自带)。

```bash
git clone https://github.com/Nxc100/SentenceFlow.git
cd SentenceFlow

# 1. Rust 全量测试(六 crate + Tauri 壳,146 用例)
cargo test --workspace --all-features

# 2. 从种子句库构建出厂 content.db(全部句子过校验管线)+ 金标回归
cargo run -p sf-pipeline --features factory --bin sf -- factory build
cargo run -p sf-pipeline --features factory --bin sf -- gold run

# 3. 前端依赖 + wasm 引擎
npm install
rustup target add wasm32-unknown-unknown
npm run build:wasm

# 4a. Web 试用版
npm run dev:trial                        # http://localhost:5173

# 4b. 桌面端开发(两个终端)
npm run dev --workspace apps/desktop     # 终端 A:前端(端口 5174)
cargo run -p sentenceflow-desktop        # 终端 B:Tauri 壳
```

### 打包发布版

```bash
cd apps/desktop
npx tauri build --bundles nsis           # Windows 安装包
# 产物:target/release/bundle/nsis/SentenceFlow_<版本>_x64-setup.exe
# (content.db 与 channels.json 作为资源一并打入)
```

## 工厂内容生产(`sf` CLI)

```bash
# 校验种子句(任何一句不过校验即失败退出)
cargo run -p sf-pipeline --features factory --bin sf -- factory validate

# 经 AI 通道批量生成(本机已装 opencode / Ollama,或提供 Key)
cargo run -p sf-pipeline --features factory --bin sf -- factory gen \
  --scene "机场值机" --level L3 --count 20 \
  --channel opencode --model opencode/deepseek-v4-flash-free

# 导出 Web 试用版内容 JSON(L1–L2 各一节,含 LevelSpec)
cargo run -p sf-pipeline --features factory --bin sf -- export trial
```

手写种子与 AI 生成走**同一条校验管线**:词表带越级、句长、成分全覆盖不重叠、
词性/成分闭合枚举、IPA 字符集、lemma 词典对账覆写、simhash 近重复——
不合格的句子进不了任何数据库。

## 授权(离线,无激活服务器)

```bash
# 厂商侧:生成密钥对(私钥离线保存,绝不入仓库)
cargo run -p sf-license --features issuer -- keygen --out-dir ./keys

# 每单签发一张 .sflic(<10 秒)
cargo run -p sf-license --features issuer -- issue \
  --email user@example.com --major-max 3 \
  --key-file ./keys/sf-license-private.secret
```

客户端内嵌公钥本地 Ed25519 验签;不绑设备,换机 = 拷贝 `.sflic` 文件。
试用 14 天全功能(时钟回拨即到期),到期转体验模式(每日 5 句,不清数据、不弹窗)。

> ⚠ **发布前必做**:`apps/desktop/src-tauri/src/licensing.rs` 中的
> `LICENSE_PUBLIC_KEY_B64` 当前是**开发测试公钥**(配对私钥在 `tools/dev-keys/`,
> 仅用于本地端到端验证激活流),上线前必须换成离线生成的生产密钥。

## 架构不变量

- **练习路径零网络**:sf-core 与练习 UI 不链接 sf-llm;全部网络请求只出自
  生成工坊 / 答疑 / 周点评三个入口。
- **LevelSpec 单一事实源**:生成约束、校验反查、练习行为读同一份 YAML,
  引擎无等级硬编码;spec 快照随 content.db 分发,内容与行为同版。
- **校验器是契约**:LLM 输出必须过确定性校验才能入库——模型再不可靠也
  不产出破碎数据。
- **隐私红线**:Key 只进系统钥匙串且备份结构上不含密钥;绝不读取用户的
  opencode 凭据(通道可用性仅经 `opencode models` 输出间接判定,
  免费模型匿名可用、无需登录)。
- **双端一致**:`now`/`seed` 全是显式参数,桌面(native)与试用版(wasm)
  对同一输入产生逐比特一致的行为。

## 测试与质量

| 检查 | 命令 |
|---|---|
| Rust 单测(146 用例) | `cargo test --workspace --all-features` |
| Lint(拒绝 warning) | `cargo clippy -p sf-core -p sf-license -p sf-pipeline -p sf-llm --all-features -- -D warnings` |
| 格式 | `cargo fmt --check` |
| 金标回归 | `cargo run -p sf-pipeline --features factory --bin sf -- gold run` |
| 前端类型检查 | `npm run typecheck` |
| POS 标注抽审(可选) | `python tools/audit/audit_pos.py content/build/content.db` |

CI(GitHub Actions,Linux + Windows)覆盖以上全部 + wasm32 编译检查。
打包后的桌面 exe 已通过 WebView2 CDP + Playwright 做过全功能 UI 实测
(含 opencode 真机生成/答疑/微基准),记录见 `doc/开发状态.md`。

## 项目状态

核心已全栈完成并实测;尚待完成(优先级序):全量 5000 句内容生产、词典
IPA/释义补全、piper 离线语音包、内容包自动更新通道、代码签名与发卡接入。
详细里程碑对照、与规范的 8 处有意偏差及理由、opencode 真机 spike 结论:
**[doc/开发状态.md](doc/开发状态.md)**。产品完整规范:
[doc/句流-完整开发文档-v5-合订版.md](doc/句流-完整开发文档-v5-合订版.md)。
手动测试清单:[doc/手动测试指引.md](doc/手动测试指引.md)。

## 词表来源与署名

词频带基于 **NGSL**(New General Service List, v1.01)— Browne, C., Culligan, B. & Phillips, J. (2013),
[CC BY 3.0](https://creativecommons.org/licenses/by/3.0/) 授权,
见 [newgeneralservicelist.com](https://www.newgeneralservicelist.com/)。

## 许可

本仓库为**专有软件**源码(买断制商业产品),保留所有权利;
不接受未经协商的再分发。词典数据的 NGSL 部分依其 CC BY 3.0 条款使用并署名如上。
