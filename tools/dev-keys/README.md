# DEV signing keys — NOT production

`dev-signing-key.b64` 是**开发/测试专用**的 Ed25519 私钥,与
`apps/desktop/src-tauri/src/licensing.rs` 里内嵌的 DEV 公钥配对,用于本地
端到端验证激活流(签发 → 粘贴 → 验签 → 盖章动效)。

发布前(§7.6):
1. 在离线机器上运行 `sf-license keygen`;
2. 用新公钥替换 `LICENSE_PUBLIC_KEY_B64`;
3. 新私钥只存离线 U 盘 + 密码管理器,绝不入仓库;
4. 本目录的 dev 私钥保持原样(它只对 dev 公钥有效,对生产版本无用)。

签发测试许可证:
```
cargo run -p sf-license --features issuer -- issue \
  --email test@example.com --major-max 3 \
  --key-file tools/dev-keys/dev-signing-key.b64
```
