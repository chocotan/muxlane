pluginManagement {
    repositories {
        mavenCentral()
        gradlePluginPortal()
        google()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        mavenCentral()
        google()
    }
}

rootProject.name = "muxlane-android"
include(":core")

val sdkDir = System.getenv("ANDROID_HOME")
    ?: System.getenv("ANDROID_SDK_ROOT")
    ?: file("local.properties").takeIf { it.exists() }?.readLines()
        ?.firstOrNull { it.startsWith("sdk.dir=") }
        ?.substringAfter("=")
        ?.replace("\\\\", "\\")
if (!sdkDir.isNullOrBlank() && file(sdkDir).resolve("platforms").exists()) {
    include(":app")
}
