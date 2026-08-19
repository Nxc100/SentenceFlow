# AI 聊天模块 — 实现方案

> 版本:v1(2026-08-19)· 性质:**v5 合订版之外的新增模块**(合订版的 AI
> 面只有答疑/周点评/生成,本方案自带设计依据,落地后记入 doc/开发状态.md)。
> 目标:①「聊天模式」用已接入的 AI 通道进行英文对话练习;②「角色扮演」
> 等趣味模式;③AI 在聊天中纠正/引导用户的英文;④「智能体模式」给本地
> opencode CLI 套一个美观外壳,友好使用其全部能力。

---

## 1. 调研结论(方法论依据)

1. **AI 会话伙伴的效果有实证**:多项研究与元分析显示,LLM 会话机器人
   显著提升二语学习者的**交流意愿(WTC)**、降低口语焦虑、增强信心与
   "语言冒险"意愿;对没有母语者伙伴的学习者,随时可用的 AI 对话是
   最有价值的口语替代练习。
2. **纠错方式有讲究**:元分析共识——**显式、有针对性**的纠错
   (指出错误 + 给出更好说法 + 一句为什么)显著优于纯隐式重述
   (recast,学习者往往注意不到);但纠错不能打断交流流。
   → 设计:AI 正常回复对话,纠错以**可折叠的小卡**附在回复下方,
   默认展开、可全局关闭。
3. **回复难度要受控**:面向学习者的对话机器人研究强调控制回复的
   语法/词汇复杂度 → 本项目现成优势:用户已有等级(levels.ts),
   系统 prompt 按等级约束 AI 回复的长度与用词。

> 来源:[AI 会话机器人提升 L2 口语与降低焦虑(Nature HSSC 2025)](https://www.nature.com/articles/s41599-025-05550-z) ·
> [LLM 聊天机器人语言练习用户研究(NLP4CALL 2024)](https://aclanthology.org/2024.nlp4call-1.18.pdf) ·
> [即时 vs 延迟纠错(Frontiers in Education 2026)](https://www.frontiersin.org/journals/education/articles/10.3389/feduc.2026.1703664/full) ·
> [纠错反馈综述(ERIC EJ1086236)](https://files.eric.ed.gov/fulltext/EJ1086236.pdf) ·
> [学习者对话机器人的语法控制(arXiv 2502.07544)](https://arxiv.org/pdf/2502.07544)

## 2. 实机 Spike 结论(2026-08-19,opencode 1.18.18,已验证)

智能体模式的技术路线**在本机实测成立**,绕开了 W2 发现的 `--attach` 缺陷:

| 能力 | 实测结果 |
|---|---|
| `opencode run -s <sessionID>` 续聊 | ✅ 服务端跨请求记忆真实可用(第一轮报名字,第二轮正确记起「Zhang Wei」) |
| 会话 ID 获取 | ✅ `--format json` 的**每条事件都带顶层 `sessionID`**,首条消息即可捕获 |
| 工作目录 | ✅ `--dir <path>` 指定智能体的操作目录 |
| 权限 | 二次 spike(M5 前置)已验证:**非交互 run 下默认智能体的 bash 工具不经确认直接执行**——`--auto` 对 run 模式意义不大,方案改为无条件安全警告(见 §3.5 实施修订) |
| 其他 | `--agent` 选智能体、`--fork` 分叉会话、`--title` 命名、`--variant` 推理力度 |

---

## 3. 产品设计

### 3.1 信息架构:一个导航项,三种模式

左侧栏新增「**AI 聊天**」(🤖,置于「AI 造句」之后)。页内三个模式签:

```
AI 聊天
├─ 自由聊天   随便聊,AI 是耐心的英语陪聊 + 温和纠错
├─ 角色扮演   AI 扮演面试官/店员/房东…,情境化实战
└─ 智能体     opencode 全能力的美观外壳(通用助手,可操作文件)
```

> 导航已 8 项,接近上限。备选方案(记录):并入「AI 造句」页作模式签。
> 默认独立入口——聊天与造句是完全不同的使用心智。

无可用通道时整页显示引导卡(复用工坊 GuideCard 模式),**绝不阻塞学习
功能**(§1.4 红线)。

### 3.2 自由聊天(含纠错引导)

- 经典聊天界面:气泡流 + 底部输入框(Enter 发送 / Shift+Enter 换行),
  AI 回复打字机流式(复用答疑抽屉的 60 字/秒缓冲模式);
- **系统 prompt 三要素**:
  1. 角色:友好耐心的英语陪聊,回复全英文、2–4 句、以追问结尾保持对话;
  2. **难度自适应**:按用户当前等级(阶段名 + can-do 注入 prompt)约束
     用词与句长——入门用户收到的回复自己读得懂;
  3. 纠错协议(见 3.4);
- 空态给 3 个开场话题签(「聊聊你的周末」「介绍你的工作」「你最喜欢的食物」),
  点一下即代发,解决"不知道说什么"的冷启动;
- 会话本地持久化,可开新话题、可删除;每轮记录含纠错卡。

### 3.3 角色扮演

- **角色卡片墙**(数据驱动,前端常量起步):面试官 💼、咖啡店店员 ☕、
  酒店前台 🏨、海关官员 🛂、房东 🏠、外国朋友 🎉、点餐服务员 🍽、
  医生 🩺 + 「自定义角色」(用户一句话描述);
- 每张角色卡:角色 system prompt(保持人设、场景目标、引导对话推进)+
  **AI 先开场**(如面试官:"Please have a seat. Tell me about yourself.")
  ——解决用户不知道怎么开始;
- 纠错协议同 3.4(默认开,可关——沉浸派用户不想被打断);
- **与情景对话联动**(高价值集成):情景对话包详情页加
  「和 AI 实战演练 →」——照剧本练完后,进入角色扮演,AI 扮演 A 方
  (店员/前台),用户即兴扮演 B 方,把"背下来的对话"变成"用出来的对话"。

### 3.4 纠错协议(显式而不打断)

系统 prompt 要求 AI 回复分两段:正文(纯对话)+ 末尾一行结构化标记:

```
⟦fix⟧{"ok":false,"better":"I have been living here for two years.","why":"live 表持续状态用现在完成进行时"}
```

- 前端解析 `⟦fix⟧` 后的 JSON:`ok:true` 不显示任何东西;有修改时在
  AI 气泡下方渲染**纠错小卡**:原句(灰)→ 更好的说法(绿)+ 一句原因;
- 解析失败 → 整段按纯文本显示(容错降级,绝不丢内容);
- 全局开关「聊天中帮我纠错」(默认开),关闭时 prompt 移除该协议;
- 纠错卡带「🔊 朗读」。(实施修订:原设计的「⭐ 收藏该句」取消——
  favorites 指向句库 id,聊天纠错句没有全套标注,入库必须过生成校验
  管线,v1 不做半吊子入口;后续可加「送去 AI 造句」深加工。)

### 3.4b 每个对话单独切模型(v1.1 追加,opencode `/model` 的可视化版)

opencode TUI 里换模型靠敲 `/model`;这里把它做成聊天窗顶部的模型芯片:

- 芯片显示当前对话实际在用的模型(`⚡ hy3-free ▾`);为本对话固定过就
  多一枚「本对话」小标;
- 点开是模型面板:通道页签(智能体模式只有 opencode)+ 模型清单
  (带「直连可用 / 🔒 需代理」标注,取自 channels.json 策略)+
  「跟随设置(全局模型)」回退项;通道按需探测,结果页面级缓存 → 秒开;
- 选择存进 `chat_thread.channel/model/model_label`,**下一条消息起生效**;
  会话还没建(自由聊天首条消息前)则先记住,建会话后立即补上;
- **实机验证**:opencode 会话中途换模型仍沿用同一 `-s` 会话,记忆不丢
  (hy3-free 建会话 → mimo-v2.5-free 续聊,正确记起先前给的暗号)。

### 3.5 智能体模式(opencode 的美观外壳)

定位:**通用 AI 助手**,不限英语学习——读写文件、跑命令、查资料,
把 opencode CLI 的全部能力用友好的 GUI 呈现。

- **会话列表**(左侧窄栏):历史会话(标题 + 时间 + 工作目录),新建/继续/删除;
- **工作目录**:新建会话时必选(系统文件夹选择器),顶部常显
  `📁 D:\my-project`——智能体只在这个目录里工作,用户清楚边界;
- **消息流**:正文打字机流式;**工具活动可视化**——JSON 事件流里的
  非 text 事件渲染为状态行(「⚙ 正在执行命令…」「📝 正在编辑 xxx」),
  完成后折叠为可展开的活动摘要,这是比黑终端友好的核心;
- **权限**(实施修订,2026-08-19 二次 spike):非交互 `run` 模式下
  opencode 默认智能体的工具(bash 等)**不经确认直接执行**——
  「默认询问 + --auto 开关」的原设计在 run 模式下没有意义。改为:
  **不提供 --auto**,在新建会话(选目录)时**无条件**展示红字警告
  「AI 将可以在该文件夹内读写文件、执行命令」,并建议单独准备文件夹;
  会话中顶部常显目录 + 一句提醒;空转 180s 超时兜底(万一某权限真挂起);
- 模型:本会话固定的优先(见 §3.4b),否则用设置里的(仅 `opencode/`
  目录下的名字透传,都不是就交给 CLI 默认);停止按钮 = 杀子进程
  (保留已收内容);
- **删除会话可选连带清理工作文件夹**(v1.1 追加):确认弹窗里勾选
  「同时清理工作文件夹」,文件夹**移入系统回收站**(可找回)而非直接抹掉,
  并显式提示先确认没有还需要的资料;层层护栏:磁盘根目录 / 用户主目录 /
  桌面·文档·下载 / Windows·Program Files·ProgramData / 应用自身数据目录
  及其祖先一律拒绝,拒绝时会话照删、文件夹保留并说明原因;
- 会话记忆在 opencode 服务端(`-s`),我们只存索引(id/标题/目录/时间)。

---

## 4. 技术方案

### 4.1 架构总览

```
sf-llm(通道层,新增多轮抽象)
  ChatTurn { role: user|assistant, text }
  ChatRequest { model, system, turns, session: Option<String>, max_tokens, temperature }
  trait ChannelAdapter {
      // 新增,带默认实现:turns 拼成对话转录塞进 complete_stream(全通道立即可用)
      fn chat_stream(&self, req: ChatRequest) -> BoxStream<GenChunk>;
  }
  - OpenAI 兼容通道(DeepSeek/Zen/Ollama)覆写:原生 messages 数组多轮
  - opencode 通道覆写:`run -s <session>` 服务端记忆;新会话从首事件捕获
    sessionID,经新增 GenChunk::SessionRef { id } 变体回传
    (additive:现有消费方都有 Ok(_) => {} 兜底臂,serde 兼容)

apps/desktop/src-tauri/chat.rs(新模块)
  chat_send(thread_id?, mode, text, …) —— 流式,事件 chat://chunk|fix|session|done|error
  chat_stop / chat_threads / chat_history / chat_delete_thread
  agent_send(thread_id?, workdir, text, auto_approve) —— 事件同上 + chat://tool
  持久化:progress.db 新表(见 4.2)

apps/desktop/src/pages/AiChat.tsx(新页面)
  三模式签 + 气泡流(复用答疑抽屉的打字机/Markdown 模式)+ 纠错卡 +
  角色卡片墙 + 智能体会话列表与工具活动条
```

### 4.2 持久化(progress.db 两张新表)

```sql
CREATE TABLE chat_thread (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  mode TEXT NOT NULL,            -- free | roleplay | agent
  title TEXT NOT NULL DEFAULT '',
  role_id TEXT NOT NULL DEFAULT '',   -- 角色扮演的角色卡 id
  oc_session TEXT NOT NULL DEFAULT '',-- opencode 服务端会话 id
  workdir TEXT NOT NULL DEFAULT '',   -- 智能体工作目录
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE chat_message (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  thread_id INTEGER NOT NULL,
  role TEXT NOT NULL,            -- user | assistant
  text TEXT NOT NULL,
  fix_json TEXT NOT NULL DEFAULT '',  -- 纠错卡结构(原样存)
  ts INTEGER NOT NULL
);
CREATE INDEX idx_chat_message_thread ON chat_message(thread_id);
```

- API 通道的多轮记忆 = 本地历史回放(每次带最近 **12 轮**,防 token 膨胀);
  opencode 通道 = 服务端记忆,历史仅作展示;
- 单线程消息上限 500 条,超出提示开新话题;删除线程级联删消息。

### 4.3 Prompt 设计(prompt.rs 新增,前缀字节稳定)

- `build_chat_prompt(level_name, can_do, fix_enabled)` — 自由聊天系统段;
- `build_roleplay_prompt(role_system, level_name, fix_enabled)` — 角色段 +
  难度约束 + 纠错协议;
- 纠错协议段(两者共用):输出契约 + `⟦fix⟧` 标记格式 + 三条 few-shot
  (有错/无错/严重错各一);
- 智能体模式**不注入任何 prompt**——直接透传用户消息,保持 opencode
  原生能力(它有自己的 agent 系统)。

### 4.3b 并发与"绝不留白"(v1.1 追加,均为真机踩坑后加固)

- **流式状态按会话分桶**:前端 `streamsRef: Map<threadId, ThreadStream>`,
  一个定时器喂所有会话;切到别的会话不打断、切回来照样看到「正在思考…」
  与已吐出的字;侧栏对生成中的会话显示「● 生成中」;
  离开整页再回来经 `chat_active_threads` 恢复指示(先订阅再查询,无缝);
- **停止按会话**:后端 `chat_cancels: Mutex<HashMap<i64, Arc<Notify>>>`,
  `chat_stop(thread_id)` 只掐这一个,其余会话继续跑;
- **空回复自愈**:真机遇到 opencode 某个会话续聊后只回 `step_finish`、
  不吐任何 text(tokens 照扣、`reason:"stop"`,同模型开新会话正常)。
  对策:续聊拿到空正文时**丢掉服务端会话、用本地历史重开一轮**
  (`chat_message` 才是事实源,`-s` 只是省 token 的优化),用户无感;
  重开仍为空则明确报「这次没有收到回复……可以再发一次或换个模型」;
- **空转超时**:聊天 120s / 智能体 180s 无任何事件即收流报错 —— CLI 真机
  出现过无限挂起,界面绝不能一直转圈。

### 4.4 关键流程:一次聊天发送

```
前端 chat_send(text)
 → 后端:存 user 消息 → 组 ChatRequest(带最近 12 轮/或 oc_session)
 → adapter.chat_stream 流式:
     GenChunk::Text       → 累积 + 节流 emit chat://chunk(打字机数据源)
     GenChunk::SessionRef → 首次记入 thread.oc_session
     GenChunk::Usage      → spend_add(计费记录,DeepSeek 显示费用)
 → 流结束:剥离 ⟦fix⟧ 段 → 正文与 fix_json 分别入库
 → emit chat://done { fix_json }
限速(RateLimited)→ emit chat://error { retry_after },界面倒计时后可重发
```

### 4.5 智能体模式技术要点

- 每条消息一次 `opencode run --format json --dir <workdir> [-s <id>] [--auto]`,
  CREATE_NO_WINDOW + 代理注入沿用现有 `hidden_command`/`apply_proxy`;
- 事件解析复用 `run_line_to_chunk` 思路,但**保留非 text 事件**转成
  `chat://tool { kind, detail }`(step_start/step_finish/tool 事件);
- 停止 = kill 子进程(会话在服务端完好,可继续);
- **M5 前置 spike**(排期内):非 `--auto` 时触发需批准工具的实际行为
  (挂起?拒绝?),据此决定默认策略与超时保护(120s 无事件 → 提示)。

### 4.6 与现有系统的边界

- **不写 SRS、不写练习日志、不计试用句数**(聊天不是句子练习);
  收藏纠错句走既有 favorites(那是用户显式动作);
- 计费与限速走既有 spend/backoff 基础设施;
- 答疑抽屉(ask_ai)保持独立不动——它是练习中的即时上下文答疑,
  与聊天模块是不同场景;实现中可把打字机/Markdown 抽成共享组件供两者用。

---

## 5. 里程碑

| # | 内容 | 预估 |
|---|---|---|
| M1 | sf-llm 多轮抽象(ChatTurn/ChatRequest/chat_stream 默认实现 + OpenAI 覆写 + opencode `-s` 覆写 + SessionRef)+ 单测 | 1 天 |
| M2 | 后端 chat.rs(两张表 + chat_send 流式 + stop/threads/history/delete)+ ipc 镜像 | 1 天 |
| M3 | 自由聊天 UI(气泡/打字机/纠错卡/开场话题/开关)| 1.5 天 |
| M4 | 角色扮演(角色卡墙 + AI 开场 + 自定义角色 + 情景对话「实战演练」联动)| 1 天 |
| M5 | 智能体模式(权限 spike → 会话列表/工作目录/工具活动条/--auto 开关)| 1.5 天 |
| M6 | 打包实测(CDP 三模式全流程 + 断网/限速/解析失败降级)+ 文档 + 发版 | 0.5 天 |

> 依赖:M1→M2→M3;M4 依赖 M3;M5 仅依赖 M2,可与 M3/M4 并行。

## 6. 风险与对策

| 风险 | 对策 |
|---|---|
| 免费模型角色保持力/英语质量参差 | 角色 prompt 强约束 + few-shot;界面提示可用「帮我选模型」换更强模型 |
| `⟦fix⟧` 标记解析失败 | 容错降级为纯文本显示,绝不丢内容;标记选用生僻括号避免撞正文 |
| 非交互下工具权限挂起(智能体) | M5 前置 spike 实测;120s 无事件超时提示;停止按钮兜底 |
| 聊天历史膨胀 | 回放窗口 12 轮;单线程 500 条上限;线程可删 |
| 导航 8 项拥挤 | 四字标签已验证可容纳;备选(并入 AI 造句页)已记录 |
| 智能体误操作文件 | 工作目录显式选择 + 常显;--auto 默认关 + 红字警告;不提供系统盘根目录快捷入口 |
| 用户把智能体当聊天用(或反之) | 模式签副标题写清区别;智能体新建会话强制选目录形成心智区隔 |

## 7. 规范符合性

- §1.4 AI 永远不是门槛:整页仅在有通道时可用,无通道显示引导卡,
  学习功能零依赖 ✓
- 术语规范:全程无 L/CEFR 字样,难度注入用阶段名 ✓
- 隐私:聊天记录仅存本机 progress.db;不读 opencode auth ✓
- 计费透明:DeepSeek 聊天同样计入 spend 与 CostBar 口径 ✓
- serde 三处同步铁律:ChatTurn/事件负载在 ipc.ts 镜像 ✓(实现时执行)

---

*落地后在 doc/开发状态.md 记录里程碑与实测结论;M5 的权限 spike 结论
无论成败都要记入(它决定智能体模式的默认策略)。*
