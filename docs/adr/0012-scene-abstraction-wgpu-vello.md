---
status: accepted
---

# 渲染：自有 Scene 抽象，wgpu + Vello 为主后端，SDF 快路径，线性色彩空间

框架定义自己的 **Scene（Display List）**，作为节点树绘制与具体渲染器之间唯一的接口。主后端在 wgpu 上运行 Vello：任意矢量路径、渐变、裁剪和文本的矢量路径（ADR-0010）都由 Vello 负责。圆角矩形、边框、阴影这类高频 UI 图元走**SDF 快路径**，在着色器中解析计算。不支持计算着色器的环境（老旧 GPU、虚拟机、远程桌面）回退到 vello_cpu / tiny-skia。一套代码覆盖 Metal、D3D12、Vulkan。

色彩：内部在**线性空间**混合，支持 Display P3 广色域，接口预留 HDR（扩展动态范围）；渐变插值的色彩空间可以选择。

## Considered Options

- **每平台原生图形后端**（GPUI 式）：延迟和体积最优，但要维护三套着色器和资源管理。Scene 抽象保留了将来为特定平台换后端的可能。
- **Skia**：功能最全，但依赖 C++、体积大，违背纯 Rust 的倾向。

## Consequences

- 平台层（ADR-0002）是自研的，需要自己把原生 surface（CAMetalLayer、HWND/DXGI、wl_surface）交给 wgpu。
- 主题和样式里的颜色值要带色彩空间语义，默认按 sRGB 解释。
