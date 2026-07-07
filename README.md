# MiBand TOTP AstroBox Plugin V2

AstroBox V2 插件，用于把 `otpauth://totp/...` URI 推送到小米手环上的 TOTP 快应用。

## 构建

```bash
rustup target add wasm32-wasip2
python3 scripts/build_dist.py --release --package
```

构建产物在 `dist/` 目录，`.abp` 文件可导入 AstroBox V2。

## 通信

插件会查找第一个已连接设备，确认设备上安装了 `com.xw.mibandtotp`，然后通过 AstroBox V2 `interconnect.send-qaic-message` 发送：

```json
{
  "list": [
    {
      "name": "Issuer",
      "usr": "account@example.com",
      "key": "SECRET",
      "algorithm": "SHA1",
      "digits": 6,
      "period": 30
    }
  ]
}
```
