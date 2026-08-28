# 「AI 萌宠」模块 · 整合执行方案

> **目标**:把 `D:\xc_work\pet_ai`(孵宠 HatchDesk v0.2.0)的**全部功能**整合进句流,
> 成为一个独立的「AI 萌宠」模块 —— 透明置顶的桌面宠物窗 + 句流主窗内的孵化器页面。
> **红线**:不影响句流任何既有功能;代码质量与项目架构对齐句流现有规范。
>
> 方案基于对两个代码库的逐文件勘察(pet_ai:Rust 10,214 行 / 前端 5,658 行 /
> 51 个 Tauri 命令 / 10 个事件 / 126 个单测;句流:见《句流-功能全景文档》)。
> 文档日期:2026-08-28。执行前请先通读 §2(源项目盘点)与 §4(风险清单)。

---

## 目录

1. [结论速览](#1-结论速览)
2. [源项目盘点(HatchDesk 是什么)](#2-源项目盘点)
3. [目标架构](#3-目标架构)
4. [风险清单与逐条对策](#4-风险清单与逐条对策)
5. [分阶段执行计划(七个 Phase,每阶段独立可验收)](#5-分阶段执行计划)
6. [命令与事件重命名总表](#6-命令与事件重命名总表)
7. [测试与验收门禁](#7-测试与验收门禁)
8. [有意不迁移项(记录在案的决策)](#8-有意不迁移项)
9. [P1 扩展:与学习闭环的联动(本期不做)](#9-p1-扩展与学习闭环的联动)
10. [工作量与提交切分](#10-工作量与提交切分)

---

## 1. 结论速览

| 决策点 | 结论 |
|---|---|
| 总体形态 | **纯逻辑下沉 `crates/sf-pet`(≈5,700 行零改动)+ 桌面壳胶水层 `apps/desktop/src-tauri/src/pet/` + 宠物窗独立 Vite 入口(vanilla TS 原样迁移)+ 孵化器五页签重写为句流 React 页「AI 萌宠」** |
| 窗口模型 | 句流主窗(不变)+ 运行时创建的 `pet` 透明置顶窗。**HatchDesk 的 studio 窗取消**,其功能并入主窗「AI 萌宠」导航页 |
| 命令命名空间 | 51 个命令全部改为 `pet_*` 前缀(§6 总表),并入句流 `generate_handler!` 单列表 |
| 事件命名空间 | 全部改为 `pet://*`(§6),对齐句流 `workshop://`、`chat://` 惯例 |
| 数据落盘 | 全部收进 `<app_data>/pet/` 子目录;宠物设置并入句流 `Settings` 的新 `pet` 分节(kv,`#[serde(default)]` 向后兼容) |
| 主题 | 废弃 HatchDesk 自己的 localStorage 主题系统,宠物窗与「AI 萌宠」页**跟随句流四主题**(马卡龙等),CSP 保持句流现状**零放宽** |
| 托盘 | **不引入**(句流现无托盘,主窗关闭=退出的语义不变);宠物显隐入口 = 「AI 萌宠」页 + 宠物右键菜单 |
| 新增依赖 | Rust:`image`、`chrono`、`notify`、`tract-onnx`(feature 门控)、`tauri-plugin-notification`、`tauri-plugin-opener`;前端:**零新增**(宠物窗零框架,孵化器用句流现有 React 栈) |
| 练习路径不变量 | 「练习路径零网络调用」「LevelSpec 单一事实源」「锁序 progress→content→settings」全部**不受触碰**;pet 模块自持状态与锁,不 import 练习任何东西,练习也不 import pet |
| 体积影响 | exe 约 +5 MB(u2netp 模型 4.4 MB + tract 推理);编译时间增加明显(tract-onnx 依赖树)——均有 feature 开关兜底 |

---

## 2. 源项目盘点

### 2.1 HatchDesk 能做什么(全部保留)

**产品一句话**:拖一张图 → 60 秒变成活在桌面上的宠物;复制「咒语」去豆包/即梦/ChatGPT
生成网格图拖回来,宠物当场学会待机+走路;吃饭/提醒/睡觉由道具引擎补齐;可选 5 秒视频
抽帧获得真呼吸。**零生成式云端、零 API Key、素材处理与 AI 抠图全在本机。**

功能清单(P0 七件套 + v1.1 改进,详见 pet_ai/README.md):

1. **桌面运行时**:透明置顶宠物窗、轮廓级点击穿透(只有宠物的不透明像素拦截鼠标)、
   拖拽落地物理、工作区漫游、光标感知、夜间自动睡觉、右键菜单;
2. **合成向导**:L0 单图孵化(泛洪/绿幕抠图 + u2netp 智能抠图兜底)→ L1/L2 网格图与
   视频抽帧进化(内容感知切帧、pHash 查重、自动容差、救帧、重切、逐帧替换);
3. **咒语包**:3 平台 × 8 状态提示词模板(data.json 数据化)、布局引导图生成、失败自查表;
4. **本地动画引擎**:手写骨骼引擎(轮廓追踪→网格→ARAP→软光栅→步态烘焙)把单图
   「进化」成会走会跳;运行时 Canvas 动画(分层剪纸行走/眨眼/呼吸/睡姿/粒子/道具);
5. **提醒系统**:一次性日程/循环提醒/番茄钟,1s 心跳线程 + 系统通知 + 宠物气泡播报;
6. **出生视频**:15s 竖版 720×1280(孵蛋→破壳→走秀→落版),ffmpeg 管道编码(外部依赖);
7. **.petkit 宠物包**:导入/导出(ZIP:pet.json + spritesheet.webp),导入前双层校验。

### 2.2 技术底盘对照

| 维度 | HatchDesk | 句流 | 整合含义 |
|---|---|---|---|
| 壳 | Tauri 2 | Tauri 2 | 同代,可并 |
| 前端 | **零框架** vanilla TS + Canvas,手写 `el()` DOM 工厂 | React 18 + TS | 宠物窗保留 vanilla(合理:纯 Canvas 性能层);孵化器重写为 React |
| 设计令牌 | tokens.css,**马卡龙色系 + 明暗双主题** | tokens.css 四主题(含马卡龙少女) | 血缘相近,映射成本低;统一到句流令牌 |
| 窗口 | 双窗(pet + studio),全运行时创建 | 单主窗(配置声明) | pet 窗照搬;studio 并入主窗页面 |
| CSP | `null`(全开) | 严格 `default-src 'self'` | 实测 HatchDesk 只需 `img-src data:`(句流已有)+ 两段内联主题脚本(将删除)→ **零放宽** |
| 对话框 | tauri-plugin-dialog | `rfd` crate | 统一改走 rfd(句流惯例) |
| 状态 | 4 个 `Mutex` State | `Arc<AppState>` 统一状态 | pet 状态并入 AppState 新字段 |
| 数据 | `%APPDATA%/com.hatchdesk.app/` 散文件 | `%APPDATA%/app.sentenceflow.desktop/` + progress.db kv | pet 数据收进 `pet/` 子目录;设置并入 kv |
| 测试 | 126 个 Rust 单测 + 浏览器 mock 测试台 | 209 个 Rust 单测 + CDP 真机实测 | 单测随纯逻辑 crate 全量迁移 |

### 2.3 命令面与事件面(迁移对象全集)

- **51 个 `#[tauri::command]`**,全部集中在 `pet_ai/src-tauri/src/commands.rs`(759 行),
  注册于 `lib.rs:71-123`。与句流现有 65 命令**当前零字面冲突**,但 `settings_get/set`、
  `track_event`、`app_quit`、`work_area_get`、`export_birth_video` 等属高危通用名 → §6 全表重命名。
- **9 个 Rust→前端事件** + 1 个前端跨窗事件(`theme-changed`),全部无命名空间,
  其中 `settings-changed`/`theme-changed`/`export-progress` 与句流现在或将来必然相撞 → §6 全表重命名。
- **4 个后台执行体**:1s 提醒心跳线程(无停止机制,需改造)、notify 下载目录监听、
  文件稳定性等待短线程、3 处 `spawn_blocking`(视频抠像 ×2、视频导出)。
- **磁盘布局**:`config.json` + `reminders.json` + `events.jsonl` + `pets/<petId>/{pet.json,
  spritesheet.webp(1536×1664 WebP), rig_source.png, source/<state>/NNN.png}`。
- **资产内嵌**:u2netp.onnx 4.4 MB(`include_bytes!` + `OnceLock` 懒加载)、
  spellbook/data.json 25 KB(`include_str!`,注意:现状每次调用重复解析,迁移时加缓存)。

### 2.4 纯逻辑 / tauri 耦合切面(决定 crate 边界的关键事实)

逐文件核对结果:**只有 14 个文件触碰 tauri**,以下模块是零 tauri 纯逻辑,可原样搬迁:

| 模块 | 行数 | 内容 |
|---|---|---|
| `imaging/`(9 文件) | 2,180 | 色键/泛洪/降噪/切帧/对齐合成/pHash/u2netp 抠图/视频解帧/运动分析 |
| `rig/`(8 文件) | 1,455 | 轮廓/网格/ARAP(含手写 Cholesky)/软光栅/步态/烘焙 |
| `validator/` | 335 | 硬性+软性双层 QA |
| `exporter/`(2 文件) | 604 | 出生视频逐帧渲染 + ffmpeg 管道封装 |
| `spellbook/`(2 文件) | 840 | 咒语模板拼装 + 程序化引导图 |
| `petkit/mod.rs` | 269 | PetSpec/StateId 八状态契约/图集几何常量 |

合计 ≈ **5,683 行(56%)零改动下沉**。唯一障碍是 `error.rs` 的
`AppError::Tauri(#[from] tauri::Error)` 变体(1 行)—— 在 crate 版错误类型中移除即可。

tauri 耦合层(将重写进胶水模块):`commands.rs`(759)、`wizard/mod.rs`(1,668,其中约
1,400 行编排逻辑实为纯的,tauri 触点集中在 6 个函数)、`petkit/store.rs`(354,耦合仅为
AppHandle→路径,参数化 `&Path` 即纯化)、`runtime/`(126)、`reminder/`(407)、
`settings.rs`(138)、`watcher.rs`(160)、`tray.rs`(76,不迁移)、`webcache.rs`(147,不迁移)、
`analytics.rs`(27,不迁移)、`paths.rs`(39)。

---

## 3. 目标架构

### 3.1 五条总原则(对齐句流架构不变量)

1. **增量、不入侵**:pet 模块只做加法。练习/句库/AI 等既有模块的任何文件不因本次整合
   产生行为变化;`sf-core`/`sf-pipeline`/`sf-llm` 一行不改。
2. **双向隔离**:`sf-pet` 不依赖任何句流 crate;句流练习路径不 import pet 模块。
   耦合点只有三处,且都在壳层:`lib.rs` 装配、`Settings.pet` 分节、主窗导航项。
3. **命名空间强制**:命令 `pet_*`、事件 `pet://*`、数据目录 `pet/`、React 路由 key `aipet`。
   杜绝与现有 65 命令 / 17 事件的任何现在或将来的碰撞。
4. **锁序不变**:句流既有锁序 `progress → content → settings` 照旧;pet 自持
   `PetState`(独立锁),规则:**pet 锁永远最后取、绝不跨 await 持有、绝不在持 pet 锁时
   去取句流三把锁**。
5. **CSP 零放宽**:删除 HatchDesk 的两段内联主题脚本(改为跟随句流主题),其余经实测
   仅需 `img-src data:` 与 `style-src 'unsafe-inline'`,句流现有 CSP 已具备。

### 3.2 代码布局

```
crates/
  sf-pet/                          # 新:纯逻辑 crate(零 tauri 依赖)
    src/
      error.rs                     # PetError(去掉 Tauri 变体;io/serde/图像/zip)
      petkit.rs                    # ← pet_ai petkit/mod.rs(八状态契约、PetSpec、几何常量)
      store.rs                     # ← petkit/store.rs,AppHandle 参数全部改 &Path
      archive.rs                   # ← petkit/archive.rs(.petkit 打包/解包)
      imaging/…(9 文件原样)
      rig/…(8 文件原样)
      validator.rs
      exporter/…(2 文件;ffmpeg 定位逻辑参数化 override 路径)
      spellbook/…(2 文件 + data.json;bundle() 加 OnceLock 缓存修掉重复解析)
      wizard.rs                    # ← wizard/mod.rs 的纯编排部分(~1,400 行):
                                   #   会话结构、切帧/合成/校验编排、SessionView 构建,
                                   #   进度通过 `impl FnMut(Progress)` 回调注入
    Cargo.toml                     # features: default=["matting"]; matting=dep:tract-onnx
    tests/ …                       # 126 个单测随模块迁移

apps/desktop/src-tauri/src/
  pet/                             # 新:tauri 胶水层(全部新写,对照原文件语义)
    mod.rs                         # pub fn commands 列表宏 / setup(&AppHandle) / on_window_event 分派
    state.rs                       # PetState{ settings快照, wizard: Mutex<WizardSession>,
                                   #           reminders: ReminderInner, watcher, heartbeat_stop: AtomicBool }
    commands.rs                    # 51 个 pet_* 命令(薄:取参 → sf-pet → 落盘/emit)
    runtime.rs                     # pet 窗创建/落位/缩放/穿透/显隐(← runtime/mod.rs,去 studio 部分)
    reminder.rs                    # CRUD/tick/番茄钟(← reminder/mod.rs),通知走 plugin-notification
    watcher.rs                     # 下载目录监听(← watcher.rs)
    paths.rs                       # <app_data>/pet/{pets,reminders.json} 路径学
  (既有 15 个文件不动;lib.rs/settings.rs/state.rs 三处做加法,见 §3.5)

apps/desktop/
  pet.html                         # 新:宠物窗入口(无内联脚本;引 packages/ui tokens.css)
  src/pet-window/                  # ← pet_ai src/pet/ 12 文件 + shared 里宠物窗用到的部分,
                                   #   vanilla TS 原样迁移;ipc.ts 改为 pet_* 命令名与 pet:// 事件
  src/pages/AiPet/                 # 新:「AI 萌宠」React 页
    index.tsx                      # 页内五页签壳(向导/咒语包/我的宠物/提醒/设置)
    WizardTab.tsx  SpellbookTab.tsx  PetsTab.tsx  RemindersTab.tsx  PetSettingsTab.tsx
    CalibrateModal.tsx             # 认主校准(canvas 点选,逻辑复用 calibrate.ts 移植)
    petIpc.ts                      # 类型化 IPC(← shared/ipc.ts + types.ts,pet_* 化)
  vite.config.ts                   # rollupOptions.input = { main: index.html, pet: pet.html }
```

### 3.3 窗口模型与生命周期

- **pet 窗**:运行时创建(label `"pet"`,`pet.html`),参数照搬原实现:
  300×390 逻辑像素 × scale(0.75–2.0 钳位)、transparent/无边框/无阴影/置顶/跳过任务栏/
  不可缩放、`accept_first_mouse`,初始落位工作区右下角。
  - 创建时机:`setup` 中 **仅当 `settings.pet.enabled == true`**(新增总开关,默认 `false`
    —— 老用户升级后无感知,主动到「AI 萌宠」页开启,这是"不影响既有功能"的最后一道闸)。
  - 关闭语义:主窗关闭 → 进程退出 → pet 窗随之销毁(句流现有语义不变)。
    pet 窗自身收到 `CloseRequested`(如 Alt+F4)→ `prevent_close` + 隐藏
    (新增一个 `on_window_event`,按 label 分派;句流现无该处理器,属纯增量)。
- **studio 窗:取消**。五页签内容进主窗「AI 萌宠」页。原 `studio_open(tab)` 语义改为:
  `pet_open_studio(tab)` = 主窗 `show + set_focus` + emit `pet://nav`(React 页监听切签)。
  宠物右键菜单的「打开孵化器 / 设置提醒 / 去孵化」全走这条,体验等价。
- **点击穿透**:两层机制原样保留(Rust 全局开关 + 前端 30Hz 轮廓级命中测试)。
  依赖的 `pet_cursor_position_get`、`pet_work_area_get` 与 14 项 `core:window:*` 权限
  通过新增 capability 文件授予 **仅 pet 窗**(§3.6)。

### 3.4 数据模型

```
<app_data>/pet/
├─ pets/<petId>/{pet.json, spritesheet.webp, rig_source.png, source/<state>/NNN.png}
└─ reminders.json
```

- **`config.json` 不迁移**:HatchDesk 的 13 项设置并入句流 `Settings`(kv "settings")新分节:

```rust
/// apps/desktop/src-tauri/src/settings.rs 追加(#[serde(default)] 保证旧档兼容)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PetSettings {
    pub enabled: bool,            // 新增总开关,默认 false
    pub scale: f32,               // 1.0(0.75–2.0)
    pub roam: bool,               // true
    pub click_through: bool,      // false
    pub night_sleep: bool,        // true
    pub performance_mode: bool,   // false
    pub sleep_after_min: u32,     // 8
    pub props_enabled: bool,      // true
    pub watcher_enabled: bool,    // ⚠ 默认 false(句流语境下的隐私保守化,原默认 true)
    pub ffmpeg_path: Option<String>,
    pub active_pet: Option<String>,
    pub first_hatch_done: bool,   // ← first_run_done 改名(不再控制窗口显示,只控引导文案)
}
```

- 丢弃字段:`pro`(句流免费,无付费判据)、`analytics`(整个 analytics 模块不迁移,§8)。
- 写路径:统一走新增命令 `pet_settings_set`(内部:更新 `Settings.pet` → 复用句流
  `save_settings` 持久化 → 差异联动窗口(scale/穿透/显隐)→ emit `pet://settings`)。
  句流原有 `get_settings/set_settings` 不动(前端设置页看不到 pet 分节,pet 设置只在
  「AI 萌宠」页内管理,避免两处写同一分节)。
- **主题**:pet 窗启动时经 `pet_bootstrap` 命令拿 `{settings.pet, appearance.theme, 图集}`;
  句流 `set_settings` 在 appearance 变更时**追加** emit `pet://theme`(对既有前端无影响,
  纯新增事件)。宠物窗气泡/右键菜单配色使用句流令牌变量。

### 3.5 对句流既有文件的全部触点(穷举,超出此列表即越界)

| 文件 | 改动 | 性质 |
|---|---|---|
| `src-tauri/Cargo.toml` | 加依赖:`sf-pet`、`chrono`、`notify`、`tauri-plugin-notification`、`tauri-plugin-opener`;tauri features 加 `image-png`、`macos-private-api` | 加法 |
| `src-tauri/tauri.conf.json` | `"macOSPrivateApi": true`(透明窗 macOS 硬要求;Windows 无影响) | 加法 |
| `src-tauri/src/lib.rs` | `mod pet;` + 两个插件 init + `pet::setup(&handle)` + `on_window_event`(新增,按 label 分派)+ handler 列表追加 51 项 | 加法 |
| `src-tauri/src/state.rs` | `AppState` 加 `pet: pet::PetState` 字段 | 加法 |
| `src-tauri/src/settings.rs` | `Settings` 加 `pet: PetSettings`(serde default);`set_settings` 在 appearance 变更时追加 emit `pet://theme` | 加法 |
| `src-tauri/capabilities/` | 新增 `pet.json`(见 §3.6);`default.json` 的 main 窗加 opener 两项权限 | 加法 |
| `apps/desktop/vite.config.ts` | rollup 双入口 | 加法 |
| `apps/desktop/src/App.tsx` | 导航数组加 `{ key:"aipet", label:"AI 萌宠", icon:"🐾" }` + 路由分支 | 加法 |
| `apps/desktop/src/appState.tsx` | 无需改(pet 设置不走全局 settings context) | — |

### 3.6 权限与安全(capabilities)

新增 `src-tauri/capabilities/pet.json`:

```jsonc
{
  "identifier": "pet",
  "description": "宠物窗:窗口自管理(拖拽/落位/穿透/显示器查询);主窗:外链与文件定位。",
  "windows": ["pet"],
  "permissions": [
    "core:default",
    "core:window:allow-start-dragging", "core:window:allow-set-position",
    "core:window:allow-set-size", "core:window:allow-set-focus",
    "core:window:allow-show", "core:window:allow-hide",
    "core:window:allow-set-always-on-top", "core:window:allow-set-ignore-cursor-events",
    "core:window:allow-current-monitor", "core:window:allow-primary-monitor",
    "core:window:allow-outer-position", "core:window:allow-outer-size",
    "core:window:allow-inner-size", "core:window:allow-scale-factor"
  ]
}
```

主窗(default.json)追加:`opener:allow-open-url`(scope 限定 spellbook 三个平台域名)与
`opener:allow-reveal-item-in-dir`。**dialog 插件不引入**:文件选择/保存改为 Rust 侧
`rfd`(句流 `pick_folder` 同款),新增 `pet_pick_file` / `pet_save_path` 两个薄命令。
CSP 保持现状不动(§3.1 原则 5)。通知插件仅 Rust 侧调用,无需 capability。

### 3.7 后台执行体改造

| 原实现 | 改造 |
|---|---|
| 1s 心跳线程:`loop{tick;sleep}` 永不退出、启动即跑 | `PetState.heartbeat: Option<JoinHandle>` + `Arc<AtomicBool>` 停止旗标;**`pet.enabled` 开启时才 spawn**,关闭/退出时置旗标 join;`tick` 逻辑原样 |
| `rig_bake` 同步阻塞 IPC 线程(原实现缺陷,会卡住句流全部命令) | 改 `async` + `spawn_blocking`,完成后 emit `pet://changed` |
| watcher 文件稳定性短线程(stop 后仍可能 emit) | 原样迁移(8s 自愈,可接受),emit 前检查 watcher 仍活跃 |
| `spawn_blocking` ×3(视频抠像/导出) | 原样 |

---

## 4. 风险清单与逐条对策

勘察发现 27 项嵌入风险,按处置方式归并(R 编号供执行时逐项销账):

### 4.1 阻断级(架构已消化)

| 风险 | 对策(方案落点) |
|---|---|
| R1 `run()` 独占 Builder | 胶水层拆为 `pet::setup/commands/on_window_event`,由句流 lib.rs 装配(§3.2) |
| R2 `on_window_event` 覆盖式注册 | 句流现无此处理器;新增一个统一处理器按 label 分派,pet 分支只管 `"pet"` 窗(§3.3) |
| R3 `generate_handler!` 不可拼接 | 51 项直接追加进句流单列表(§6 总表) |
| R4 内联主题脚本 vs 严 CSP | 删除(主题改走句流令牌 + `pet://theme`),CSP 零放宽(§3.4) |
| R5 capability 白名单 | 新增 `pet.json`(§3.6) |
| R6 Cargo/conf 缺特性 | §3.5 触点表;`tray-icon` 特性**不加**(托盘不迁移) |
| R7 Vite 单入口 | 双入口(§3.2);dev 下 pet 窗 URL 为 `devUrl/pet.html`,打包走 frontendDist |

### 4.2 进程级假设(必须改写)

| 风险 | 对策 |
|---|---|
| R8 `app_quit`/托盘退出会杀句流整个进程 | 命令删除;右键菜单「退出宠物」→ `pet_visible_set(false)`;真正退出只走句流主窗 |
| R9 托盘 id/图标 expect panic | 托盘整体不迁移(§8) |
| R10 `webcache` 清缓存会连带清掉句流主窗 WebView 缓存 | **整块不迁移**;句流打包 exe 无 devUrl 缓存陈旧问题(该 hack 是 HatchDesk 特定环境产物) |
| R11 窗口 label 硬编码 `"pet"`/`"studio"` | `"pet"` 保留(常量化);`"studio"` 随窗取消而消失 |
| R12 `config.json` 落在共享数据根 | 设置并入句流 kv;文件数据收进 `pet/` 子目录(§3.4) |
| R13 心跳线程不可停 | §3.7 |
| R14 `~/HatchDesk/` 硬编码导出目录 | `reference_export` 改为 rfd 另存为对话框,由用户选择位置 |
| R15 进程级 static(`SEQ`、ONNX `OnceLock`) | 语义在单进程内正确,保留;`SEQ` 移入 `PetState` 顺手消除 |

### 4.3 命名与共享资源冲突(全表重命名解决)

| 风险 | 对策 |
|---|---|
| R16 事件名通用词(`settings-changed` 等 10 个) | 全部 `pet://*` 化(§6);`theme-changed` 机制整体废弃 |
| R17 localStorage 同源互踩(`hatchdesk-theme` 会覆写 `<html data-theme>`) | HatchDesk 主题系统删除;宠物窗 `data-theme` 由 `pet_bootstrap`/`pet://theme` 驱动,与句流同源同值 |
| R18 命令名通用词 | 51 个全部 `pet_*` 前缀(§6) |

### 4.4 健壮性(迁移时顺手修复/接受)

| 风险 | 处置 |
|---|---|
| R19 `panic=abort` profile | 不带入;句流 workspace profile 为准(panic 可 unwind,命令层报错而非崩溃) |
| R20 Mutex `.unwrap()` 无 poison 恢复 | 与句流现有惯例一致,保留;胶水层新代码遵守「短临界区、不跨 await」 |
| R21 3 处非测试 panic | tray 不迁移消 1;`run().expect` 随重构消 1;spellbook `data()` 重复解析加 `OnceLock` 缓存并消 expect |
| R22 `rig_bake` 阻塞 | §3.7 改 spawn_blocking(**必须修**,否则会卡句流所有 IPC) |
| R23 WizardState 内存驻留(可达数十 MB) | 保留原语义,追加:离开「AI 萌宠」页 5 分钟后自动 `pet_wizard_reset`(React 页卸载定时器) |
| R24 下载目录监听的隐私面 | 默认关(§3.4);「AI 萌宠」设置页明示文案 |
| R25 +4.4MB 模型与 tract 编译时间 | `sf-pet` 的 `matting` feature(默认开);CI 可用 `--no-default-features` 快检 |
| R26 前端模块级单例泄漏(dropzones/listener 反复注册) | React 重写天然解决:每个 Tab `useEffect` 返回清理函数;拖放区注册器移植时改为「注册即返还注销闭包」 |
| R27 `work_area_get` 窗口注入语义 | 改名 `pet_work_area_get` 并显式取 `"pet"` 窗,消除误用面 |

---

## 5. 分阶段执行计划

> 每个 Phase 一个(或一组)提交,**独立可编译、可验收、可回滚**;门禁不过不进下一阶段。
> 全程遵守:句流 209 个既有单测 + typecheck 始终全绿。

### Phase 0 · 基线固化(半小时)

1. 记录基线:`cargo test --workspace --all-features`(209)、`npm run typecheck`、
   `sf gold run`(7211/7211)结果存档。
2. `git tag pre-pet-integration`(本地标记,便于回滚对照)。

**门禁**:三项基线全绿。

### Phase 1 · `crates/sf-pet` 纯逻辑落库(1-2 天)

1. 新建 crate,按 §3.2 布局搬迁 `imaging/ rig/ validator/ exporter/ spellbook/ petkit/
   store/ archive/ wizard(纯部分)`;新写 `PetError`(无 Tauri 变体)。
2. `store.rs` 全部 `AppHandle` 参数改 `&Path`;`wizard` 的进度回调参数化
   (`FnMut(WizardProgress)`);`exporter::ffmpeg::locate(override: Option<&Path>)` 已参数化,原样。
3. spellbook `data()` 加 `OnceLock` 缓存(修 R21 性能项)。
4. 迁移全部 126 个单测(路径/夹具随迁),`u2netp.onnx` 与 `data.json` 移入 crate。
5. workspace `Cargo.toml` members 加入 `crates/sf-pet`;**此阶段桌面壳完全不引用它**。

**门禁**:`cargo test -p sf-pet` ≥126 全绿;`cargo test --workspace` 既有 209 不受影响;
`cargo clippy -p sf-pet -- -D warnings` 干净;`cargo check -p sf-pet --no-default-features`
(无 matting)通过。

### Phase 2 · 桌面壳胶水层(2-3 天)

1. `src-tauri/src/pet/` 五文件(§3.2):51 个命令重命名落地(§6),薄封装调 `sf-pet`;
   `PetState` 并入 `AppState`;`PetSettings` 并入 `Settings`(serde default)。
2. `lib.rs` 装配:插件 ×2、`pet::setup`(按 `pet.enabled` 决定建窗与心跳)、
   `on_window_event` 分派、handler 追加。
3. capabilities `pet.json` + default.json 增补;`tauri.conf.json` 加 macOSPrivateApi。
4. R8/R13/R14/R22/R27 五项改造在此阶段完成。
5. 此阶段前端尚无 pet.html —— pet 窗建窗代码就绪但 `enabled` 默认 false,主程序行为与
   基线完全一致。

**门禁**:全 workspace 编译 + 既有 209 测试全绿;新增胶水层单测(设置分节序列化兼容:
旧 settings JSON 反序列化后 `pet` 取默认值);打包 exe 启动冒烟(CDP:8 导航项、
bootstrap 正常 —— 证明"默认关闭时零影响")。

### Phase 3 · 宠物窗前端(2 天)

1. `pet.html`(无内联脚本,link 句流 tokens.css + pet.css)+ vite 双入口。
2. `src/pet-window/` 迁移 12 个 vanilla 模块 + `petIpc.ts`(pet_* 命令名 + pet:// 事件);
   `dropzones` 改为可注销注册;主题初始化改为 `pet_bootstrap` 返回值 + `pet://theme` 监听。
3. 「AI 萌宠」页先出**最小壳**:总开关(启用后建窗/关闭销毁)+ 宠物显隐 + 缩放/漫游/
   穿透三开关,验证 pet 窗全链路(孵化仪式暂用 .petkit 导入一只测试宠物驱动)。
4. 真机验收点:轮廓级穿透(宠物外点击落到底层应用)、拖拽落地物理、贴边漫游、
   右键菜单、省电模式帧率、四主题下气泡/菜单配色。

**门禁**:typecheck 全绿;CDP + 真实输入实测上述验收点;主窗全部既有页面点检无回归。

### Phase 4 · 「AI 萌宠」React 页(3-5 天,工作量主体)

按依赖序逐 Tab 重写(每 Tab 一次提交,UI 用句流设计系统组件,逻辑对照原 vanilla 实现):

1. **我的宠物**(← pets.ts 407 行):宠物卡片/设为当前/删除/导入导出(.petkit,rfd)/
   校验预检/骨骼烘焙/出生视频导出(名片与水印 canvas 生成逻辑原样移植为纯函数);
2. **认主校准** CalibrateModal(← calibrate.ts 161 行);
3. **孵化向导**(← wizard.ts 999 行,最大单体):首孵 L0 流程/进化(网格/视频)/专家模式
   strip 编辑;`pet://video-progress`、`pet://watcher-file` 接入;孵化剧场动画保留;
4. **咒语包**(← spellbook.ts 254 行):平台切换/复制/引导图另存(rfd)/自查表;
5. **提醒·番茄钟**(← reminders.ts 206 行):CRUD + 番茄钟盘面 + `pet://pomodoro-tick`;
6. **宠物设置**(← settings.ts 204 行):§3.4 字段全集 + ffmpeg 状态检测。

**门禁**:typecheck;逐 Tab 的 CDP 冒烟脚本(列表渲染/关键按钮/事件流);
R23 的页卸载重置定时器落地;全量回归清单(§7)首轮通过。

### Phase 5 · 打包与资源(半天)

1. `npx tauri build` 验证:pet.html 进 dist、双窗资源齐;exe 体积记录(预期 +≈5 MB)。
2. dist-portable 三件套照旧(pet 数据在用户目录,不随包)。
3. NSIS 安装-卸载-再安装冒烟(宠物数据留存于 app_data,符合句流备份哲学;
   注:pet 数据**不进** backup_export 的 zip —— 备份契约不变,记录为已知边界)。

**门禁**:打包产物在干净环境冒烟(启用宠物 → 孵化一只 → 重启进程宠物仍在)。

### Phase 6 · 全量回归 + 真机验收(1 天)

按 §7 清单双轨执行:句流既有 70 项功能速查表抽查(重点:设置页、主题切换、备份、
练习全流程)+ AI 萌宠 26 项新功能清单全过。CDP + 真实输入驱动打包 exe(沿用句流
既有验收方法论)。

### Phase 7 · 文档与收尾(半天)

1. 《句流-功能全景文档》新增「AI 萌宠」章 + 命令/事件表并入 §11 全表(65+51 命令、17+10 事件)。
2. 《开发状态》记录本次整合与全部有意偏差(§8)。
3. `README`/发布说明素材更新;版本规划:整合完成后随 **v0.5.0** 发布。

---

## 6. 命令与事件重命名总表

### 6.1 命令(51 个,`旧名 → pet_新名`;桶内按原注册序)

| 域 | 重命名 |
|---|---|
| 设置 | `settings_get → pet_settings_get`,`settings_set → pet_settings_set` |
| 宠物库 | `pets_list → pet_list`,`pet_thumb → pet_thumb`(保留),`pet_set_active`/`pet_set_anchors`/`pet_delete`(保留),`pet_active_assets`(保留),`rig_bake → pet_rig_bake` |
| 宠物包 | `petkit_export → pet_kit_export`,`petkit_import → pet_kit_import`,`validate_petkit_file → pet_kit_validate` |
| 向导 | `wizard_* → pet_wizard_*`(reset/session/add_strip/add_sheet/add_video/add_video_multi/swap_states/evolve/remove_strip/preview/hatch/reslice/replace_frame/export_frame 共 14 个),`l0_preview → pet_l0_preview`,`l0_hatch → pet_l0_hatch`,`reference_export → pet_reference_export` |
| 监听 | `watcher_start → pet_watcher_start`,`watcher_stop → pet_watcher_stop` |
| 咒语包 | `spellbook_get → pet_spellbook_get`,`guide_save → pet_guide_save`,`rescue_get → pet_rescue_get` |
| 提醒 | `reminders_list → pet_reminders_list`,`reminder_create/toggle/delete/snooze → pet_reminder_*`,`pomodoro_start/stop/status → pet_pomodoro_*` |
| 导出 | `exporter_status → pet_exporter_status`,`export_birth_video → pet_export_birth_video` |
| 运行时 | `work_area_get → pet_work_area_get`,`cursor_position_get → pet_cursor_position_get`,`pet_visible_set`(保留),`click_through_set → pet_click_through_set`,`studio_open → pet_open_studio` |
| 新增 | `pet_bootstrap`(宠物窗启动包:设置+主题+图集)、`pet_pick_file`、`pet_save_path`(rfd) |
| **删除** | `app_quit`(R8)、`track_event`(analytics 不迁移) |

净结果:51 − 2 删 + 3 新 = **52 个 `pet_*` 命令**,句流命令总量 65 → 117。

### 6.2 事件(全部 Rust→前端,除注明)

| 旧 | 新 | 备注 |
|---|---|---|
| `pet-changed` | `pet://changed` | 图集/激活宠物变更(孵化/进化/校准/删除) |
| `settings-changed` | `pet://settings` | 仅 pet 分节快照 |
| `reminder-fired` | `pet://reminder` | |
| `pomodoro-tick` | `pet://pomodoro` | 1Hz |
| `export-progress` | `pet://export-progress` | |
| `video-progress` | `pet://video-progress` | |
| `watcher-file` | `pet://watcher-file` | |
| `studio-nav` | `pet://nav` | 语义改为主窗页内切签 |
| `click-through-changed` | `pet://click-through` | 原前端零监听(死接口),迁移后宠物窗接上 |
| `theme-changed`(前端互发) | **废弃** | 改为 Rust 侧 `pet://theme`(appearance 变更时发) |

---

## 7. 测试与验收门禁

| 层 | 内容 | 通过标准 |
|---|---|---|
| 单测 | `sf-pet` 126 个(迁移)+ 胶水层新增(设置兼容/路径学/命令薄层)≈10 个 | `cargo test --workspace --all-features` ≥ 345 全绿 |
| Lint | clippy `-D warnings`(含 sf-pet)、`cargo fmt --check` | 干净 |
| 前端 | `npm run typecheck`(pet-window + AiPet 页纳入) | 通过 |
| 金标 | `sf gold run` | 7211/7211(不受影响,回归性质) |
| 零影响证明 | `pet.enabled=false`(默认)下,打包 exe 与基线行为逐项对照:启动耗时、8 页面冒烟、settings 往返、备份导出 | 无差异 |
| 宠物窗真机 | 穿透/拖拽/漫游/睡眠/眨眼/道具/气泡/右键菜单/四主题/省电模式/多显示器拖移 | 逐项人工+CDP |
| 孵化链路真机 | L0 单图孵化 → 咒语复制 → 网格图进化 → 视频抽帧 → 救帧/重切 → 校准 → 烘焙 → .petkit 导出/导入 → 出生视频(有/无 ffmpeg 两态) | 全通 |
| 提醒真机 | 日程/循环/番茄钟/贪睡,系统通知 + 宠物气泡双通道 | 全通 |
| 并发红线 | 造句工坊生成中执行 `pet_rig_bake` / 视频导入,句流 IPC 不卡顿(R22 验证) | 命令响应 < 200ms |

---

## 8. 有意不迁移项(决策与理由,写入《开发状态》)

| 项 | 理由 |
|---|---|
| 托盘(tray.rs) | 句流无托盘先例;主窗关闭=退出语义不变;显隐入口由页面与右键菜单覆盖。避免 R9 panic 面与常驻语义分裂 |
| `webcache.rs` | HatchDesk 特定环境 hack,且会清掉句流主窗缓存(R10) |
| `analytics.rs` + `track_event` + `events.jsonl` | 句流隐私哲学:仅匿名诊断包按需导出,不做常驻埋点 |
| `app_quit` | 进程级语义归句流主窗所有(R8) |
| studio 独立窗 | 并入主窗页面,减少一个窗口生命周期与一套跨窗同步 |
| `pro`/激活码占位 | 句流免费版,无付费判据 |
| 浏览器 mock 测试台(test/) | P2 再评估;当前验收走句流 CDP 真机方法论,单测已随 crate 全量迁移 |
| 下载目录监听默认开 | 改默认关(隐私保守化),功能保留可手动开启 |

以上均为**功能面之外**的工程设施;用户可见的产品功能(§2.1 七件套 + v1.1 全部改进)**无一删减**。

## 9. P1 扩展:与学习闭环的联动(本期不做)

整合完成后天然可做、且不破坏隔离原则的增量(经由事件总线,pet 侧单向订阅):

- 签名时刻答对 → `pet://celebrate`(宠物开心 + 饼干道具):句流 `submit_attempt`
  成功分支加一行 emit(pet.enabled 时);
- 连续打卡里程碑 → 宠物成长仪式;
- 番茄钟「专注结束」→ 直接跳转「今日练习」;
- 咒语包增加「AI 生成」按钮:经句流 sf-llm 四通道文生图(远期,依赖图像通道)。

## 10. 工作量与提交切分

| Phase | 预估 | 提交建议 |
|---|---|---|
| 0 基线 | 0.5h | (无提交) |
| 1 sf-pet crate | 1-2 天 | `feat(pet): sf-pet 纯逻辑 crate 落库(imaging/rig/validator/exporter/spellbook/petkit,126 测试随迁)` |
| 2 胶水层 | 2-3 天 | `feat(pet): 桌面壳接入 —— 52 个 pet_* 命令、PetState/PetSettings、pet 窗运行时(默认关闭)` |
| 3 宠物窗 | 2 天 | `feat(pet): 宠物窗前端(vanilla Canvas 栈)+ AI 萌宠页最小壳` |
| 4 React 页 | 3-5 天 | 逐 Tab 六个提交 `feat(pet): AI 萌宠 · <Tab 名>` |
| 5 打包 | 0.5 天 | `chore(pet): 打包与资源核验` |
| 6 回归 | 1 天 | `fix(pet): 回归修复 ×N` |
| 7 文档 | 0.5 天 | `docs(pet): 全景文档新增 AI 萌宠章;开发状态记录整合与偏差` |
| **合计** | **约 10-14 个工作日** | 全部落 main(沿句流惯例),每阶段可独立回滚 |

**回滚策略**:任一阶段失败,`git revert` 该阶段提交即可 —— 因为每阶段都保证
「默认关闭 + 触点全加法」,revert 不会牵连既有功能;Phase 2 之前甚至不触碰
`apps/desktop` 一行代码。

---

*执行本方案时,若发现与两侧代码实况不符之处,以代码为准,并回写本文档与《开发状态》。*
