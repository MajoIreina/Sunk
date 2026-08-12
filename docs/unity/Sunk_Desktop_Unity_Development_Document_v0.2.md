# Sunk
## Unity 6 + URP 开发需求与技术架构文档
**版本：V0.2**
**状态：开发基线 / Draft**

> 本文档是 `unity` 分支的开发基线；产品正式名称为 **Sunk**。

---

## 1. 项目概述

Sunk 是一款运行于 Windows 和 macOS 的桌面浮窗应用。

应用主体不是传统 UI，而是一个高度还原《Interstellar（星际穿越）》中 Gargantua（卡冈图雅）视觉特征的 3D 黑洞。

用户可以将文件、文件夹、快捷方式或应用程序拖入黑洞。黑洞通过引力捕获、轨道运动、加速、事件视界穿越等视觉过程吞噬目标，然后执行对应的文件操作，例如移入系统回收站/废纸篓、删除，或在支持且明确的情况下卸载应用。

核心设计理念：

> **让 Gargantua 本身成为用户界面。**

---

## 2. 产品目标

1. 高度还原 Gargantua 的视觉特征。
2. 支持 Windows 和 macOS。
3. 以桌面透明浮窗形式运行。
4. 支持从 Windows Explorer / macOS Finder 拖入文件。
5. 支持文件、文件夹、快捷方式等目标。
6. 使用黑洞引力动画完成文件吞噬反馈。
7. 根据 GPU/CPU 负载动态降低渲染质量。
8. 无交互时保持较低 CPU/GPU 占用。
9. 最终安装包目标控制在 **200 MB 以内**。
10. 保持较好的启动速度和桌面应用体验。

---

## 3. 非目标

当前版本不以以下内容为目标：

- 完整科学级 Kerr 黑洞数值模拟。
- 完整通用物理引擎模拟。
- 制作完整 3D 游戏世界。
- 大规模 3D 场景。
- HDRP 全功能渲染。
- 复杂角色、骨骼、动画系统。
- 大量传统纹理资产。

核心策略：

> 使用物理启发式 / 相对论近似模型实现视觉高度还原，而不是为了科学精确度把项目变成科研级黑洞模拟器。

---

# 4. 技术路线

| 模块 | 技术 |
|---|---|
| Engine | Unity 6 |
| Render Pipeline | Universal Render Pipeline (URP) |
| 主渲染 | Custom URP Renderer Feature / Render Pass |
| Gargantua | Custom HLSL GPU Shader |
| 黑洞模型 | 自定义数学模型 |
| 光线追踪 | GPU Ray Integration / Ray Marching |
| 吸积盘 | Procedural Generation |
| 引力透镜 | Relativistic Approximation |
| Doppler | Custom Shader |
| Redshift | Custom Shader |
| Bloom | URP / Custom Post Process |
| 业务逻辑 | C# |
| 文件系统 | C# + Native Platform Layer |
| Windows | Win32 Native Plugin |
| macOS | Cocoa / Objective-C Native Plugin |
| 构建 | Windows / macOS |
| 包体目标 | ≤200 MB |

---

# 5. 总体架构

```text
Sunk
│
├── Unity Runtime
│
├── Gargantua Renderer
│   ├── Ray Integrator
│   ├── Black Hole Model
│   ├── Accretion Disk
│   ├── Gravitational Lensing
│   ├── Doppler Beaming
│   ├── Gravitational Redshift
│   ├── Photon Ring
│   ├── Star Field
│   └── Post Processing
│
├── Interaction System
│   ├── Drag & Drop
│   ├── Target Detection
│   ├── Gravity Capture
│   ├── Orbit Simulation
│   ├── Event Horizon
│   └── Consumption Animation
│
├── File Operation System
│   ├── File Inspection
│   ├── Trash
│   ├── Delete
│   └── Uninstall
│
├── Performance System
│   ├── GPU Frame Time
│   ├── CPU Frame Time
│   ├── Dynamic Resolution
│   ├── Dynamic Ray Steps
│   ├── Effect Quality
│   └── Idle Mode
│
└── Platform Layer
    ├── Windows
    │   ├── Win32 Window
    │   ├── Explorer Drag & Drop
    │   ├── Recycle Bin
    │   └── Uninstall
    │
    └── macOS
        ├── Cocoa Window
        ├── Finder Drag & Drop
        ├── Trash
        └── Application handling
```

---

# 6. Unity 项目结构

```text
Sunk/
│
├── Assets/
│   ├── Sunk/                         # 产品自有资产边界
│   │   ├── Art/
│   │   ├── Audio/
│   │   ├── Materials/
│   │   ├── Shaders/
│   │   │   ├── Gargantua/
│   │   │   │   ├── Gargantua.hlsl
│   │   │   │   ├── RayIntegrator.hlsl
│   │   │   │   ├── BlackHoleModel.hlsl
│   │   │   │   ├── AccretionDisk.hlsl
│   │   │   │   ├── Relativity.hlsl
│   │   │   │   ├── Lensing.hlsl
│   │   │   │   ├── Radiation.hlsl
│   │   │   │   └── Common.hlsl
│   │   │   └── Post/
│   │   ├── Scripts/
│   │   │   ├── Core/
│   │   │   ├── Rendering/
│   │   │   ├── Interaction/
│   │   │   ├── FileSystem/
│   │   │   ├── Performance/
│   │   │   └── Platform/
│   │   ├── Plugins/
│   │   │   ├── Windows/
│   │   │   └── macOS/
│   │   ├── Prefabs/
│   │   ├── Scenes/
│   │   └── Settings/
│   ├── Scenes/                       # Unity URP 模板基线
│   └── Settings/                     # Unity URP 模板基线
├── Packages/
└── ProjectSettings/
```

---

# 7. Gargantua Renderer

## 7.1 核心原则

不要使用 `Sphere + Torus + Particle System + Orange Material` 作为 Gargantua 的主要视觉实现。

应该采用：

```text
Camera
   ↓
Full Screen Render Target
   ↓
Gargantua HLSL Shader
   ↓
Per-Pixel Ray Integration
   ↓
Black Hole / Disk / Lensing
   ↓
Post Processing
```

黑洞主要由数学模型和 GPU Shader 生成。

---

# 8. Gargantua 视觉组成

## 8.1 Event Horizon

表现为中心无法逃逸的黑暗区域。

要求：

- 无普通球体轮廓感。
- 边缘与引力透镜自然融合。
- 与 Photon Sphere / Shadow 一致。
- 不产生明显传统 3D 游戏模型感。

## 8.2 Black Hole Shadow

黑洞阴影不是简单黑色圆形贴图。

Shader 应通过光线积分判断：

```text
Ray
 ↓
进入捕获区域
 ↓
无法逃逸
 ↓
Black
```

## 8.3 Photon Ring

需要表现黑洞附近强引力区域中的高亮光环。

重点：

- 非简单纹理圆环。
- 与光线弯曲相关。
- 具有细、亮、锐的局部特征。
- 高质量模式下增强稳定性。

---

# 9. Accretion Disk

吸积盘是 Gargantua 视觉最重要的组成之一。

不能只使用一个固定橙色圆盘。

需要包含：

```text
Radius
+
Density
+
Temperature
+
Velocity
+
Turbulence
+
Emission
```

建议基础温度模型：

```text
T(r) ∝ r^(-3/4)
```

然后根据温度计算近似黑体辐射颜色。

视觉趋势：

```text
Inner Disk
    ↓
白 / 蓝白
    ↓
黄色
    ↓
橙色
    ↓
红色
    ↓
Outer Disk
```

具体参数以视觉调参为准。

---

# 10. Gravitational Lensing

这是 Gargantua Renderer 的核心。

普通对象：

```text
Camera
 ↓
Object
```

Gargantua：

```text
Camera
 ↓
Photon
 ↓
Gravity
 ↓
Ray Direction changes
 ↓
继续积分
 ↓
观察是否击中吸积盘 / 背景
```

目标是形成：

- 黑洞阴影
- 盘面上下包裹效果
- 背面吸积盘可见
- 强烈空间弯曲感

---

# 11. Ray Integration

初版采用 GPU 数值积分。

概念：

```text
for each pixel:
    ray = camera_ray()

    for i in 0..ray_steps:
        gravity = calculate_gravity(position)
        direction += gravity * step
        position += direction * step

        if inside_event_horizon:
            terminate

        if intersects_disk:
            accumulate_radiation
```

实际实现必须：

- 使用稳定积分方法。
- 防止 ray divergence。
- 避免过大的步长穿过吸积盘。
- 根据距离动态调整 step size。
- 高质量模式增加采样。
- 低质量模式降低步数。

---

# 12. Kerr / Spin 参数

Gargantua 不应被实现为完全静态 Schwarzschild 黑洞。

Renderer 应保留：

```text
BlackHoleParameters
├── Mass
├── Spin
├── HorizonRadius
├── DiskInnerRadius
├── DiskOuterRadius
├── DiskTemperature
└── LensingStrength
```

第一阶段可以采用近似模型，后续逐步加入 Spin、Frame Dragging 近似和 Kerr-like lensing。

---

# 13. Doppler Beaming

吸积盘高速旋转导致明显的相对论 Doppler 效应。

Shader 输入：

```text
velocity
viewDirection
```

目标视觉：

```text
Approaching side
    ↓
Brighter / whiter / bluer

Receding side
    ↓
Darker / redder
```

这是还原 Gargantua 的关键效果，优先级高。

---

# 14. Gravitational Redshift

靠近黑洞的光需要进行近似引力红移处理。

最终颜色应综合：

```text
Temperature
+
Doppler
+
Gravitational Redshift
+
Emission
```

而不是只由距离决定。

---

# 15. Procedural Disk Turbulence

吸积盘应避免明显重复纹理。

推荐：

```text
Procedural Noise
+
Radial Noise
+
Angular Noise
+
Time
```

产生：

- 局部亮斑
- 气体结构
- 不规则纹理
- 旋转运动

必须控制时间稳定性，避免低分辨率下严重闪烁。

---

# 16. Star Field

背景星场可以采用：

- Procedural stars
- 少量低分辨率纹理
- GPU generated star distribution

目标是增强电影感，但不能让星星抢过吸积盘。

---

# 17. Post Processing

建议：

```text
Gargantua
 ↓
Bloom
 ↓
Tone Mapping
 ↓
Color Grading
 ↓
Final Composite
```

Bloom 不能过度。视觉重点是强烈亮度对比，而不是整幅画面泛橙光。

---

# 18. Render Quality System

定义：

```text
RenderQuality
├── resolutionScale
├── raySteps
├── diskDetail
├── lensingQuality
├── bloomQuality
├── starDensity
├── turbulenceQuality
└── targetFPS
```

建议初始档位：

| 模式 | Resolution | Ray Steps | 目标 FPS |
|---|---:|---:|---:|
| Cinematic | 100% | 128–256 | 60 |
| High | 75–100% | 96–128 | 60 |
| Balanced | 60–75% | 64–96 | 45–60 |
| Performance | 40–60% | 32–64 | 30 |
| Background | 25–40% | 16–32 | 5–15 |

实际数值必须通过硬件测试调整。

---

# 19. 动态性能控制

Performance Manager 周期性读取：

```text
GPU Frame Time
CPU Frame Time
```

例如目标 60 FPS，预算约 16.6 ms。

如果持续超预算：

```text
Resolution ↓
Ray Steps ↓
Bloom ↓
Disk Detail ↓
Star Density ↓
```

如果有余量：

```text
Resolution ↑
Ray Steps ↑
Detail ↑
```

必须使用 hysteresis、平滑窗口和最小保持时间，避免质量频繁抖动。

---

# 20. Idle / Background Mode

桌面软件必须区别于游戏。

建议：

```text
Active Interaction
→ 60 FPS

Normal Desktop
→ 30–60 FPS

Inactive
→ 10–15 FPS

Long Idle
→ 5–10 FPS

Minimized
→ 停止渲染
```

如果窗口不可见，可以暂停绝大多数渲染更新。

---

# 21. 文件交互系统

文件对象进入黑洞后不直接瞬移。

状态机：

```text
Idle
 ↓
Detected
 ↓
Captured
 ↓
Orbiting
 ↓
Accelerating
 ↓
CrossingHorizon
 ↓
Consumed
 ↓
FileOperation
 ↓
Completed
```

---

# 22. 引力捕获动画

文件进入黑洞附近：

```text
Target
 ↓
Gravity Range
 ↓
Capture
```

动画参数：

```text
position
velocity
acceleration
angularVelocity
scale
rotation
stretch
progress
```

可以加入轨道运动、螺旋加速、视觉拉伸、自转、光晕和引力尾迹。

---

# 23. 文件对象视觉表示

- 文件：使用系统文件图标作为 Sprite / Quad / UI texture。
- 文件夹：使用系统 folder icon，可增加更强的粒子/拉伸效果。
- 快捷方式 / 应用：使用系统对应 icon，卸载流程单独判断。
- 接近事件视界时进行缩放、拉伸和旋转。

---

# 24. 文件操作安全原则

核心原则：

> **视觉吞噬 ≠ 立即执行不可逆操作。**

推荐：

```text
视觉进入 Event Horizon
 ↓
确认操作
 ↓
执行 File Operation
```

对于危险操作：

- 永久删除需要明确设置。
- 默认优先移动到系统 Trash / Recycle Bin。
- 权限不足时必须明确提示。
- 文件操作失败时不能表现为成功。
- 失败状态需要反馈。

---

# 25. FileSystem Abstraction

建议：

```csharp
public interface IFileSystemService
{
    FileInfoModel Inspect(string path);

    Task<OperationResult> MoveToTrash(string path);

    Task<OperationResult> Delete(string path);
}
```

Windows 和 macOS 实现分别封装，Core Interaction 不直接调用平台 API。

---

# 26. Uninstall 系统

卸载必须与普通删除区分：

```text
Delete
Trash
Uninstall
```

不是同一个操作。

Windows 可能存在 MSI、EXE uninstaller、Registry uninstall entry 或应用自带卸载器。

macOS 可能涉及 `.app`、Application Support、Preferences、Caches、Launch Agents。

因此使用：

```csharp
IUninstaller
```

并对每个平台单独实现。

---

# 27. Windows 架构

```text
Unity
 │
 ├── C# Core
 │
 └── Windows Native Plugin
       │
       └── Win32
```

Native Layer 主要负责：

- Transparent window
- Borderless window
- Always-on-top
- Window positioning
- Click-through
- Explorer Drag & Drop
- Recycle Bin integration
- Startup integration

---

# 28. macOS 架构

```text
Unity
 │
 ├── C# Core
 │
 └── macOS Native Plugin
       │
       └── Cocoa
```

Native Layer 主要负责：

- Transparent NSWindow
- Window level
- Finder Drag & Drop
- Trash integration
- Launch at Login
- macOS-specific application behavior

---

# 29. 跨平台原则

目标：

```text
Shared Code ≫ Platform Code
```

共享部分：

- Gargantua Renderer
- HLSL Shader
- Interaction
- Simulation
- Performance Manager
- File Model
- UI
- Settings

平台专属：

- Window internals
- Native Drag & Drop
- Trash / Recycle Bin
- Uninstall
- Startup
- Code signing / packaging

业务层不要直接调用 Windows/macOS API。

---

# 30. 透明桌面窗口

这是 Unity 路线的早期技术验证项目。

必须验证：

### Windows

- Transparent
- Borderless
- Always-on-top
- Explorer Drop
- Mouse input
- Click-through

### macOS

- Transparent
- Borderless
- Window level
- Finder Drop
- Mouse input
- App lifecycle

在没有完成这一验证之前，不进入完整产品开发。

---

# 31. UI 原则

尽量减少传统 UI。

默认状态：

```text
Desktop
     │
     ▼
   Black Hole
```

控制 UI 可以采用：

- 鼠标悬停
- Context Menu
- Settings Panel
- 极简状态提示

不建议在黑洞旁长期显示大量按钮。

---

# 32. 配置系统

建议保存：

```text
Settings
├── Position
├── Size
├── AlwaysOnTop
├── QualityMode
├── TargetFPS
├── IdleFPS
├── DeleteMode
├── AnimationDuration
├── SoundEnabled
└── LaunchAtStartup
```

默认设置应适合普通用户，不要求用户理解渲染参数。

---

# 33. 音效

可选：

- 轻微低频环境声
- 文件进入引力范围音效
- 加速音效
- Event Horizon 吞噬音效
- 操作完成音效

默认保持较低音量，不能破坏桌面工具的安静定位。

---

# 34. 性能预算

重点指标：

```text
GPU Frame Time
CPU Frame Time
RAM
GPU Utilization
GPU Power
Startup Time
Package Size
```

重点测试状态：

1. Idle
2. Normal
3. Hover
4. Dragging
5. Consuming
6. Multiple Targets
7. Window Obscured
8. Minimized

---

# 35. 多任务策略

多个文件同时拖入：

```text
Target A
Target B
Target C
Target D
```

必须受到 Performance Budget 控制。

示例策略：

```text
1 target
→ Cinematic

2–3 targets
→ High

4–8 targets
→ Balanced

>8 targets
→ Performance
```

具体阈值通过 profiling 决定。

---

# 36. 包体控制

目标：

> **≤200 MB**

原则：

1. 不使用不必要的 Unity Packages。
2. 删除未使用资源。
3. Addressables / AssetBundle 只有在有明确收益时使用。
4. 严格控制 Shader Variants。
5. 不保存重复纹理。
6. 音频采用合理压缩。
7. 不引入大型第三方框架。
8. 发布前检查 Build Report。

最终大小必须以实际 Release Build 测量。

---

# 37. Shader Variant 管理

避免：

```text
大量 shader keywords
+
大量 variants
```

应：

- 固定不需要的功能。
- 使用 Shader Stripping。
- 控制 Material Keywords。
- 检查 Build 中实际包含的 Shader。

---

# 38. 开发阶段

## Phase 0 — Feasibility

目标：

- Windows 透明 Unity 窗口
- macOS 透明 Unity 窗口
- Explorer/Finder Drag & Drop
- Always-on-top
- 基础 URP Render Pass

验收：

> 两个平台都能显示透明浮窗并接收文件拖入。

## Phase 1 — Black Hole Prototype

实现：

- Event Horizon
- Shadow
- Photon Sphere / Ring
- 基础吸积盘

验收：

> 静态镜头下已经具有明确 Gargantua 视觉特征。

## Phase 2 — Relativistic Rendering

实现：

- Ray integration
- Gravitational lensing
- Disk bending
- Doppler
- Redshift
- Spin

验收：

> 能明显表现电影版 Gargantua 的关键视觉结构。

## Phase 3 — Cinematic Quality

实现：

- Procedural turbulence
- Star field
- Bloom
- Tone mapping
- Temporal stability
- Anti-aliasing

验收：

> 近距离观察仍然具有高质量电影感。

## Phase 4 — Performance System

实现：

- Dynamic Resolution
- Dynamic Ray Steps
- Dynamic Effects
- Idle Mode
- Background Mode

验收：

> 高质量交互与低负载桌面常驻之间自动切换。

## Phase 5 — File Interaction

实现：

- File capture
- Gravity orbit
- Spiral acceleration
- Event Horizon crossing
- Visual consumption

验收：

> 文件进入黑洞全过程自然、稳定、有反馈。

## Phase 6 — File Operations

实现：

- Trash
- Delete
- Uninstall detection
- Operation result
- Error handling

验收：

> 文件操作真实可靠，失败不会显示为成功。

## Phase 7 — Platform Integration

Windows：

- Win32
- Explorer
- Recycle Bin
- Startup

macOS：

- Cocoa
- Finder
- Trash
- Launch at Login

## Phase 8 — Optimization

重点：

- GPU profiling
- CPU profiling
- Memory
- Startup
- Shader stripping
- Build size
- Idle power

## Phase 9 — Release

Windows：

- Release Build
- Installer
- Code Signing

macOS：

- `.app`
- DMG
- Code Signing
- Notarization

---

# 39. 测试矩阵

### Windows

- Intel integrated GPU
- AMD GPU
- NVIDIA GPU
- 多显示器
- 高 DPI
- Explorer Drag & Drop

### macOS

- Apple Silicon
- Intel Mac
- Retina display
- 多显示器
- Finder Drag & Drop
- macOS window management

---

# 40. Gargantua 视觉验收标准

### A. 黑洞

- 中心阴影自然。
- 没有普通 Sphere 感。

### B. 吸积盘

- 不像简单 Torus。
- 有温度变化。
- 有高速旋转感。
- 有动态湍流。

### C. 引力透镜

- 能看到明显弯曲。
- 背面吸积盘产生正确的包裹效果。

### D. Doppler

- 一侧明显更亮。
- 一侧偏红。
- 不产生不自然色块。

### E. Photon Ring

- 高质量模式下清晰。
- 不像简单 Bloom 圆环。

### F. 整体

> 用户无需解释，就能将其联想到《Interstellar》的 Gargantua。

---

# 41. 最重要的开发原则

## 原则 1：先做 Gargantua，再做产品

```text
Gargantua Renderer
        ↓
Performance
        ↓
Desktop Window
        ↓
File Interaction
        ↓
File Operation
```

## 原则 2：不要把 Gargantua 做成普通 3D 模型

核心必须是：

> GPU 数学模型 + Ray Integration + Procedural Rendering。

## 原则 3：视觉质量优先，但允许动态降级

最高质量必须足够惊艳，但桌面常驻不能一直满负载。

## 原则 4：删除与卸载必须安全

视觉可以激进，文件操作必须保守。

## 原则 5：平台代码必须隔离

业务层不要到处出现 Windows/macOS 条件编译。

---

# 42. MVP 定义

第一个可用版本至少包括：

```text
✓ Windows
✓ macOS
✓ Transparent Desktop Window
✓ Gargantua Renderer
✓ Accretion Disk
✓ Gravitational Lensing
✓ Doppler
✓ Basic Redshift
✓ Drag & Drop
✓ File Capture
✓ Gravity Animation
✓ Move to Trash
✓ Dynamic Quality
✓ Idle Mode
```

暂时可以不包括：

```text
✗ 高级卸载
✗ 复杂设置
✗ 多种主题
✗ 高级音效
✗ 云同步
✗ 多黑洞
```

---

# 43. V0.2 成功标准

1. Windows / macOS 均能正常启动。
2. 桌面透明浮窗稳定。
3. 文件能够从 Explorer/Finder 拖入。
4. Gargantua 视觉核心效果完整。
5. 黑洞吞噬动画自然。
6. 文件操作真实可靠。
7. Idle 状态资源占用明显降低。
8. 多目标拖入不会导致程序失控。
9. Release Build ≤200 MB。
10. Windows/macOS 均完成基础发布流程。

---

# 44. 后续技术文档

建议继续拆分：

1. `Gargantua_Renderer_Spec.md`
   - 黑洞数学模型
   - Kerr / Spin
   - Ray Integration
   - Lensing
   - Disk
   - Doppler
   - Redshift

2. `Unity_URP_Render_Architecture.md`
   - Render Feature
   - Render Pass
   - RTHandle
   - Shader
   - Temporal pipeline

3. `Desktop_Platform_Architecture.md`
   - Windows
   - macOS
   - Transparent Window
   - Drag & Drop
   - Native Plugin

4. `File_Operation_Spec.md`
   - Trash
   - Delete
   - Uninstall
   - Permissions
   - Error handling

5. `Performance_Spec.md`
   - GPU budget
   - Dynamic Resolution
   - Ray Step scaling
   - Idle mode
   - Multi-task scaling

6. `Release_Spec.md`
   - Windows packaging
   - macOS packaging
   - Signing
   - Notarization
   - ≤200 MB target

---

# 45. 最终技术决策

当前项目正式采用：

> **Unity 6 + URP + Custom HLSL Gargantua Renderer**

核心策略：

> **Unity 负责跨平台应用基础设施；自定义 GPU Renderer 负责 Gargantua；Native Plugin 负责 Windows/macOS 桌面系统特有能力。**

目标不是制作一个普通“黑洞特效软件”，而是：

> **制作一个以 Gargantua 为核心视觉和交互界面的跨平台桌面工具。**

最终产品的关键竞争力：

```text
                    Gargantua
                        │
          ┌─────────────┼─────────────┐
          │             │             │
       Visual        Interaction    Performance
          │             │             │
       Relativity     Drag/File      Adaptive
          │             │             │
          └─────────────┼─────────────┘
                        │
                 Desktop Utility
```

**视觉还原度是第一优先级；性能、桌面集成和包体控制围绕这一目标进行工程化优化。**
