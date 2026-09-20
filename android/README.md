# Muxlane Android

远程查看器。协议与 VT 在 `:core`（纯 JVM，可单测）。`:app` 需要 Android SDK。

```bash
gradle :core:test
```

有 SDK 时在 `local.properties` 写 `sdk.dir=`，然后 `gradle :app:assembleDebug`。
