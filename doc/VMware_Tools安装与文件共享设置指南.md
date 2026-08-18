# VMware Tools 安装与主机-虚拟机文件共享设置指南

> 适用环境：VMware Workstation + Windows 10 x64 虚拟机
> 最后更新：2026年8月

---

## 一、前提条件

- VMware Workstation Pro 已安装（当前免费，无需许可证）
- Windows 虚拟机已安装完成并能正常进入桌面
- 虚拟机处于**开机状态**

---

## 二、安装 VMware Tools

VMware Tools 是实现文件共享、拖放、复制粘贴等功能的基础，**必须先安装**。

### 方法 A：通过菜单自动安装（推荐）

1. 菜单栏点击 **VM → Install VMware Tools...**
2. 虚拟机内自动挂载 DVD 驱动器
3. 在**虚拟机内部**打开"此电脑" → 双击 DVD 驱动器 → 运行 `setup` 或 `setup64.exe`
4. 按向导完成安装 → **重启虚拟机**

### 常见问题及解决

#### 问题 1：Install VMware Tools 显示灰色，无法点击

**原因：** 虚拟机光驱正在挂载其他 ISO 文件（如 Windows 安装镜像）。

**解决步骤：**

1. 进入 **VM → Settings (Ctrl+D) → Hardware → CD/DVD (SATA)**
2. 取消勾选 **Connected**（如果可以操作的话）
3. 点 **OK** 保存
4. 回到 **VM** 菜单，Install VMware Tools 应该可以点击了

#### 问题 2：Connected 选项也是灰色，无法取消勾选

**原因：** 虚拟机运行状态下不允许热切换光驱连接。

**解决步骤（手动挂载 + 重启）：**

1. 进入 **VM → Settings (Ctrl+D) → Hardware → CD/DVD (SATA)**
2. 保持选择 **Use ISO image file**
3. 点击 **Browse...**，导航到以下路径：
   ```
   C:\Program Files (x86)\VMware\VMware Workstation\windows.iso
   ```
4. 选中 `windows.iso`，点"打开"
5. 确保 **Connect at power on** 已勾选 ✅
6. 点 **OK** 保存
7. 通过 **VM → Power → Restart Guest (Ctrl+R)** 重启虚拟机
8. 重启后进入虚拟机桌面，打开"此电脑"
9. 双击 **DVD 驱动器 (D:) VMware Tools**
10. 双击 `setup`（类型为"应用程序"，约 137 KB）
11. 安装向导中一路点 **Next**，安装类型选 **Complete**
12. 点 **Install** → 等待完成 → 点 **Finish**
13. **再次重启虚拟机**

#### 问题 3：弹出"The VMware Tools should only be installed inside a virtual machine"

**原因：** 你在主机（物理电脑）上运行了安装程序，而不是在虚拟机内部。

**解决方法：**

1. 关闭主机上的弹窗和安装向导
2. 将 `windows.iso` 挂载到虚拟机的 CD/DVD 设备（按上方步骤操作）
3. **在虚拟机窗口内部**操作：打开"此电脑" → DVD 驱动器 → 运行 setup

> ⚠️ 核心要点：必须双击进入 VMware 的虚拟机画面，在虚拟机的桌面环境里执行安装，而不是在主机的文件管理器中操作。

---

## 三、设置文件共享

VMware Tools 安装完成并重启后，有以下几种方式实现文件传输。

### 方法 1：共享文件夹（最推荐，适合大量文件）

1. 菜单栏 **VM → Settings (Ctrl+D)**
2. 切换到 **Options** 选项卡
3. 左侧选择 **Shared Folders**
4. 右侧选择 **Always enabled**（始终启用）
5. 勾选 **Map as a network drive in Windows guests**
6. 点击 **Add** → 选择主机上要共享的文件夹路径 → 完成
7. 点 **OK** 保存

**访问方式（在虚拟机内）：**

- "此电脑"中会出现一个网络驱动器（通常为 Z: 盘）
- 或通过路径访问：`\\vmware-host\Shared Folders`

### 方法 2：拖放 + 复制粘贴（适合临时传少量文件）

1. 菜单栏 **VM → Settings (Ctrl+D)**
2. 切换到 **Options** 选项卡
3. 左侧选择 **Guest Isolation**
4. 勾选 **Enable drag and drop**（启用拖放）
5. 勾选 **Enable copy and paste**（启用复制粘贴）
6. 点 **OK** 保存

**使用方式：**

- 直接从主机桌面拖动文件到虚拟机窗口
- 在主机上 Ctrl+C 复制文件，在虚拟机中 Ctrl+V 粘贴
- 反向操作同样支持

---

## 四、故障排查清单

| 症状 | 检查项 |
|------|--------|
| Install VMware Tools 灰色 | CD/DVD 是否挂载了其他 ISO → 取消或替换为 windows.iso |
| Connected 灰色无法勾选 | 改用 Connect at power on + 重启虚拟机 |
| 安装提示只能在虚拟机内安装 | 确认是在虚拟机画面内操作，不是在主机上 |
| 共享文件夹不显示 | 确认 VMware Tools 已安装且服务正在运行 |
| 拖放/粘贴不工作 | 检查 Guest Isolation 设置 + 确认 VMware Tools 服务运行中 |
| DVD 驱动器为空 | 检查 CD/DVD 设置中 ISO 路径是否指向 windows.iso |

### 检查 VMware Tools 服务状态

在虚拟机内：

1. 按 `Win + R`，输入 `services.msc`，回车
2. 找到 **VMware Tools Service**
3. 确认状态为"正在运行"，启动类型为"自动"
4. 如果未运行，右键点击 → 启动

### VMware Tools ISO 默认路径

```
Windows 主机：C:\Program Files (x86)\VMware\VMware Workstation\windows.iso
Linux 主机：  /usr/lib/vmware/isoimages/windows.iso
```

---

## 五、操作流程图（快速参考）

```
开始
 │
 ├─ VM → Install VMware Tools 可点击？
 │   ├─ 是 → 点击 → 虚拟机内运行 setup → 重启 → 设置共享文件夹 ✅
 │   └─ 否（灰色）
 │       │
 │       ├─ VM → Settings → CD/DVD → 取消 Connected
 │       │   ├─ 可以取消 → OK → 再试 Install VMware Tools
 │       │   └─ Connected 也是灰色
 │       │       │
 │       │       ├─ Browse 选择 windows.iso
 │       │       ├─ 确保 Connect at power on ✅
 │       │       ├─ OK → Restart Guest
 │       │       └─ 虚拟机内打开 DVD → 运行 setup → 重启 → 设置共享 ✅
 │       │
 │       └─ 如果以上都不行 → 关机虚拟机 → 再改 CD/DVD 设置 → 开机
 │
 └─ 完成
```
