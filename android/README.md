# Muxlane Android

远程查看器。协议与 VT 在 `:core`（纯 JVM，可单测）。`:app` 需要 Android SDK。

```bash
gradle :core:test
```

有 SDK 时在 `local.properties` 写 `sdk.dir=`，然后 `gradle :app:assembleDebug`。

正式 APK 使用环境变量注入签名，不把密钥放进仓库：

```bash
MUXLANE_KEYSTORE_PATH=/path/release.jks \
MUXLANE_KEYSTORE_PASSWORD=... \
MUXLANE_KEY_ALIAS=... \
MUXLANE_KEY_PASSWORD=... \
./gradlew :app:assembleRelease
```

GitHub Release 需要配置同名密码/别名 Secret，以及 Base64 编码的
`MUXLANE_ANDROID_KEYSTORE_BASE64`。
