# 内置模型

## u2netp.onnx

- 用途：L0 照片抠图（改进方案环节七）——复杂背景的本地判别式分割，`imaging/matting.rs` 经 `tract`（纯 Rust）推理。
- 来源：U²-Net 家族轻量变体，取自 rembg 模型分发（github.com/danielgatis/rembg releases）。
- 许可：**Apache-2.0**（可商用内置；对比：RMBG 系仅限非商用，禁止内置）。
- 体积：约 4.4 MB；输入 1×3×320×320，输出 d0 显著图。

### 本仓库对模型的修改（数值等价，tract 兼容化）

1. 所有 Resize 节点 `coordinate_transformation_mode`: `pytorch_half_pixel` → `half_pixel`
   （两者仅在输出维度=1 时有差异；本模型全部上采样输出 >1，逐元素等价）。
2. 所有 Resize 的动态 `sizes` 输入（Shape→Slice→Concat 子图）替换为**逐节点烘焙的常量 `scales`**
   （输入固定 320×320 后全部静态可求；侧输出为 2×/4×/8×/16×/32×）。
3. 验证：onnxruntime 对照原始模型，随机输入下最大绝对误差 6.6e-06（数值等价）。

修补脚本见开发记录；重新生成：下载原始 u2netp.onnx 后运行 patch 脚本即可。
运行时行为：模型加载/推理任一失败 → 自动回退泛洪抠图（质量不低于旧版）。
